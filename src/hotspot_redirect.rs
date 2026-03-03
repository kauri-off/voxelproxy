use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windivert::prelude::*;

// ── Public types ──────────────────────────────────────────────────────────────

pub struct HotspotSubnet {
    pub host_ip: Ipv4Addr,
    pub network: Ipv4Addr,
    pub mask: u8,
}

pub(crate) struct NatEntry {
    original_dst_ip: Ipv4Addr,
    original_dst_port: u16,
    timestamp: Instant,
}

pub(crate) type NatTable = Arc<Mutex<HashMap<(Ipv4Addr, u16), NatEntry>>>;

// ── Admin check ───────────────────────────────────────────────────────────────

/// Returns true if the current process is running with elevated (Administrator) privileges.
pub fn is_admin() -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
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

// ── Hotspot subnet detection ──────────────────────────────────────────────────

/// Detects the Windows Mobile Hotspot adapter by looking for a 192.168.137.x address.
/// Returns None if no hotspot adapter is active.
pub fn detect_hotspot_subnet() -> Option<HotspotSubnet> {
    use windows::{
        Win32::Foundation::ERROR_BUFFER_OVERFLOW,
        Win32::NetworkManagement::IpHelper::*,
        Win32::Networking::WinSock::{AF_INET, AF_UNSPEC},
    };

    const INITIAL_BUFFER_SIZE: u32 = 15000;

    unsafe {
        let mut buf_len: u32 = INITIAL_BUFFER_SIZE;
        let mut buf: Vec<u8> = vec![0; buf_len as usize];

        let mut result = GetAdaptersAddresses(
            AF_UNSPEC.0 as u32,
            GAA_FLAG_INCLUDE_GATEWAYS,
            None,
            Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
            &mut buf_len,
        );

        if result == ERROR_BUFFER_OVERFLOW.0 {
            buf.resize(buf_len as usize, 0);
            result = GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_GATEWAYS,
                None,
                Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut buf_len,
            );
        }

        if result != 0 {
            return None;
        }

        let mut adapter = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let mut addr = (*adapter).FirstUnicastAddress;
            while !addr.is_null() {
                let sockaddr = (*addr).Address.lpSockaddr;
                if !sockaddr.is_null() && (*sockaddr).sa_family == AF_INET {
                    let octets = std::slice::from_raw_parts(sockaddr as *const u8, 16);
                    let ip = Ipv4Addr::new(octets[4], octets[5], octets[6], octets[7]);
                    let o = ip.octets();
                    if o[0] == 192 && o[1] == 168 && o[2] == 137 {
                        return Some(HotspotSubnet {
                            host_ip: ip,
                            network: Ipv4Addr::new(192, 168, 137, 0),
                            mask: 24,
                        });
                    }
                }
                addr = (*addr).Next;
            }
            adapter = (*adapter).Next;
        }
    }

    None
}

// ── WinDivert redirect ────────────────────────────────────────────────────────

/// Opens WinDivert handles and spawns two OS threads to:
/// - Intercept inbound TCP packets from hotspot clients targeting ports 25560–25570
///   and rewrite their destination to 127.0.0.1:bind_port.
/// - Intercept outbound TCP packets from the proxy back to hotspot clients
///   and rewrite the source back to the original server address.
///
/// Both handles are opened here so any WinDivert errors are propagated immediately.
pub(crate) fn start_redirect(subnet: &HotspotSubnet, bind_port: u16) -> anyhow::Result<NatTable> {
    let nat: NatTable = Arc::new(Mutex::new(HashMap::new()));

    let net = subnet.network.octets();

    // Intercept outgoing connections from hotspot clients to Minecraft port range.
    // These arrive as inbound packets on the hotspot adapter before ICS NATs them.
    let client_filter = format!(
        "ip and tcp and inbound and \
         (tcp.DstPort >= 25560 and tcp.DstPort <= 25570) and \
         ip.SrcAddr >= {}.{}.{}.1 and ip.SrcAddr <= {}.{}.{}.254",
        net[0], net[1], net[2],
        net[0], net[1], net[2],
    );

    // Intercept return packets from our proxy (127.0.0.1:bind_port) back to hotspot clients.
    let return_filter = format!(
        "ip and tcp and outbound and ip.SrcAddr == 127.0.0.1 and tcp.SrcPort == {}",
        bind_port
    );

    let client_wd = WinDivert::network(&client_filter, 0, Default::default())
        .map_err(|e| anyhow::anyhow!("WinDivert (перехват клиентов) не удалось открыть: {}", e))?;

    let return_wd = WinDivert::network(&return_filter, 0, Default::default())
        .map_err(|e| anyhow::anyhow!("WinDivert (обратный путь) не удалось открыть: {}", e))?;

    let nat_client = Arc::clone(&nat);
    let nat_return = Arc::clone(&nat);

    std::thread::spawn(move || run_client_intercept_loop(client_wd, bind_port, nat_client));
    std::thread::spawn(move || run_return_intercept_loop(return_wd, nat_return));

    Ok(nat)
}

