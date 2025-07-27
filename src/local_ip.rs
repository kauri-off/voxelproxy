use std::net::Ipv4Addr;

#[cfg(target_os = "windows")]
pub fn get_local_ip() -> Option<Ipv4Addr> {
    use windows::{
        Win32::Foundation::ERROR_BUFFER_OVERFLOW,
        Win32::NetworkManagement::IpHelper::*,
        Win32::Networking::WinSock::{AF_INET, AF_UNSPEC},
    };

    let mut preferred_ip: Option<Ipv4Addr> = None;

    unsafe {
        let mut buffer_length: u32 = 15000;
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
            return None;
        }

        let mut adapter = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !adapter.is_null() {
            let has_gateway = !(*adapter).FirstGatewayAddress.is_null();

            if has_gateway {
                let mut addr = (*adapter).FirstUnicastAddress;
                while !addr.is_null() {
                    let sockaddr = (*addr).Address.lpSockaddr;
                    if !sockaddr.is_null() && (*sockaddr).sa_family == AF_INET {
                        let octets = std::slice::from_raw_parts(sockaddr as *const u8, 16);
                        let ip = Ipv4Addr::new(octets[4], octets[5], octets[6], octets[7]);

                        if ip.octets()[0] == 192 && ip.octets()[1] == 168 {
                            return Some(ip); 
                        } else if preferred_ip.is_none() {
                            preferred_ip = Some(ip); 
                        }
                    }
                    addr = (*addr).Next;
                }
            }

            adapter = (*adapter).Next;
        }
    }

    return preferred_ip;
}

#[cfg(not(target_os = "windows"))]
pub fn get_local_ip() -> Option<Ipv4Addr> {
    use get_if_addrs::get_if_addrs;
    get_if_addrs()
        .unwrap()
        .into_iter()
        .find(|iface| {
            if iface.is_loopback() || iface.addr.ip().is_ipv6() {
                false
            } else {
                true
            }
        })
        .map(|t| match t.ip() {
            std::net::IpAddr::V4(ipv4_addr) => Some(ipv4_addr),
            std::net::IpAddr::V6(ipv6_addr) => None,
        })
        .unwrap_or(None)
}
