use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use etherparse::SlicedPacket;
use windivert::{
    WinDivert,
    layer::{ForwardLayer, NetworkLayer, WinDivertLayerTrait},
    packet::WinDivertPacket,
    prelude::WinDivertFlags,
};
use windows::Win32::Foundation::HANDLE;

use crate::events::EventSink;
use crate::logger::Logger;

// Raw FFI so another thread can shut down handles blocked in recv().
unsafe extern "C" {
    fn WinDivertShutdown(handle: HANDLE, how: u32) -> u32;
    fn WinDivertClose(handle: HANDLE) -> u32;
}

/// # Safety
/// Relies on `handle` being laid out first in windivert-0.6.0's `WinDivert<L>`.
unsafe fn wd_raw_handle<L: WinDivertLayerTrait>(wd: &WinDivert<L>) -> HANDLE {
    unsafe { (wd as *const WinDivert<L> as *const HANDLE).read() }
}

/// Closes the WinDivert handles on drop, which stops the worker threads.
pub(crate) struct RedirectHandle {
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

struct NatEntry {
    client_addr: Ipv4Addr,
    client_port: u16,
    original_dst_ip: Ipv4Addr,
    original_dst_port: u16,
    interface_id: u32,
    subinterface_id: u32,
    timestamp: Instant,
}

type NatMap = Mutex<HashMap<(Ipv4Addr, u16), NatEntry>>;
type NatTable = Arc<NatMap>;

fn rewrite_dst(data: &mut [u8], ip_header_len: usize, new_addr: Ipv4Addr, new_port: u16) {
    data[16..20].copy_from_slice(&new_addr.octets());
    data[ip_header_len + 2..ip_header_len + 4].copy_from_slice(&new_port.to_be_bytes());
}

fn rewrite_src(data: &mut [u8], ip_header_len: usize, new_addr: Ipv4Addr, new_port: u16) {
    data[12..16].copy_from_slice(&new_addr.octets());
    data[ip_header_len..ip_header_len + 2].copy_from_slice(&new_port.to_be_bytes());
}

/// Redirects hotspot client traffic to the local proxy and rewrites replies back.
pub(crate) fn start_redirect(
    bind_port: u16,
    port_min: u16,
    port_max: u16,
    events: EventSink,
) -> anyhow::Result<RedirectHandle> {
    let nat: NatTable = Arc::new(Mutex::new(HashMap::new()));

    let client_filter = format!(
        "tcp and (tcp.DstPort >= {} and tcp.DstPort <= {})",
        port_min, port_max
    );

    let return_filter = format!(
        "tcp and ip.SrcAddr == 127.0.0.1 and tcp.SrcPort == {}",
        bind_port
    );

    let flags = WinDivertFlags::new();

    // Forward layer: captures routed hotspot client traffic
    let wd_forward = WinDivert::forward(client_filter.as_str(), 0, flags)
        .map_err(|e| anyhow::anyhow!("WinDivert (forward): {}", e))?;

    // Network layer: inject-only handle (filter = "false" captures nothing)
    let wd_inject = WinDivert::network("false", 0, flags)
        .map_err(|e| anyhow::anyhow!("WinDivert (network inject): {}", e))?;

    // Network layer: captures proxy responses on loopback going back to clients
    let wd_return = WinDivert::network(&return_filter, 0, flags)
        .map_err(|e| anyhow::anyhow!("WinDivert (return): {}", e))?;

    let raw_forward = unsafe { wd_raw_handle(&wd_forward).0 } as usize;
    let raw_inject = unsafe { wd_raw_handle(&wd_inject).0 } as usize;
    let raw_return = unsafe { wd_raw_handle(&wd_return).0 } as usize;
    let redirect = RedirectHandle {
        handles: vec![raw_forward, raw_inject, raw_return],
    };

    let nat_client = Arc::clone(&nat);
    let events2 = events.clone();
    std::thread::spawn(move || {
        run_client_intercept_loop(wd_forward, wd_inject, bind_port, nat_client, events2)
    });

    let nat_cleanup = Arc::downgrade(&nat);
    std::thread::spawn(move || run_return_intercept_loop(wd_return, nat, events));
    std::thread::spawn(move || run_nat_cleanup(nat_cleanup));

    Ok(redirect)
}

// Exits once both intercept loops have stopped and dropped the table.
fn run_nat_cleanup(nat: Weak<NatMap>) {
    loop {
        std::thread::sleep(Duration::from_secs(60));
        let Some(nat) = nat.upgrade() else { break };
        let cutoff = Instant::now()
            .checked_sub(Duration::from_secs(300))
            .unwrap_or_else(Instant::now);
        nat.lock().unwrap().retain(|_, v| v.timestamp > cutoff);
    }
}

fn run_client_intercept_loop(
    wd_forward: WinDivert<ForwardLayer>,
    wd_inject: WinDivert<NetworkLayer>,
    bind_port: u16,
    nat: NatTable,
    events: EventSink,
) {
    let log = Logger::new(&events);
    let mut buf = vec![0u8; 65535];
    loop {
        let packet = match wd_forward.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                log.error(format!(
                    "WinDivert: ошибка получения пакета (клиент → прокси): {}",
                    e
                ));
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
            log.warn(format!(
                "WinDivert: ошибка контрольной суммы (клиент): {}",
                e
            ));
            continue;
        }
        if let Err(e) = wd_inject.send(&net_packet) {
            log.warn(format!("WinDivert: ошибка отправки (клиент): {}", e));
        }
    }
}

fn run_return_intercept_loop(wd: WinDivert<NetworkLayer>, nat: NatTable, events: EventSink) {
    let log = Logger::new(&events);
    let mut buf = vec![0u8; 65535];
    loop {
        let packet = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                log.error(format!(
                    "WinDivert: ошибка получения пакета (ответ → клиент): {}",
                    e
                ));
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
            log.warn(format!(
                "WinDivert: ошибка контрольной суммы (ответ): {}",
                e
            ));
            continue;
        }
        if let Err(e) = wd.send(&net_packet) {
            log.warn(format!("WinDivert: ошибка отправки (ответ): {}", e));
        }
    }
}
