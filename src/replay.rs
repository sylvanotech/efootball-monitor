//! Offline replay of capture files, and anonymization of captures before
//! sharing them (bug reports, regression fixtures).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::capture_file::{Reader, Writer};
use crate::geo::GeoDb;
use crate::monitor::{Config, MatchSummary, Monitor};
use crate::packet::{self, ip_offset, DLT_EN10MB, DLT_LINUX_SLL, DLT_LINUX_SLL2};
use crate::speed::SpeedServers;
use crate::util::{out, Micros};

/// Packets scanned to guess the capturing host's address.
const GUESS_LIMIT: usize = 200_000;
/// Extra time simulated after the last packet so that running matches end.
const FLUSH_SECONDS: i64 = 20;
/// Ports that never carry game traffic; dropped by `anonymize` (DNS can reveal browsing).
const NOISE_PORTS: [u16; 12] = [
    53, 67, 68, 123, 137, 138, 1900, 5353, 5355, 8000, 12345, 12346,
];

/// The address seen in the most UDP packets: in a capture made on the gaming PC
/// (or of a mirrored console) every game packet involves it.
pub fn guess_local(path: &Path) -> Result<Option<IpAddr>> {
    let mut counts: HashMap<IpAddr, u64> = HashMap::new();
    for rec in Reader::open(path)?.take(GUESS_LIMIT) {
        let rec = rec?;
        if let Some(p) = packet::parse(rec.linktype, &rec.data) {
            *counts.entry(p.src).or_default() += 1;
            *counts.entry(p.dst).or_default() += 1;
        }
    }
    Ok(counts
        .into_iter()
        .max_by_key(|&(ip, n)| (n, ip))
        .map(|(ip, _)| ip))
}

pub struct ReplayOptions {
    pub local_ips: Vec<String>,
    pub realtime: bool,
    pub geo: GeoDb,
    /// Show the overlay window (use with `realtime`).
    pub overlay: Option<crate::overlay::Overlay>,
}

pub struct ReplayResult {
    pub packets: u64,
    pub matches: Vec<MatchSummary>,
    pub speed: Vec<String>,
}

impl ReplayResult {
    /// Stable textual summary, compared against `fixtures/*.expected`.
    pub fn lines(&self) -> Vec<String> {
        self.matches
            .iter()
            .map(ToString::to_string)
            .chain(self.speed.iter().cloned())
            .collect()
    }
}

/// Feeds a capture file through the monitor, driving its clock with the packet
/// timestamps. Active probes are not performed (they would measure today's
/// network, not the recorded one) and no file is written.
pub fn replay(path: &Path, opts: ReplayOptions) -> Result<ReplayResult> {
    let mut local_ips = opts.local_ips;
    if local_ips.is_empty() {
        if let Some(ip) = guess_local(path)? {
            local_ips.push(ip.to_string());
        }
    }
    out(format!(
        "Replaying {} (local address: {})\n",
        path.display(),
        if local_ips.is_empty() {
            "none".to_string()
        } else {
            local_ips.join(", ")
        }
    ));
    let cfg = Config {
        overlay_file: None,
        match_log: None,
    };
    let mut m = Monitor::new(cfg, local_ips, opts.geo, SpeedServers::in_memory());
    m.overlay = opts.overlay;

    let mut packets = 0u64;
    let mut next_tick: Option<Micros> = None;
    let mut prev_ts: Option<Micros> = None;
    for rec in Reader::open(path).with_context(|| format!("cannot read {}", path.display()))? {
        let rec = rec?;
        if opts.realtime {
            if let Some(prev) = prev_ts {
                let gap = (rec.ts - prev).clamp(0, 5_000_000) as u64;
                std::thread::sleep(Duration::from_micros(gap));
            }
            prev_ts = Some(rec.ts);
        }
        let tick = next_tick.get_or_insert(rec.ts + 1_000_000);
        while *tick <= rec.ts {
            m.tick(*tick);
            *tick += 1_000_000;
        }
        if let Some(p) = packet::parse(rec.linktype, &rec.data) {
            m.on_packet(&p, rec.ts);
            packets += 1;
        }
    }
    if let Some(t) = next_tick {
        for k in 0..FLUSH_SECONDS {
            m.tick(t + k * 1_000_000);
        }
    }
    m.finish();
    Ok(ReplayResult {
        packets,
        matches: m.finished.clone(),
        speed: m.speed.summary(),
    })
}

