//! 私网与保留地址。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return blocked_v4(v4);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || in_net_v6(v6, Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10)
                || in_net_v6(v6, Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7)
                || in_net_v6(v6, Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32)
        }
    }
}

fn blocked_v4(v4: Ipv4Addr) -> bool {
    v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_multicast()
        || in_net_v4(v4, Ipv4Addr::new(0, 0, 0, 0), 8)
        || in_net_v4(v4, Ipv4Addr::new(100, 64, 0, 0), 10)
        || in_net_v4(v4, Ipv4Addr::new(192, 0, 0, 0), 24)
        || in_net_v4(v4, Ipv4Addr::new(198, 18, 0, 0), 15)
        || in_net_v4(v4, Ipv4Addr::new(240, 0, 0, 0), 4)
}

fn in_net_v4(ip: Ipv4Addr, base: Ipv4Addr, bits: u32) -> bool {
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    (u32::from(ip) & mask) == (u32::from(base) & mask)
}

fn in_net_v6(ip: Ipv6Addr, base: Ipv6Addr, bits: u32) -> bool {
    let ip = u128::from(ip);
    let base = u128::from(base);
    let mask = if bits == 0 {
        0
    } else {
        u128::MAX << (128 - bits)
    };
    (ip & mask) == (base & mask)
}
