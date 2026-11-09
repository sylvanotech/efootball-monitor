//! Active probes used during client-server matches: TCP connect latency to the
//! game server port, or ICMP echo for servers on port 5735.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

/// TCP connect time in ms, or 1000 on failure/timeout (1 s).
pub fn tcping(ip: &str, port: u16) -> i64 {
    let Ok(addr) = ip.parse::<IpAddr>() else {
        thread::sleep(Duration::from_secs(1));
        return 1000;
    };
    let start = Instant::now();
    match TcpStream::connect_timeout(&SocketAddr::new(addr, port), Duration::from_secs(1)) {
        Ok(_) => (start.elapsed().as_millis() as i64).max(1),
        Err(_) => 1000,
    }
}

fn checksum(b: &[u8]) -> u16 {
    let mut sum: u32 = b
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32)
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// ICMP echo round trip in ms, or 10000 on failure.
/// Uses an unprivileged ICMP datagram socket where available (Linux/macOS),
/// otherwise a raw socket (Windows, needs Administrator).
pub fn icmp_ping(ip: &str) -> i64 {
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};
    let Ok(IpAddr::V4(dst)) = ip.parse::<IpAddr>() else {
        return 10_000;
    };
    let (sock, raw) = match Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::ICMPV4)) {
        Ok(s) => (s, false),
        Err(_) => match Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::ICMPV4)) {
            Ok(s) => (s, true),
            Err(_) => return 10_000,
        },
    };
    let _ = sock.set_read_timeout(Some(Duration::from_secs(1)));
    let id = (std::process::id() & 0xffff) as u16;
    let mut pkt = [0u8; 20];
    pkt[0] = 8;
    pkt[4..6].copy_from_slice(&id.to_be_bytes());
    pkt[6..8].copy_from_slice(&1u16.to_be_bytes());
    let ck = checksum(&pkt);
    pkt[2..4].copy_from_slice(&ck.to_be_bytes());
    let start = Instant::now();
    if sock
        .send_to(&pkt, &SockAddr::from(SocketAddr::new(dst.into(), 0)))
        .is_err()
    {
        return 10_000;
    }
    let udp: std::net::UdpSocket = sock.into(); // plain recv() on the ICMP socket
    let mut buf = [0u8; 1024];
    while start.elapsed() < Duration::from_secs(1) {
        let Ok(n) = udp.recv(&mut buf) else {
            return 10_000;
        };
        let icmp = if raw || buf[0] >> 4 == 4 {
            &buf[((buf[0] & 0x0f) as usize * 4).min(n)..n]
        } else {
            &buf[..n]
        };
        // Datagram sockets have their id rewritten by the kernel, so only check it for raw.
        if icmp.len() >= 8
            && icmp[0] == 0
            && u16::from_be_bytes([icmp[6], icmp[7]]) == 1
            && (!raw || u16::from_be_bytes([icmp[4], icmp[5]]) == id)
        {
            return start.elapsed().as_millis() as i64;
        }
    }
    10_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn checksum_rfc1071() {
        // echo request, id=1 seq=1, no payload
        let p = [8u8, 0, 0, 0, 0, 1, 0, 1];
        assert_eq!(checksum(&p), 0xf7fd);
    }

    #[test]
    fn tcping_open_and_closed() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let ms = tcping("127.0.0.1", port);
        assert!((1..900).contains(&ms));
        drop(l);
        // Some sandboxes/proxies accept every TCP connect; only check refusal where the OS refuses.
        let refused = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_secs(1),
        )
        .is_err();
        if refused {
            assert_eq!(tcping("127.0.0.1", port), 1000);
        }
    }
}