/// Spawns a background cleanup thread that removes NAT entries older than 5 minutes.
pub(crate) fn start_nat_cleanup(nat: NatTable) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(300))
            .unwrap_or_else(Instant::now);
        let mut table = nat.lock().unwrap();
        table.retain(|_, v| v.timestamp > cutoff);
    });
}

// ── WinDivert worker threads ──────────────────────────────────────────────────

/// Intercepts TCP packets from hotspot clients destined for ports 25560–25570.
/// Saves the original destination in the NAT table, then rewrites the destination
/// to 127.0.0.1:bind_port and reinjects the packet.
fn run_client_intercept_loop(
    wd: WinDivert<NetworkLayer>,
    bind_port: u16,
    nat: NatTable,
) {
    let mut buf = vec![0u8; 65535];
    loop {
        let mut packet = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[WinDivert] Ошибка recv (клиент): {}", e);
                break;
            }
        };

        // Parse IP+TCP headers from borrowed data
        let (src_ip, dst_ip, src_port, dst_port, ihl) = {
            let data = packet.data.as_ref();
            if data.len() < 20 {
                let _ = wd.send(&packet);
                continue;
            }
            let ihl = (data[0] & 0x0F) as usize * 4;
            if data.len() < ihl + 4 {
                let _ = wd.send(&packet);
                continue;
            }
            let src_ip = Ipv4Addr::new(data[12], data[13], data[14], data[15]);
            let dst_ip = Ipv4Addr::new(data[16], data[17], data[18], data[19]);
            let src_port = u16::from_be_bytes([data[ihl], data[ihl + 1]]);
            let dst_port = u16::from_be_bytes([data[ihl + 2], data[ihl + 3]]);
            (src_ip, dst_ip, src_port, dst_port, ihl)
        };

        // Record original destination so the return path can restore it
        {
            let mut table = nat.lock().unwrap();
            table.insert(
                (src_ip, src_port),
                NatEntry {
                    original_dst_ip: dst_ip,
                    original_dst_port: dst_port,
                    timestamp: Instant::now(),
                },
            );
        }

        // Rewrite dst → 127.0.0.1:bind_port (claims ownership of data, triggers clone)
        let data = packet.data.to_mut();
        data[16] = 127;
        data[17] = 0;
        data[18] = 0;
        data[19] = 1;
        let bp = bind_port.to_be_bytes();
        data[ihl + 2] = bp[0];
        data[ihl + 3] = bp[1];

        if let Err(e) = packet.recalculate_checksums(Default::default()) {
            eprintln!("[WinDivert] Ошибка контрольной суммы (клиент): {}", e);
            continue;
        }

        if let Err(e) = wd.send(&packet) {
            eprintln!("[WinDivert] Ошибка send (клиент): {}", e);
        }
    }
}

/// Intercepts return packets from our proxy (127.0.0.1:bind_port) going back to
/// hotspot clients. Looks up the NAT table to find the original server address and
/// rewrites the source IP/port accordingly, so the hotspot client sees the real server.
fn run_return_intercept_loop(wd: WinDivert<NetworkLayer>, nat: NatTable) {
    let mut buf = vec![0u8; 65535];
    loop {
        let mut packet = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[WinDivert] Ошибка recv (обратный): {}", e);
                break;
            }
        };

        // Parse headers: src=127.0.0.1:bind_port, dst=client_ip:client_ephemeral_port
        let (dst_ip, dst_port, ihl) = {
            let data = packet.data.as_ref();
            if data.len() < 20 {
                let _ = wd.send(&packet);
                continue;
            }
            let ihl = (data[0] & 0x0F) as usize * 4;
            if data.len() < ihl + 4 {
                let _ = wd.send(&packet);
                continue;
            }
            let dst_ip = Ipv4Addr::new(data[16], data[17], data[18], data[19]);
            let dst_port = u16::from_be_bytes([data[ihl + 2], data[ihl + 3]]);
            (dst_ip, dst_port, ihl)
        };

        // Look up: (client_ip, client_ephemeral_port) → original server (ip, port)
        let entry = {
            let table = nat.lock().unwrap();
            table
                .get(&(dst_ip, dst_port))
                .map(|e| (e.original_dst_ip, e.original_dst_port))
        };

        let (orig_ip, orig_port) = match entry {
            Some(e) => e,
            None => {
                // Not a tracked connection, pass through unchanged
                let _ = wd.send(&packet);
                continue;
            }
        };

        // Rewrite src → original server IP:port
        let data = packet.data.to_mut();
        let orig_octets = orig_ip.octets();
        data[12] = orig_octets[0];
        data[13] = orig_octets[1];
        data[14] = orig_octets[2];
        data[15] = orig_octets[3];
        let op = orig_port.to_be_bytes();
        data[ihl] = op[0];
        data[ihl + 1] = op[1];

        if let Err(e) = packet.recalculate_checksums(Default::default()) {
            eprintln!("[WinDivert] Ошибка контрольной суммы (обратный): {}", e);
            continue;
        }

        if let Err(e) = wd.send(&packet) {
            eprintln!("[WinDivert] Ошибка send (обратный): {}", e);
        }
    }
}
