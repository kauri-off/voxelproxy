use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use etherparse::SlicedPacket;
use windivert::{
    WinDivert,
    layer::{ForwardLayer, NetworkLayer, WinDivertLayerTrait},
    packet::WinDivertPacket,
    prelude::WinDivertFlags,
};
use windows::Win32::Foundation::HANDLE;

use crate::logger::Logger;

// Re-declare WinDivert FFI functions so we can call them from a different thread
// than the one doing recv() — the Rust wrapper requires &mut self which prevents
// sharing, but the underlying Windows API is fully thread-safe for these operations.
unsafe extern "C" {
    fn WinDivertShutdown(handle: HANDLE, how: u32) -> u32;
    fn WinDivertClose(handle: HANDLE) -> u32;
}

/// Reads the raw Windows HANDLE out of a `WinDivert<L>`.
///
/// # Safety
/// `WinDivert<L>` has `handle: HANDLE` as its first declared field.
/// With Rust's default repr, HANDLE (a pointer, align 8 on 64-bit) sorts first
/// — confirmed by reading windivert-0.6.0 source code. The struct is not
/// `repr(C)` but this layout is stable for this field combination.
unsafe fn wd_raw_handle<L: WinDivertLayerTrait>(wd: &WinDivert<L>) -> HANDLE {
    unsafe { (wd as *const WinDivert<L> as *const HANDLE).read() }
}

/// Drops the WinDivert intercept handles, unblocking any threads blocked in `recv()`.
///
/// When dropped, calls `WinDivertShutdown` + `WinDivertClose` on each stored handle.
/// This causes any pending `WinDivertRecv` call to return with an error, which makes
/// the worker thread loops break and exit cleanly.
pub(crate) struct RedirectHandle {
    // Store handle as usize (the raw pointer value) to avoid !Send from *mut c_void.
    handles: Vec<usize>,
}

// SAFETY: HANDLE is an opaque pointer used only as a kernel object reference;
// WinDivertShutdown/Close are thread-safe per WinDivert documentation.
unsafe impl Send for RedirectHandle {}

impl Drop for RedirectHandle {
    fn drop(&mut self) {
        for &h in &self.handles {
            let handle = HANDLE(h as *mut _);
            unsafe {
                WinDivertShutdown(handle, 3); // 3 = WinDivertShutdownMode::Both
                WinDivertClose(handle);
            }
        }
    }
}

pub(crate) struct NatEntry {
    client_addr: Ipv4Addr,
    client_port: u16,
    original_dst_ip: Ipv4Addr,
    original_dst_port: u16,
    interface_id: u32,
    subinterface_id: u32,
    timestamp: Instant,
}

pub(crate) type NatTable = Arc<Mutex<HashMap<(Ipv4Addr, u16), NatEntry>>>;

// ── Admin check ───────────────────────────────────────────────────────────────

pub fn is_admin() -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut return_length = 0u32;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_length,
        );

        let _ = CloseHandle(token);
        result.is_ok() && elevation.TokenIsElevated != 0
    }
}

// ── Packet rewriting helpers ──────────────────────────────────────────────────

fn rewrite_dst(data: &mut [u8], ip_header_len: usize, new_addr: Ipv4Addr, new_port: u16) {
    data[16..20].copy_from_slice(&new_addr.octets());
    data[ip_header_len + 2..ip_header_len + 4].copy_from_slice(&new_port.to_be_bytes());
}

fn rewrite_src(data: &mut [u8], ip_header_len: usize, new_addr: Ipv4Addr, new_port: u16) {
    data[12..16].copy_from_slice(&new_addr.octets());
    data[ip_header_len..ip_header_len + 2].copy_from_slice(&new_port.to_be_bytes());
}

// ── WinDivert redirect ────────────────────────────────────────────────────────