// ---- anonymization

struct Anonymizer {
    local: Option<IpAddr>,
    map: HashMap<IpAddr, IpAddr>,
    next_private: u32,
    next_public: u32,
}

impl Anonymizer {
    fn map(&mut self, ip: IpAddr) -> IpAddr {
        if crate::cloud::lookup(&ip.to_string()).is_some() {
            return ip; // game servers are public infrastructure: keep them
        }
        if let Some(&m) = self.map.get(&ip) {
            return m;
        }
        let private = match ip {
            IpAddr::V4(a) => a.is_private() || a.is_loopback() || a.is_link_local(),
            IpAddr::V6(a) => {
                (a.segments()[0] & 0xfe00) == 0xfc00 || (a.segments()[0] & 0xffc0) == 0xfe80
            }
        };
        let mapped = if Some(ip) == self.local {
            match ip {
                IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)),
            }
        } else if private {
            self.next_private += 1;
            let n = 9 + self.next_private;
            match ip {
                IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::new(192, 168, (n >> 8) as u8, n as u8)),
                IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::new(
                    0xfd00,
                    0,
                    0,
                    0,
                    0,
                    0,
                    (n >> 16) as u16,
                    n as u16,
                )),
            }
        } else {
            self.next_public += 1;
            let n = self.next_public;
            match ip {
                // Documentation ranges (RFC 5737 / RFC 3849): never routed.
                IpAddr::V4(_) => {
                    let (net, host) = ((n - 1) / 254, (n - 1) % 254 + 1);
                    let prefix =
                        [[203, 0, 113], [198, 51, 100], [192, 0, 2]][(net as usize).min(2)];
                    IpAddr::V4(Ipv4Addr::new(prefix[0], prefix[1], prefix[2], host as u8))
                }
                IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::new(
                    0x2001,
                    0xdb8,
                    0,
                    0,
                    0,
                    0,
                    (n >> 16) as u16,
                    n as u16,
                )),
            }
        };
        self.map.insert(ip, mapped);
        mapped
    }
}

fn checksum_add(mut sum: u32, data: &[u8]) -> u32 {
    for c in data.chunks(2) {
        sum += u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32;
    }
    sum
}

