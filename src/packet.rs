//! Link/IP/UDP decoding: Ethernet (with VLAN tags), raw IP (e.g. WireGuard)
//! and Linux cooked captures.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const DLT_EN10MB: i32 = 1;
pub const DLT_RAW: i32 = 12;
pub const DLT_RAW_OPENBSD: i32 = 14;
pub const LINKTYPE_RAW: i32 = 101;
pub const DLT_LINUX_SLL: i32 = 113;
pub const DLT_LINUX_SLL2: i32 = 276;

pub fn supported_linktype(lt: i32) -> bool {
    matches!(
        lt,
        DLT_EN10MB | DLT_RAW | DLT_RAW_OPENBSD | LINKTYPE_RAW | DLT_LINUX_SLL | DLT_LINUX_SLL2
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpPacket<'a> {
    pub src: IpAddr,
    pub dst: IpAddr,
    pub sport: u16,
    pub dport: u16,
    /// UDP length field (header + payload), as read by the original (`g_nUdpLenth`).
    pub udp_len: u16,
    pub payload: &'a [u8],
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// Offset of the IP header in a captured frame, or None for non-IP frames.
pub fn ip_offset(linktype: i32, data: &[u8]) -> Option<usize> {
    match linktype {
        DLT_EN10MB => {
            let mut et = be16(data, 12)?;
            let mut off = 14;
            while et == 0x8100 || et == 0x88a8 {
                et = be16(data, off + 2)?;
                off += 4;
            }
            (et == 0x0800 || et == 0x86dd).then_some(off)
        }
        DLT_RAW | DLT_RAW_OPENBSD | LINKTYPE_RAW => Some(0),
        DLT_LINUX_SLL => Some(16),
        DLT_LINUX_SLL2 => Some(20),
        _ => None,
    }
}

pub fn parse(linktype: i32, data: &[u8]) -> Option<UdpPacket<'_>> {
    let off = ip_offset(linktype, data)?;
    let ip = data.get(off..)?;
    let (src, dst, l4): (IpAddr, IpAddr, &[u8]) = match ip.first()? >> 4 {
        4 => {
            let ihl = ((ip[0] & 0x0f) as usize) * 4;
            if ihl < 20 || *ip.get(9)? != 17 {
                return None;
            }
            let s: [u8; 4] = ip.get(12..16)?.try_into().ok()?;
            let d: [u8; 4] = ip.get(16..20)?.try_into().ok()?;
            (
                Ipv4Addr::from(s).into(),
                Ipv4Addr::from(d).into(),
                ip.get(ihl..)?,
            )
        }
        6 => {
            if *ip.get(6)? != 17 {
                return None;
            }
            let s: [u8; 16] = ip.get(8..24)?.try_into().ok()?;
            let d: [u8; 16] = ip.get(24..40)?.try_into().ok()?;
            (
                Ipv6Addr::from(s).into(),
                Ipv6Addr::from(d).into(),
                ip.get(40..)?,
            )
        }
        _ => return None,
    };
    let sport = be16(l4, 0)?;
    let dport = be16(l4, 2)?;
    let udp_len = be16(l4, 4)?;
    let rest = l4.get(8..)?;
    let plen = (udp_len as usize).saturating_sub(8).min(rest.len());
    Some(UdpPacket {
        src,
        dst,
        sport,
        dport,
        udp_len,
        payload: &rest[..plen],
    })
}

#[cfg(test)]
pub use tests::eth_ipv4_udp;

#[cfg(test)]
mod tests {
    use super::*;

    pub fn eth_ipv4_udp(src: [u8; 4], dst: [u8; 4], sp: u16, dp: u16, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 12];
        v.extend_from_slice(&[0x08, 0x00]);
        let tot = 20 + 8 + payload.len();
        v.extend_from_slice(&[
            0x45,
            0,
            (tot >> 8) as u8,
            tot as u8,
            0,
            0,
            0,
            0,
            64,
            17,
            0,
            0,
        ]);
        v.extend_from_slice(&src);
        v.extend_from_slice(&dst);
        let ul = 8 + payload.len();
        v.extend_from_slice(&sp.to_be_bytes());
        v.extend_from_slice(&dp.to_be_bytes());
        v.extend_from_slice(&(ul as u16).to_be_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn parses_ethernet_ipv4_udp() {
        let f = eth_ipv4_udp([192, 168, 1, 10], [34, 84, 43, 65], 50000, 5521, &[1, 2, 3]);
        let p = parse(DLT_EN10MB, &f).unwrap();
        assert_eq!(p.src.to_string(), "192.168.1.10");
        assert_eq!(p.dst.to_string(), "34.84.43.65");
        assert_eq!((p.sport, p.dport, p.udp_len), (50000, 5521, 11));
        assert_eq!(p.payload, &[1, 2, 3]);
    }

    #[test]
    fn parses_raw_and_rejects_tcp() {
        let f = eth_ipv4_udp([1, 1, 1, 1], [2, 2, 2, 2], 1, 2, &[]);
        assert!(parse(DLT_RAW, &f[14..]).is_some());
        let mut tcp = f.clone();
        tcp[14 + 9] = 6;
        assert!(parse(DLT_EN10MB, &tcp).is_none());
    }

    #[test]
    fn parses_ipv6() {
        let mut f = vec![0x60, 0, 0, 0, 0, 10, 17, 64];
        f.extend_from_slice(&"2001:db8::1".parse::<Ipv6Addr>().unwrap().octets());
        f.extend_from_slice(&"2001:db8::2".parse::<Ipv6Addr>().unwrap().octets());
        f.extend_from_slice(&[0x13, 0x88, 0x13, 0x89, 0, 10, 0, 0, 0xaa, 0xbb]);
        let p = parse(LINKTYPE_RAW, &f).unwrap();
        assert_eq!(p.src.to_string(), "2001:db8::1");
        assert_eq!((p.sport, p.dport), (5000, 5001));
        assert_eq!(p.payload, &[0xaa, 0xbb]);
    }
}