/// Opens WinDivert handles and spawns worker threads for NAT redirect.
///
/// Inbound path:  forward layer captures CLIENT→EXTERNAL, rewrites to loopback,
///                injects via network layer so our proxy receives the connection.
/// Return path:   network layer captures proxy response, rewrites src/dst back,
///                injects via network layer with the hotspot interface index.
pub(crate) fn start_redirect(bind_port: u16, log: Logger) -> anyhow::Result<(NatTable, RedirectHandle)> {
    let nat: NatTable = Arc::new(Mutex::new(HashMap::new()));

    let client_filter = "tcp and (tcp.DstPort >= 25560 and tcp.DstPort <= 25570)";

    let return_filter = format!(
        "tcp and ip.SrcAddr == 127.0.0.1 and tcp.SrcPort == {}",
        bind_port
    );

    let flags = WinDivertFlags::new();

    // Forward layer: captures routed hotspot client traffic
    let wd_forward = WinDivert::forward(&client_filter, 0, flags)
        .map_err(|e| anyhow::anyhow!("WinDivert (forward): {}", e))?;

    // Network layer: inject-only handle (filter = "false" captures nothing)
    let wd_inject = WinDivert::network("false", 0, flags)
        .map_err(|e| anyhow::anyhow!("WinDivert (network inject): {}", e))?;

    // Network layer: captures proxy responses on loopback going back to clients
    let wd_return = WinDivert::network(&return_filter, 0, flags)
        .map_err(|e| anyhow::anyhow!("WinDivert (return): {}", e))?;

    // Extract raw HANDLEs before moving handles into threads.
    // RedirectHandle will close them on drop, unblocking any pending recv() calls.
    let raw_forward = unsafe { wd_raw_handle(&wd_forward).0 } as usize;
    let raw_inject  = unsafe { wd_raw_handle(&wd_inject).0 } as usize;
    let raw_return  = unsafe { wd_raw_handle(&wd_return).0 } as usize;
    let redirect = RedirectHandle { handles: vec![raw_forward, raw_inject, raw_return] };

    let nat_client = Arc::clone(&nat);
    let log_client = log.clone();
    std::thread::spawn(move || {
        run_client_intercept_loop(wd_forward, wd_inject, bind_port, nat_client, log_client)
    });

    let nat_return = Arc::clone(&nat);
    let log_return = log.clone();
    std::thread::spawn(move || run_return_intercept_loop(wd_return, nat_return, log_return));

    Ok((nat, redirect))
}

/// Spawns a background thread that prunes NAT entries older than 5 minutes.
pub(crate) fn start_nat_cleanup(nat: NatTable) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let cutoff = Instant::now()
                .checked_sub(std::time::Duration::from_secs(300))
                .unwrap_or_else(Instant::now);
            nat.lock().unwrap().retain(|_, v| v.timestamp > cutoff);
        }
    });
}

// ── Packet loops ──────────────────────────────────────────────────────────────

fn run_client_intercept_loop(
    wd_forward: WinDivert<ForwardLayer>,
    wd_inject: WinDivert<NetworkLayer>,
    bind_port: u16,
    nat: NatTable,
    log: Logger,
) {
    let mut buf = vec![0u8; 65535];
    loop {
        let packet = match wd_forward.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                log.error(format!("WinDivert: ошибка получения пакета (клиент → прокси): {}", e));
                break;
            }
        };

        let packet_slice = match SlicedPacket::from_ip(&packet.data) {
            Ok(s) => s,
            Err(_) => {
                let _ = wd_forward.send(&packet);
                continue;
            }
        };

        let ip_slice = match packet_slice.net.as_ref() {
            Some(etherparse::NetSlice::Ipv4(s)) => s,
            _ => {
                drop(packet_slice);
                let _ = wd_forward.send(&packet);
                continue;
            }
        };

        let tcp_slice = match packet_slice.transport.as_ref() {
            Some(etherparse::TransportSlice::Tcp(s)) => s,
            _ => {
                drop(packet_slice);
                let _ = wd_forward.send(&packet);
                continue;
            }
        };

        let ip_header_len = ip_slice.header().ihl() as usize * 4;
        let src_addr = ip_slice.header().source_addr();
        let src_port = tcp_slice.source_port();
        let dst_addr = ip_slice.header().destination_addr();
        let dst_port = tcp_slice.destination_port();
        let interface_id = packet.address.interface_index();
        let subinterface_id = packet.address.subinterface_index();

        drop(packet_slice);

        // Key by (127.0.0.1, client_port) — matches the dst of the proxy's response
        nat.lock().unwrap().insert(
            (Ipv4Addr::LOCALHOST, src_port),
            NatEntry {
                client_addr: src_addr,
                client_port: src_port,
                original_dst_ip: dst_addr,
                original_dst_port: dst_port,
                interface_id,
                subinterface_id,
                timestamp: Instant::now(),
            },
        );

        // CLIENT:port → EXTERNAL:MC_PORT  =>  127.0.0.1:port → 127.0.0.1:bind_port
        let mut data = packet.data.to_vec();
        rewrite_src(&mut data, ip_header_len, Ipv4Addr::LOCALHOST, src_port);
        rewrite_dst(&mut data, ip_header_len, Ipv4Addr::LOCALHOST, bind_port);

        let mut net_packet = unsafe { WinDivertPacket::<NetworkLayer>::new(data) };
        net_packet.address.set_outbound(true);
        net_packet.address.as_mut().set_loopback(true);

        if let Err(e) = net_packet.recalculate_checksums(Default::default()) {
            log.warn(format!("WinDivert: ошибка контрольной суммы (клиент): {}", e));
            continue;
        }
        if let Err(e) = wd_inject.send(&net_packet) {
            log.warn(format!("WinDivert: ошибка отправки (клиент): {}", e));
        }
    }
}