fn checksum_finish(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Rewrites one frame in place. Returns false when the frame must be dropped.
fn anonymize_frame(a: &mut Anonymizer, linktype: i32, data: &mut [u8]) -> bool {
    let Some(p) = packet::parse(linktype, data) else {
        return false;
    };
    if NOISE_PORTS.contains(&p.sport) || NOISE_PORTS.contains(&p.dport) {
        return false;
    }
    let multicast = match p.dst {
        IpAddr::V4(d) => d.is_broadcast() || d.is_multicast() || d.octets()[3] == 255,
        IpAddr::V6(d) => d.is_multicast(),
    };
    if multicast {
        return false;
    }
    let (src, dst) = (a.map(p.src), a.map(p.dst));
    let Some(off) = ip_offset(linktype, data) else {
        return false;
    };
    // Hardware addresses identify devices: blank them.
    match linktype {
        DLT_EN10MB => data[..12].fill(0),
        DLT_LINUX_SLL => data[6..14].fill(0),
        DLT_LINUX_SLL2 => data[12..20].fill(0),
        _ => {}
    }
    let ip = &mut data[off..];
    match (src, dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            let ihl = ((ip[0] & 0x0f) as usize) * 4;
            ip[12..16].copy_from_slice(&s.octets());
            ip[16..20].copy_from_slice(&d.octets());
            ip[10..12].fill(0);
            let ck = checksum_finish(checksum_add(0, &ip[..ihl]));
            ip[10..12].copy_from_slice(&ck.to_be_bytes());
            if let Some(udp_ck) = ip.get_mut(ihl + 6..ihl + 8) {
                udp_ck.fill(0); // optional for IPv4
            }
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            ip[8..24].copy_from_slice(&s.octets());
            ip[24..40].copy_from_slice(&d.octets());
            let udp_len = u16::from_be_bytes([ip[44], ip[45]]) as usize;
            ip[46..48].fill(0);
            let end = (40 + udp_len).min(ip.len());
            let mut sum = checksum_add(0, &ip[8..40]);
            sum = checksum_add(sum, &(udp_len as u32).to_be_bytes());
            sum = checksum_add(sum, &17u32.to_be_bytes());
            sum = checksum_add(sum, &ip[40..end]);
            let ck = match checksum_finish(sum) {
                0 => 0xffff,
                c => c,
            };
            ip[46..48].copy_from_slice(&ck.to_be_bytes());
        }
        _ => return false,
    }
    true
}

/// Copies `input` to `output` keeping only game-relevant UDP packets, with
/// personal addresses replaced. Returns (kept, dropped) packet counts.
pub fn anonymize(input: &Path, output: &Path) -> Result<(u64, u64)> {
    if input == output {
        bail!("input and output must be different files");
    }
    let local = guess_local(input)?;
    let mut a = Anonymizer {
        local,
        map: HashMap::new(),
        next_private: 0,
        next_public: 0,
    };
    let mut writer: Option<(Writer, i32)> = None;
    let (mut kept, mut dropped) = (0u64, 0u64);
    for rec in Reader::open(input)? {
        let mut rec = rec?;
        if let Some((_, lt)) = &writer {
            if *lt != rec.linktype {
                dropped += 1; // a classic pcap file holds a single link type
                continue;
            }
        }
        if !anonymize_frame(&mut a, rec.linktype, &mut rec.data) {
            dropped += 1;
            continue;
        }
        if writer.is_none() {
            writer = Some((Writer::create(output, rec.linktype)?, rec.linktype));
        }
        writer.as_mut().unwrap().0.write(rec.ts, &rec.data)?;
        kept += 1;
    }
    match writer {
        Some((mut w, _)) => w.flush()?,
        None => bail!("no game-related UDP packet found in {}", input.display()),
    }
    Ok((kept, dropped))
}

/// Synthetic traffic used by the tests and by `fixtures/synthetic-session.pcap`.
#[cfg(test)]
pub mod synthetic {
    use crate::packet::eth_ipv4_udp;
    use crate::util::Micros;

    pub const ME: [u8; 4] = [192, 168, 1, 20];
    pub const GCP: [u8; 4] = [34, 84, 43, 65]; // Google Cloud asia-northeast1
    pub const AWS: [u8; 4] = [15, 188, 178, 40]; // AWS eu-west-3
    pub const PEER: [u8; 4] = [81, 2, 3, 4];
    pub const T0: Micros = 1_767_000_000_000_000;

    fn msg(t: u16, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        v[..2].copy_from_slice(&t.to_be_bytes());
        v
    }

