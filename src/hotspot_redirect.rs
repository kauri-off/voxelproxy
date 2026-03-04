use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windivert::prelude::*;

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

// ── Hotspot subnet detection ──────────────────────────────────────────────────

/// Finds the Windows Mobile Hotspot adapter (192.168.137.x).
/// Returns `None` if no hotspot adapter is active.
pub fn detect_hotspot_subnet() -> Option<HotspotSubnet> {
    use windows::{
        Win32::Foundation::ERROR_BUFFER_OVERFLOW,
        Win32::NetworkManagement::IpHelper::*,
        Win32::Networking::WinSock::{AF_INET, AF_UNSPEC},
    };

    unsafe {
        let mut buf_len: u32 = 15_000;
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

/// Opens two WinDivert handles (driver installation runs on the calling thread),
/// then spawns minimal-stack worker threads that only run the packet recv/send loop.
pub(crate) fn start_redirect(subnet: &HotspotSubnet, bind_port: u16) -> anyhow::Result<NatTable> {
    let nat: NatTable = Arc::new(Mutex::new(HashMap::new()));

    let net = subnet.network.octets();

    let client_filter = format!(
        "ip and tcp and inbound and \
         (tcp.DstPort >= 25560 and tcp.DstPort <= 25570) and \
         ip.SrcAddr >= {}.{}.{}.1 and ip.SrcAddr <= {}.{}.{}.254",
        net[0], net[1], net[2], net[0], net[1], net[2],
    );

    let return_filter = format!(
        "ip and tcp and outbound and ip.SrcAddr == 127.0.0.1 and tcp.SrcPort == {}",
        bind_port
    );

    let wd_client = WinDivert::network(&client_filter, 0, Default::default())
        .map_err(|e| anyhow::anyhow!("WinDivert (клиенты): {}", e))?;
    let wd_return = WinDivert::network(&return_filter, 0, Default::default())
        .map_err(|e| anyhow::anyhow!("WinDivert (обратный): {}", e))?;

    let nat_client = Arc::clone(&nat);
    std::thread::Builder::new()
        .name("wd-client".to_string())
        .stack_size(512 * 1024)
        .spawn(move || run_client_intercept_loop(wd_client, bind_port, nat_client))?;

    let nat_return = Arc::clone(&nat);
    std::thread::Builder::new()
        .name("wd-return".to_string())
        .stack_size(512 * 1024)
        .spawn(move || run_return_intercept_loop(wd_return, nat_return))?;

    Ok(nat)
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

fn run_client_intercept_loop(wd: WinDivert<NetworkLayer>, bind_port: u16, nat: NatTable) {
    let mut buf = vec![0u8; 65535];
    loop {
        let mut packet = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[WinDivert] recv error (client): {}", e);
                break;
            }
        };

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

        nat.lock().unwrap().insert(
            (src_ip, src_port),
            NatEntry {
                original_dst_ip: dst_ip,
                original_dst_port: dst_port,
                timestamp: Instant::now(),
            },
        );

        let data = packet.data.to_mut();
        data[16] = 127;
        data[17] = 0;
        data[18] = 0;
        data[19] = 1;
        let bp = bind_port.to_be_bytes();
        data[ihl + 2] = bp[0];
        data[ihl + 3] = bp[1];

        if let Err(e) = packet.recalculate_checksums(Default::default()) {
            eprintln!("[WinDivert] checksum error (client): {}", e);
            continue;
        }
        if let Err(e) = wd.send(&packet) {
            eprintln!("[WinDivert] send error (client): {}", e);
        }
    }
}

fn run_return_intercept_loop(wd: WinDivert<NetworkLayer>, nat: NatTable) {
    let mut buf = vec![0u8; 65535];
    loop {
        let mut packet = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[WinDivert] recv error (return): {}", e);
                break;
            }
        };

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

        let entry = nat
            .lock()
            .unwrap()
            .get(&(dst_ip, dst_port))
            .map(|e| (e.original_dst_ip, e.original_dst_port));

        let (orig_ip, orig_port) = match entry {
            Some(e) => e,
            None => {
                let _ = wd.send(&packet);
                continue;
            }
        };

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
            eprintln!("[WinDivert] checksum error (return): {}", e);
            continue;
        }
        if let Err(e) = wd.send(&packet) {
            eprintln!("[WinDivert] send error (return): {}", e);
        }
    }
}
