use std::net::Ipv4Addr;

/// IPv4 addresses of all adapters, with whether the adapter has a gateway.
#[cfg(target_os = "windows")]
fn adapter_ipv4s() -> Vec<(Ipv4Addr, bool)> {
    use windows::{
        Win32::Foundation::ERROR_BUFFER_OVERFLOW,
        Win32::NetworkManagement::IpHelper::*,
        Win32::Networking::WinSock::{AF_INET, AF_UNSPEC},
    };

    const INITIAL_ADAPTER_BUFFER_SIZE: u32 = 15000;

    let mut found = Vec::new();

    unsafe {
        let mut buffer_length: u32 = INITIAL_ADAPTER_BUFFER_SIZE;
        let mut buffer: Vec<u8> = vec![0; buffer_length as usize];

        let mut result = GetAdaptersAddresses(
            AF_UNSPEC.0 as u32,
            GAA_FLAG_INCLUDE_GATEWAYS,
            None,
            Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
            &mut buffer_length,
        );

        if result == ERROR_BUFFER_OVERFLOW.0 {
            buffer.resize(buffer_length as usize, 0);
            result = GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_GATEWAYS,
                None,
                Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut buffer_length,
            );
        }

        if result != 0 {
            return found;
        }

        let mut adapter = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let has_gateway = !(*adapter).FirstGatewayAddress.is_null();
            let mut addr = (*adapter).FirstUnicastAddress;
            while !addr.is_null() {
                let sockaddr = (*addr).Address.lpSockaddr;
                if !sockaddr.is_null() && (*sockaddr).sa_family == AF_INET {
                    let octets = std::slice::from_raw_parts(sockaddr as *const u8, 16);
                    let ip = Ipv4Addr::new(octets[4], octets[5], octets[6], octets[7]);
                    found.push((ip, has_gateway));
                }
                addr = (*addr).Next;
            }
            adapter = (*adapter).Next;
        }
    }

    found
}

#[cfg(target_os = "windows")]
pub fn get_local_ip() -> Option<Ipv4Addr> {
    let candidates: Vec<Ipv4Addr> = adapter_ipv4s()
        .into_iter()
        .filter(|(_, has_gateway)| *has_gateway)
        .map(|(ip, _)| ip)
        .collect();
    candidates
        .iter()
        .find(|ip| ip.octets()[0] == 192 && ip.octets()[1] == 168)
        .or_else(|| candidates.first())
        .copied()
}

#[cfg(target_os = "windows")]
pub fn describe_interfaces() -> String {
    adapter_ipv4s()
        .iter()
        .map(|(ip, gw)| format!("{ip}{}", if *gw { " (gateway)" } else { "" }))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(not(target_os = "windows"))]
pub fn get_local_ip() -> Option<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .find(|iface| !iface.is_loopback() && iface.addr.ip().is_ipv4())
        .and_then(|iface| {
            if let std::net::IpAddr::V4(v4) = iface.ip() {
                Some(v4)
            } else {
                None
            }
        })
}

#[cfg(not(target_os = "windows"))]
pub fn describe_interfaces() -> String {
    match if_addrs::get_if_addrs() {
        Ok(ifaces) => ifaces
            .iter()
            .map(|i| format!("{}={}", i.name, i.ip()))
            .collect::<Vec<_>>()
            .join(", "),
        Err(e) => format!("error: {e}"),
    }
}