    /// Frames of a session: speed test, a ~45-second client-server match with
    /// ~10% loss, then a ~40-second peer-to-peer match.
    pub fn session() -> Vec<(Micros, Vec<u8>)> {
        let mut f = Vec::new();
        let mut t = T0;
        // Speed test: 30 pings to each server, the AWS one loses every 10th answer.
        for i in 0..30u16 {
            for (srv, port, rtt) in [(GCP, 5521u16, 35_000), (AWS, 50000, 240_000)] {
                t += 10_000;
                f.push((t, eth_ipv4_udp(ME, srv, 40000, port, &i.to_be_bytes())));
                if !(port == 50000 && i % 10 == 9) {
                    f.push((
                        t + rtt,
                        eth_ipv4_udp(srv, ME, port, 40000, &i.to_be_bytes()),
                    ));
                }
            }
        }
        // Client-server match on Google Cloud: initial latency 42 ms.
        t += 30_000_000;
        f.push((t, eth_ipv4_udp(ME, GCP, 50000, 30123, &msg(0xe660, 20))));
        f.push((
            t + 42_000,
            eth_ipv4_udp(GCP, ME, 30123, 50000, &msg(0xf088, 20)),
        ));
        t += 100_000;
        for g in 0..20u32 {
            for k in 0..256u32 {
                if k % 10 == 3 {
                    continue; // ~10% loss
                }
                let mut d = msg(0xf188, 24);
                d[9] = g as u8;
                f.push((
                    t + (g * 256 + k) as Micros * 8_000,
                    eth_ipv4_udp(GCP, ME, 30123, 50000, &d),
                ));
            }
        }
        t += 20 * 256 * 8_000 + 30_000_000;
        // Peer-to-peer match: request/response, keep-alives every second, game data.
        f.push((t, eth_ipv4_udp(ME, PEER, 50000, 50001, &msg(0x080b, 20))));
        f.push((
            t + 20_000,
            eth_ipv4_udp(PEER, ME, 50001, 50000, &msg(0x090b, 20)),
        ));
        for s in 0..40i64 {
            let base = t + 1_000_000 + s * 1_000_000;
            let mut req = msg(0x080c, 20);
            req[8..20].copy_from_slice(&[s as u8; 12]);
            let mut rsp = msg(0x090c, 20);
            rsp[8..20].copy_from_slice(&[s as u8; 12]);
            f.push((base, eth_ipv4_udp(ME, PEER, 50000, 50001, &req)));
            f.push((
                base + 25_000 + (s % 5) * 1000,
                eth_ipv4_udp(PEER, ME, 50001, 50000, &rsp),
            ));
            for k in 0..128i64 {
                let mut d = msg(0, 24);
                d[2] = (s / 2) as u8; // 256 messages per group
                f.push((
                    base + 30_000 + k * 7_000,
                    eth_ipv4_udp(PEER, ME, 50001, 50000, &d),
                ));
            }
        }
        f.sort_by_key(|(ts, _)| *ts);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_file::Record;
    use std::path::PathBuf;

    fn write_capture(path: &Path, frames: &[(Micros, Vec<u8>)]) {
        let mut w = Writer::create(path, DLT_EN10MB).unwrap();
        for (ts, d) in frames {
            w.write(*ts, d).unwrap();
        }
        w.flush().unwrap();
    }

    fn opts() -> ReplayOptions {
        ReplayOptions {
            local_ips: Vec::new(),
            realtime: false,
            geo: GeoDb::default(),
            overlay: None,
        }
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("efm-replay-{}-{name}", std::process::id()))
    }

    #[test]
    fn replays_a_synthetic_session() {
        let p = tmp("session.pcap");
        write_capture(&p, &synthetic::session());
        assert_eq!(
            guess_local(&p).unwrap(),
            Some("192.168.1.20".parse().unwrap())
        );
        let r = replay(&p, opts()).unwrap();
        std::fs::remove_file(&p).unwrap();
        let lines = r.lines();
        assert_eq!(r.matches.len(), 2, "{lines:#?}");
        let cs = &r.matches[0];
        assert!(!cs.p2p);
        assert_eq!(
            (cs.ip.as_str(), cs.port, cs.initial),
            ("34.84.43.65", 30123, 42)
        );
        assert!((cs.max_loss - 10.2).abs() < 0.5, "{}", cs.max_loss);
        let p2p = &r.matches[1];
        assert!(p2p.p2p);
        assert_eq!(p2p.ip, "81.2.3.4");
        assert!((25..=29).contains(&p2p.avg), "{lines:#?}");
        assert!(
            lines.contains(
                &"speed 15.188.178.40:50000 sent=30 answered=27 min=240ms avg=240ms".to_string()
            ),
            "{lines:#?}"
        );
        assert!(
            lines.contains(
                &"speed 34.84.43.65:5521 sent=30 answered=30 min=35ms avg=35ms".to_string()
            ),
            "{lines:#?}"
        );
    }