fn run_return_intercept_loop(wd: WinDivert<NetworkLayer>, nat: NatTable, log: Logger) {
    let mut buf = vec![0u8; 65535];
    loop {
        let packet = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                log.error(format!("WinDivert: ошибка получения пакета (ответ → клиент): {}", e));
                break;
            }
        };

        let packet_slice = match SlicedPacket::from_ip(&packet.data) {
            Ok(s) => s,
            Err(_) => {
                let _ = wd.send(&packet);
                continue;
            }
        };

        let ip_slice = match packet_slice.net.as_ref() {
            Some(etherparse::NetSlice::Ipv4(s)) => s,
            _ => {
                drop(packet_slice);
                let _ = wd.send(&packet);
                continue;
            }
        };

        let tcp_slice = match packet_slice.transport.as_ref() {
            Some(etherparse::TransportSlice::Tcp(s)) => s,
            _ => {
                drop(packet_slice);
                let _ = wd.send(&packet);
                continue;
            }
        };

        let ip_header_len = ip_slice.header().ihl() as usize * 4;
        let dst_addr = ip_slice.header().destination_addr();
        let dst_port = tcp_slice.destination_port();

        drop(packet_slice);

        let entry = nat.lock().unwrap().get(&(dst_addr, dst_port)).map(|e| {
            (
                e.client_addr,
                e.client_port,
                e.original_dst_ip,
                e.original_dst_port,
                e.interface_id,
                e.subinterface_id,
            )
        });

        let (client_addr, client_port, orig_ip, orig_port, interface_id, subinterface_id) =
            match entry {
                Some(e) => e,
                None => {
                    let _ = wd.send(&packet);
                    continue;
                }
            };

        // 127.0.0.1:bind_port → 127.0.0.1:client_port
        //  =>  EXTERNAL:MC_PORT → CLIENT:client_port
        let mut data = packet.data.to_vec();
        rewrite_src(&mut data, ip_header_len, orig_ip, orig_port);
        rewrite_dst(&mut data, ip_header_len, client_addr, client_port);

        let mut net_packet = unsafe { WinDivertPacket::<NetworkLayer>::new(data) };
        net_packet.address.set_outbound(true);
        net_packet.address.set_interface_index(interface_id);
        net_packet.address.set_subinterface_index(subinterface_id);

        if let Err(e) = net_packet.recalculate_checksums(Default::default()) {
            log.warn(format!("WinDivert: ошибка контрольной суммы (ответ): {}", e));
            continue;
        }
        if let Err(e) = wd.send(&net_packet) {
            log.warn(format!("WinDivert: ошибка отправки (ответ): {}", e));
        }
    }
}
