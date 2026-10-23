//! 私网与保留地址。
//!
//! `198.18.0.0/15` 是 RFC 2544 的基准测试段，但 Clash 等 TUN 代理默认把它当作
//! fake-IP 使用。默认放行这一段，否则所有域名都会在解析后被判成私网；真实的
//! 内网地址、回环和链路本地地址仍然拦截。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = embedded_v4(v6) {
                if blocked_v4(v4) {
                    return true;
                }
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

/// TUN 代理常用的 fake-IP 段。放行但在日志里标注，便于区分真实地址。
pub fn is_fake_ip(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v4) if in_net_v4(v4, Ipv4Addr::new(198, 18, 0, 0), 15))
}

fn embedded_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4() {
        return Some(v4);
    }
    let seg = v6.segments();
    if seg[0] == 0x2002 {
        return Some(Ipv4Addr::new(
            (seg[1] >> 8) as u8,
            (seg[1] & 0xff) as u8,
            (seg[2] >> 8) as u8,
            (seg[2] & 0xff) as u8,
        ));
    }
    if seg[0] == 0x0064
        && seg[1] == 0xff9b
        && seg[2] == 0
        && seg[3] == 0
        && seg[4] == 0
        && seg[5] == 0
    {
        let octets = v6.octets();
        return Some(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ));
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_ip_is_not_private() {
        assert!(!is_blocked("198.18.3.208".parse().unwrap()));
        assert!(!is_blocked("198.19.0.1".parse().unwrap()));
    }

    #[test]
    fn real_private_ranges_stay_blocked() {
        for ip in ["127.0.0.1", "10.0.0.1", "192.168.1.1", "169.254.169.254"] {
            assert!(is_blocked(ip.parse().unwrap()), "{ip}");
        }
    }
}