    #[test]
    fn anonymize_keeps_servers_and_hides_people() {
        let input = tmp("anon-in.pcap");
        let output = tmp("anon-out.pcap");
        let mut frames = synthetic::session();
        // Noise that must be dropped: a DNS query.
        frames.push((
            synthetic::T0,
            packet::eth_ipv4_udp(synthetic::ME, [8, 8, 8, 8], 5555, 53, b"example"),
        ));
        write_capture(&input, &frames);
        let (kept, dropped) = anonymize(&input, &output).unwrap();
        assert_eq!((kept, dropped), (frames.len() as u64 - 1, 1));

        let recs: Vec<Record> = Reader::open(&output).unwrap().map(Result::unwrap).collect();
        let mut seen = std::collections::BTreeSet::new();
        for r in &recs {
            assert!(r.data[..12].iter().all(|&b| b == 0), "MAC not blanked");
            let p = packet::parse(r.linktype, &r.data).unwrap();
            seen.insert(p.src.to_string());
            seen.insert(p.dst.to_string());
            // IPv4 header checksum must be valid again.
            assert_eq!(checksum_finish(checksum_add(0, &r.data[14..34])), 0);
        }
        assert!(seen.contains("34.84.43.65") && seen.contains("15.188.178.40"));
        assert!(seen.contains("192.168.0.1") && seen.contains("203.0.113.1"));
        assert!(!seen.contains("192.168.1.20") && !seen.contains("81.2.3.4"));

        // The anonymized capture must give the same result.
        let a = replay(&input, opts()).unwrap().lines();
        let b = replay(&output, opts()).unwrap().lines();
        std::fs::remove_file(&input).unwrap();
        std::fs::remove_file(&output).unwrap();
        let b: Vec<String> = b
            .into_iter()
            .map(|l| l.replace("203.0.113.1", "81.2.3.4"))
            .collect();
        assert_eq!(a, b);
    }

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    /// Every `fixtures/<name>.pcap[ng]` must replay to the lines in `fixtures/<name>.expected`.
    #[test]
    fn fixtures_replay_as_expected() {
        let Ok(entries) = std::fs::read_dir(fixtures_dir()) else {
            return;
        };
        let mut failures = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            if !matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("pcap" | "pcapng")
            ) {
                continue;
            }
            let actual = replay(&path, opts()).unwrap().lines();
            let expected_path = path.with_extension("expected");
            let expected: Vec<String> = std::fs::read_to_string(&expected_path)
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect();
            if actual != expected {
                failures.push(format!(
                    "{}\n  expected ({}):\n    {}\n  actual:\n    {}",
                    path.display(),
                    expected_path.display(),
                    expected.join("\n    "),
                    actual.join("\n    ")
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "fixture mismatches:\n{}",
            failures.join("\n\n")
        );
    }

    /// Regenerates the synthetic fixture: `cargo test write_synthetic_fixture -- --ignored`.
    #[test]
    #[ignore]
    fn write_synthetic_fixture() {
        let dir = fixtures_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("synthetic-session.pcap");
        write_capture(&p, &synthetic::session());
        let lines = replay(&p, opts()).unwrap().lines();
        let text = format!(
            "# Synthetic traffic generated by `cargo test write_synthetic_fixture -- --ignored`.\n{}\n",
            lines.join("\n")
        );
        std::fs::write(p.with_extension("expected"), text).unwrap();
    }
}
