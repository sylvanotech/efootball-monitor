//! Match detection and statistics. See `docs/PROTOCOL.md` for the wire format.
//!
//! Classification is done on the first two UDP payload bytes (big endian),
//! ignoring datagrams sent from the STUN ports 3478/5478:
//!
//! | type             | kind | meaning                                                         |
//! |------------------|------|-----------------------------------------------------------------|
//! | 0xe660 (local→)  | CS   | relay pairing: remember the time                                |
//! | 0xf088 (→local)  | CS   | first server reply ⇒ client-server match, latency = now − e660  |
//! | 0xf188 (→local)  | CS   | relayed game data, byte 9 = group id (256 messages per group)   |
//! | 0x080b / 0x090b  | P2P  | match request / response ⇒ peer-to-peer match                   |
//! | 0x080c / 0x090c  | P2P  | keep-alive; bytes 8..20 = transaction id ⇒ round-trip time       |
//! | 0x0000 (→local)  | P2P  | game data, byte 2 = group id; 100 messages ⇒ session started    |
//! | UDP len 10 on 5521/10000/30000/50000 | in-game speed test (see `speed.rs`)        |

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::geo::GeoDb;
use crate::packet::UdpPacket;
use crate::speed::{is_speed_port, SpeedServers};
use crate::util::{
    fmt_clock, fmt_duration, fmt_full, fmt_hm, fmt_hms, fmt_ms, out, rule, sparkline, status_line,
    verdict, Micros, BOLD, DIM, RESET, YELLOW,
};

pub const PORT_ICMP_ONLY: u16 = 5735; // 0x1667: probe with ICMP instead of TCP
const STUN_PORTS: [u16; 2] = [3478, 5478];

pub struct Config {
    pub overlay_file: Option<PathBuf>,
    /// `None` = do not write a match log (replays).
    pub match_log: Option<PathBuf>,
    /// Matches at or above this latency (ms) are flagged as far.
    pub max_ping: i64,
    /// Print IP addresses and ports (hidden by default, e.g. for streamers).
    pub show_ip: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            overlay_file: None,
            match_log: None,
            max_ping: 120,
            show_ip: false,
        }
    }
}

/// Number of samples kept for the live ping graph.
const GRAPH_LEN: usize = 30;
/// Seconds of match time covered by each recap line.
const RECAP_SECS: i64 = 60;

/// One finished match, in a stable text form used by replays and regression tests.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSummary {
    pub p2p: bool,
    pub ip: String,
    pub port: u16,
    pub duration_s: i64,
    pub initial: i64,
    pub samples: i64,
    pub min: i64,
    pub avg: i64,
    pub max: i64,
    pub max_loss: f32,
}

impl std::fmt::Display for MatchSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = if self.p2p {
            "peer-peer"
        } else {
            "client-server"
        };
        write!(f, "match {mode} remote={}:{}", self.ip, self.port)?;
        if !self.p2p {
            write!(f, " provider=\"{}\"", crate::cloud::describe(&self.ip))?;
        }
        write!(
            f,
            " duration={}s initial={}ms samples={} min={}ms avg={}ms max={}ms max_loss={:.1}%",
            self.duration_s,
            self.initial,
            self.samples,
            self.min,
            self.avg,
            self.max,
            self.max_loss
        )
    }
}

/// Current match.
#[derive(Default, Clone)]
pub struct MatchInfo {
    pub ip: String,
    pub port: u16,
    pub local_ip: String,
    pub local_port: u16,
    pub area: String,
    pub p2p: bool,
    pub active: bool,
    pub start: Micros,
    pub print_count: u32,
    pub initial: i64,
    pub current: i64,
    pub min: i64,
    pub max: i64,
    pub sum: i64,
    pub samples: i64,
    pub avg: i64,
    pub max_loss: f32,
    pub loss: f32,
    /// The initial latency has been added to the statistics.
    pub initial_counted: bool,
}

#[derive(Default)]
struct P2pStats {
    req_ts: Micros,
    req_tid: [u8; 12],
    session_started: bool,
    major_ip: String,
    group: Option<u8>,
    group_count: i32,
    zero_v4: u32,
    zero_v6: u32,
    stun_ticks: u32,
}

#[derive(Default)]
struct PspStats {
    active: bool,
    e660_ts: Micros,
    group: u8,
    group_count: i32,
}

pub struct Monitor {
    pub cfg: Config,
    pub local_ips: Vec<String>,
    pub geo: GeoDb,
    pub speed: SpeedServers,
    pub info: MatchInfo,
    p2p: P2pStats,
    psp: PspStats,
    p2p_loss_prev: f32,
    psp_loss_prev: f32,
    last_p2p: Micros,
    last_psp: Micros,
    match_count: u32,
    /// Recent latency samples for the live graph.
    graph: Vec<i64>,
    /// Samples since the last recap line, and when that period started.
    recap: Vec<i64>,
    recap_start: Micros,
    recap_loss: f32,
    /// The last sample was a spike (alert once per spike, not every second).
    in_spike: bool,
    probe_ticks: u32,
    /// Current time: the last packet timestamp or tick, whichever is later.
    now: Micros,
    /// Matches finished so far.
    pub finished: Vec<MatchSummary>,
    /// Always-on-top window (`--overlay`).
    pub overlay: Option<crate::overlay::Overlay>,
}

impl Monitor {
    pub fn new(cfg: Config, local_ips: Vec<String>, geo: GeoDb, speed: SpeedServers) -> Self {
        Self {
            cfg,
            local_ips,
            geo,
            speed,
            info: MatchInfo::default(),
            p2p: P2pStats::default(),
            psp: PspStats::default(),
            p2p_loss_prev: 0.0,
            psp_loss_prev: 0.0,
            last_p2p: 0,
            last_psp: 0,
            match_count: 1,
            graph: Vec::new(),
            recap: Vec::new(),
            recap_start: 0,
            recap_loss: 0.0,
            in_spike: false,
            probe_ticks: 0,
            now: 0,
            finished: Vec::new(),
            overlay: None,
        }
    }

    /// RFC 1918 addresses are always local (covers a
    /// port-mirrored console); otherwise compare with the host's addresses,
    /// ignoring the last 3 characters of the candidate.
    pub fn is_local(&self, ip: &str) -> bool {
        if ip.len() <= 4 {
            return false;
        }
        if let Ok(std::net::IpAddr::V4(a)) = ip.parse() {
            if a.is_private() {
                return true;
            }
        }
        let n = ip.len() - 3;
        self.local_ips
            .iter()
            .any(|l| l.as_bytes().get(..n) == Some(&ip.as_bytes()[..n]))
    }

    // ---- overlay

    fn write_overlay(&self, content: &str) {
        if let Some(p) = &self.cfg.overlay_file {
            let _ = std::fs::write(p, content);
        }
    }

    pub fn update_overlay(&self) {
        let i = &self.info;
        if let Some(o) = &self.overlay {
            o.show(crate::overlay::Status::Match {
                p2p: i.p2p,
                provider: if i.p2p {
                    String::new()
                } else {
                    crate::cloud::describe(&i.ip)
                },
                area: i.area.clone(),
                latency: if i.current > 0 { i.current } else { i.initial },
                loss: i.loss,
            });
        }
        if self.cfg.overlay_file.is_none() {
            return;
        }
        let elapsed = (self.now - i.start) / 1_000_000;
        let mode = if i.p2p { "peer-peer" } else { "client-server" };
        if elapsed < 91 {
            self.write_overlay(&format!(
                "{} {}:{} {}ms {:.1}%loss\n{}",
                mode, i.ip, i.port, i.current, i.loss, i.area
            ));
        } else {
            self.write_overlay(&format!(
                "{}  {}ms {:.1}% ",
                self.geo.city(&i.ip),
                i.current,
                i.loss
            ));
        }
    }

    // ---- dispatch

    pub fn on_packet(&mut self, p: &UdpPacket, ts: Micros) {
        self.now = self.now.max(ts);
        let src = p.src.to_string();
        let dst = p.dst.to_string();
        if p.udp_len == 10 && (is_speed_port(p.sport) || is_speed_port(p.dport)) {
            self.speed_handler(&src, p.sport, &dst, p.dport, p.payload, ts);
            return;
        }
        if STUN_PORTS.contains(&p.sport) || p.payload.len() < 2 {
            return;
        }
        let msg = u16::from_be_bytes([p.payload[0], p.payload[1]]);
        if is_psp_msg(msg) {
            self.last_psp = ts;
            self.psp_handler(&src, p.sport, &dst, p.dport, p.payload, ts);
        } else if is_p2p_msg(msg) {
            self.last_p2p = ts;
            self.p2p_handler(&src, p.sport, &dst, p.dport, p.payload, ts);
        }
    }

    fn speed_handler(
        &mut self,
        src: &str,
        sport: u16,
        dst: &str,
        dport: u16,
        d: &[u8],
        ts: Micros,
    ) {
        if d.len() != 2 {
            return;
        }
        let id = u16::from_be_bytes([d[0], d[1]]);
        if is_speed_port(dport) {
            self.speed.on_request(dst, dport, id, ts, &self.geo);
        } else if is_speed_port(sport) {
            self.speed.on_response(src, id, ts);
        }
    }

    // ---- client-server matches

    fn psp_handler(&mut self, src: &str, sport: u16, dst: &str, dport: u16, d: &[u8], ts: Micros) {
        let src_local = self.is_local(src);
        match u16::from_be_bytes([d[0], d[1]]) {
            0xe660 if src_local => {
                if !self.psp.active {
                    self.psp.e660_ts = ts;
                }
            }
            0xf088 if !src_local => {
                if !self.psp.active && self.psp.e660_ts != 0 {
                    let delay = (ts - self.psp.e660_ts) / 1000;
                    self.match_start(src, sport, dst, dport, false, delay);
                    self.update_overlay();
                    self.print_new_match();
                }
            }
            0xf188 if !src_local => {
                let Some(&g) = d.get(9) else { return };
                if !self.psp.active {
                    self.psp.active = true;
                    self.psp.group = g;
                    self.psp.group_count = 0;
                    self.match_start(src, sport, dst, dport, false, 0);
                    self.print_new_match();
                }
                if g == self.psp.group {
                    self.psp.group_count += 1;
                } else {
                    let prev = self.psp.group_count;
                    let rate = (256 - prev) as f32 / 256.0 * 100.0;
                    if prev > 10 {
                        self.info.loss = rate;
                        self.update_match_info(0, rate);
                        if (self.psp_loss_prev - rate).abs() > 4.0 && self.info.active {
                            self.loss_alert(ts, rate, self.psp_loss_prev);
                            self.psp_loss_prev = rate;
                        }
                    }
                    self.psp.group = g;
                    self.psp.group_count = 1;
                }
            }
            _ => {}
        }
    }

    // ---- peer-to-peer matches

    fn p2p_handler(&mut self, src: &str, sport: u16, dst: &str, dport: u16, d: &[u8], ts: Micros) {
        match u16::from_be_bytes([d[0], d[1]]) {
            0x0000 => {
                if !self.is_local(src) {
                    self.handle_game_data(src, sport, dst, dport, d, ts);
                }
            }
            0x080b | 0x090b => {
                let major = &self.p2p.major_ip;
                if major.is_empty() || src == major || dst == major {
                    self.handle_stun(src, sport, dst, dport, d, ts);
                }
            }
            0x080c | 0x090c => self.handle_stun(src, sport, dst, dport, d, ts),
            _ => {}
        }
    }

    fn handle_stun(&mut self, src: &str, sport: u16, dst: &str, dport: u16, d: &[u8], ts: Micros) {
        let src_local = self.is_local(src);
        if src_local && self.is_local(dst) {
            return;
        }
        match u16::from_be_bytes([d[0], d[1]]) {
            0x090b if !src_local => self.match_start(src, sport, dst, dport, true, 0),
            0x080b if src_local => self.match_start(dst, dport, src, sport, true, 0),
            0x080c if src_local => {
                if let Some(t) = d.get(8..20) {
                    self.p2p.req_ts = ts;
                    self.p2p.req_tid.copy_from_slice(t);
                }
            }
            0x090c if !src_local => {
                if d.get(8..20) != Some(&self.p2p.req_tid[..]) || self.p2p.req_ts == 0 {
                    return;
                }
                let delay = (ts - self.p2p.req_ts) / 1000;
                if !self.p2p.session_started {
                    self.p2p.stun_ticks = 0;
                    self.match_start(src, sport, dst, dport, true, delay);
                    self.update_overlay();
                    self.print_new_match();
                } else {
                    self.update_match_info(delay, 0.0);
                    self.print_sample(delay);
                    self.p2p.stun_ticks += 1;
                }
            }
            _ => {}
        }
    }

    fn handle_game_data(
        &mut self,
        src: &str,
        sport: u16,
        dst: &str,
        dport: u16,
        d: &[u8],
        _ts: Micros,
    ) {
        let Some(&g) = d.get(2) else { return };
        if self.p2p.group != Some(g) {
            if self.p2p.group.is_some() {
                let received = self.p2p.group_count;
                let rate = (256 - received) as f32 * 100.0 / 256.0;
                if received > 10 {
                    self.info.loss = rate;
                    self.update_match_info(0, rate);
                    if (rate - self.p2p_loss_prev).abs() > 4.0 && self.info.active {
                        self.loss_alert(self.now, rate, self.p2p_loss_prev);
                        self.p2p_loss_prev = rate;
                    }
                }
            }
            self.p2p.group = Some(g);
            self.p2p.group_count = 0;
        }
        self.p2p.group_count += 1;
        let zeros = if src.contains(':') {
            self.p2p.zero_v6 += 1;
            self.p2p.zero_v6
        } else {
            self.p2p.zero_v4 += 1;
            self.p2p.zero_v4
        };
        if zeros < 100 {
            return;
        }
        if self.p2p.major_ip.is_empty() {
            if self.info.ip != src && self.info.print_count > 0 {
                out(format!(
                    "  {YELLOW}!{RESET} Opponent address changed: {}\n",
                    self.place(src, 0)
                ));
            }
            self.info.ip = src.into();
            self.info.area = self.geo.area(src);
            self.p2p.major_ip = src.into();
        }
        if !self.p2p.session_started {
            self.p2p.session_started = true;
            self.match_start(src, sport, dst, dport, true, 0);
            self.print_new_match();
        }
    }

    // ---- match bookkeeping

    fn match_start(
        &mut self,
        ip: &str,
        port: u16,
        local: &str,
        local_port: u16,
        p2p: bool,
        delay: i64,
    ) {
        let i = &self.info;
        if !i.ip.is_empty() && i.ip != ip && i.print_count > 0 {
            out(format!(
                "  {YELLOW}!{RESET} Switching to {}\n",
                self.place(ip, port)
            ));
        }
        let i = &mut self.info;
        i.ip = ip.into();
        i.area = self.geo.area(ip);
        i.local_ip = local.into();
        i.port = port;
        i.local_port = local_port;
        i.p2p = p2p;
        i.start = self.now;
        i.active = true;
        if delay != 0 {
            i.initial = delay;
            i.current = delay;
            i.initial_counted = false;
        }
    }

    pub fn update_match_info(&mut self, latency: i64, loss: f32) {
        let i = &mut self.info;
        if latency > 0 {
            i.current = latency;
            i.sum += latency;
            i.samples += 1;
            i.avg = i.sum / i.samples;
            if i.initial == 0 {
                i.initial = latency;
            }
        }
        i.max = i.max.max(latency);
        if i.min == 0 || (latency > 0 && latency < i.min) {
            i.min = latency;
        }
        if loss > i.max_loss {
            i.max_loss = loss;
        }
    }

    /// Where a remote address is: location, cloud provider, and the address with `--show-ip`.
    fn place(&self, ip: &str, port: u16) -> String {
        let area = self.geo.area(ip);
        let cloud = crate::cloud::describe(ip);
        let mut s = format!("{BOLD}{area}{RESET}");
        if crate::cloud::lookup(ip).is_some() && cloud != area {
            s.push_str(&format!(" ({cloud})"));
        }
        if self.cfg.show_ip {
            if ip.contains(':') {
                s.push_str(&format!("  {DIM}[{ip}]:{port}{RESET}"));
            } else {
                s.push_str(&format!("  {DIM}{ip}:{port}{RESET}"));
            }
        }
        s
    }

    fn loss_alert(&self, ts: Micros, rate: f32, prev: f32) {
        let color = if rate > prev { YELLOW } else { DIM };
        out(format!(
            "  {color}!{RESET} {} packet loss {rate:.1}% (was {prev:.1}%)\n",
            fmt_hms(ts)
        ));
        self.draw_status();
    }

    /// Match header, printed once when a match is detected.
    fn print_new_match(&mut self) {
        let first = self.info.print_count == 0;
        self.info.print_count += 1;
        if self.info.initial > 0 && !self.info.initial_counted {
            self.info.initial_counted = true;
            let initial = self.info.initial;
            self.update_match_info(initial, 0.0);
        }
        if !first {
            return;
        }
        self.graph.clear();
        self.recap.clear();
        self.recap_start = self.now;
        self.recap_loss = 0.0;
        self.in_spike = false;
        let i = &self.info;
        let n = self.match_count;
        self.match_count += 1;
        // Say clearly whose location this is: in client-server the opponent is hidden behind the server.
        let (mode, whose) = if i.p2p {
            (
                format!("Peer-to-peer {DIM}(direct link to your opponent){RESET}"),
                "Opponent:",
            )
        } else {
            (
                format!("Client-server {DIM}(you and your opponent both play through this server){RESET}"),
                "Server:  ",
            )
        };
        let ping = if i.initial > 0 {
            format!(
                "{}   {}",
                fmt_ms(i.initial),
                verdict(i.initial, self.cfg.max_ping)
            )
        } else {
            format!("{DIM}measuring...{RESET}")
        };
        let mut s = format!(
            "\n{}\n  {mode}\n  {whose} {}\n  Ping:     {ping}\n",
            rule(&format!("Match {n} - {}", fmt_hms(self.now))),
            self.place(&i.ip, i.port)
        );
        if i.p2p && i.area == "unknown" && !self.geo.is_loaded() {
            s.push_str(&format!(
                "  {DIM}(add the IP2Location files to see where your opponent is){RESET}\n"
            ));
        }
        if self.cfg.show_ip {
            s.push_str(&format!(
                "  {DIM}you: {}:{}{RESET}\n",
                i.local_ip, i.local_port
            ));
        }
        s.push('\x07');
        out(s);
    }

    /// Adds a latency sample to the live graph.
    fn print_sample(&mut self, ms: i64) {
        if self.recap_start == 0 {
            self.recap_start = self.now;
        }
        if (self.now - self.recap_start) / 1_000_000 >= RECAP_SECS {
            self.print_recap();
        }
        self.graph.push(ms);
        if self.graph.len() > GRAPH_LEN {
            self.graph.remove(0);
        }
        self.recap.push(ms);
        self.recap_loss = self.recap_loss.max(self.info.loss);

        let spike = ms >= self.cfg.max_ping && ms < 1000;
        if spike && !self.in_spike {
            out(format!(
                "  {YELLOW}!{RESET} {} ping spike {}\n",
                fmt_hms(self.now),
                fmt_ms(ms)
            ));
        }
        self.in_spike = spike;
        self.draw_status();
    }

    /// One permanent line with the exact figures of the last period.
    fn print_recap(&mut self) {
        if let Some((min, avg, max)) = stats(&self.recap) {
            let lost = self.recap.iter().filter(|&&ms| ms >= 1000).count();
            let mut s = format!(
                "  {DIM}{}{RESET}  ping avg {} ({min}-{max}), loss {:.1}%",
                fmt_hm(self.recap_start),
                fmt_ms(avg),
                self.recap_loss
            );
            if lost > 0 {
                s.push_str(&format!(", {lost} s without answer"));
            }
            s.push('\n');
            out(s);
        }
        self.recap.clear();
        self.recap_start = self.now;
        self.recap_loss = self.info.loss;
    }

    fn draw_status(&self) {
        if !self.info.active || self.graph.is_empty() {
            return;
        }
        let i = &self.info;
        let last = *self.graph.last().unwrap_or(&0);
        let range = match stats(&self.graph) {
            Some((min, avg, max)) => {
                format!("{DIM}min{RESET} {min} {DIM}avg{RESET} {avg} {DIM}max{RESET} {max}")
            }
            None => String::new(),
        };
        status_line(&format!(
            "  ping {} {}  {range}  loss {:.1}%  {}",
            sparkline(&self.graph),
            fmt_ms(last),
            i.loss,
            fmt_clock((self.now - i.start) / 1_000_000)
        ));
    }

    fn match_end(&mut self) {
        crate::util::keep_status_line();
        if self.recap.len() >= 10 {
            self.print_recap();
        }
        let i = &self.info;
        let elapsed = (self.now - i.start) / 1_000_000;
        self.finished.push(MatchSummary {
            p2p: i.p2p,
            ip: i.ip.clone(),
            port: i.port,
            duration_s: elapsed,
            initial: i.initial,
            samples: i.samples,
            min: i.min,
            avg: i.avg,
            max: i.max,
            max_loss: i.max_loss,
        });
        if elapsed >= 20 {
            let ping = if i.samples > 0 {
                format!("ping avg {} (min {}, max {})", fmt_ms(i.avg), i.min, i.max)
            } else {
                "ping n/a".into()
            };
            out(format!(
                "  Match over - {} - {ping} - worst loss {:.1}%\n",
                fmt_duration(elapsed),
                i.max_loss
            ));
            if elapsed > 90 {
                self.log_match(elapsed);
            }
        } else if i.p2p {
            out(format!(
                "  {DIM}Match ended after {} (declined or cancelled){RESET}\n",
                fmt_duration(elapsed)
            ));
        } else {
            out(format!(
                "  {DIM}Match ended after {} (declined, cancelled, or the game switched to TCP){RESET}\n",
                fmt_duration(elapsed)
            ));
        }
        self.info = MatchInfo {
            print_count: 0,
            ..Default::default()
        };
        self.p2p = P2pStats::default();
        self.psp = PspStats::default();
        self.write_overlay("exit");
        if let Some(o) = &self.overlay {
            o.show(crate::overlay::Status::Waiting);
        }
    }

    fn log_match(&self, elapsed: i64) {
        let Some(path) = &self.cfg.match_log else {
            return;
        };
        let mut text = String::new();
        if !path.exists() {
            text.push_str("MatchTime,IP,Port,Region,MatchMode,Provider,MatchDuration,InitialDelay(ms),MinDelay(ms),AvgDelay(ms),MaxDelay(ms),MaxLossRate\n");
        }
        text.push_str(&self.csv_row(elapsed));
        text.push('\n');
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(text.as_bytes());
        }
    }

    pub fn csv_row(&self, elapsed: i64) -> String {
        let i = &self.info;
        format!(
            "{},{},{},{},{},{},{:02}:{:02}:{:02},{},{},{},{},{:.1}%",
            fmt_full(i.start),
            i.ip,
            i.port,
            csv_field(&i.area),
            if i.p2p { "peer-peer" } else { "client-server" },
            if i.p2p {
                "-".to_string()
            } else {
                crate::cloud::describe(&i.ip)
            },
            elapsed / 3600,
            elapsed / 60 % 60,
            elapsed % 60,
            i.initial,
            i.min,
            i.avg,
            i.max,
            i.max_loss
        )
    }

    // ---- 1 Hz worker, split around the blocking probe

    /// Server to probe this tick (client-server matches only).
    pub fn probe_target(&mut self) -> Option<(String, u16)> {
        if !self.info.active {
            self.probe_ticks = 0;
            return None;
        }
        (!self.info.p2p).then(|| (self.info.ip.clone(), self.info.port))
    }

    pub fn after_probe(&mut self, ms: i64) {
        self.probe_ticks += 1;
        if ms < 1000 {
            self.update_match_info(ms, 0.0);
        }
        self.print_sample(ms);
    }

    /// Ends a match still open (end of a replay).
    pub fn finish(&mut self) {
        if self.info.active {
            self.match_end();
        }
    }

    pub fn tick(&mut self, now: Micros) {
        self.now = self.now.max(now);
        let now = self.now;
        if self.info.active && self.probe_ticks % 2 == 0 {
            self.update_overlay();
        }
        let psp_running = self.psp.active;
        self.speed.tick(now, psp_running);
        if self.psp.active && (now - self.last_psp) / 1_000_000 > 8 {
            self.match_end();
        }
        if self.p2p.session_started && (now - self.last_p2p) / 1_000_000 > 3 {
            self.match_end();
        }
    }
}

/// Min, average and max of the answered samples (no-answer samples are 1000 ms or more).
fn stats(samples: &[i64]) -> Option<(i64, i64, i64)> {
    let ok: Vec<i64> = samples.iter().copied().filter(|&ms| ms < 1000).collect();
    let min = *ok.iter().min()?;
    let max = *ok.iter().max()?;
    Some((min, ok.iter().sum::<i64>() / ok.len() as i64, max))
}

/// Quotes a CSV field when needed (locations like `Paris, France` contain commas).
fn csv_field(s: &str) -> String {
    if s.contains([',', '"']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn is_psp_msg(m: u16) -> bool {
    m & 0xfeff == 0xf088 || m == 0xe660
}

pub fn is_p2p_msg(m: u16) -> bool {
    m == 0 || m & 0xfeff == 0x080b || m & 0xfeff == 0x080c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{eth_ipv4_udp, parse, DLT_EN10MB};

    fn mon(tag: &str) -> Monitor {
        let dir = std::env::temp_dir().join(format!("efm-test-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("matchlogs.csv");
        let _ = std::fs::remove_file(&log);
        Monitor::new(
            Config {
                match_log: Some(log),
                ..Default::default()
            },
            vec!["192.168.1.10".into()],
            GeoDb::default(),
            SpeedServers::load(dir.join("speedtest.txt")),
        )
    }

    fn feed(
        m: &mut Monitor,
        src: [u8; 4],
        dst: [u8; 4],
        sp: u16,
        dp: u16,
        payload: &[u8],
        ts: Micros,
    ) {
        let f = eth_ipv4_udp(src, dst, sp, dp, payload);
        m.on_packet(&parse(DLT_EN10MB, &f).unwrap(), ts);
    }

    fn msg(t: u16, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        v[..2].copy_from_slice(&t.to_be_bytes());
        v
    }

    const ME: [u8; 4] = [192, 168, 1, 10];
    const PEER: [u8; 4] = [81, 2, 3, 4];
    const GCP: [u8; 4] = [34, 84, 43, 65];
    const AWS: [u8; 4] = [15, 188, 178, 40];

    #[test]
    fn message_classes() {
        assert!(is_psp_msg(0xf088) && is_psp_msg(0xf188) && is_psp_msg(0xe660));
        assert!(is_p2p_msg(0) && is_p2p_msg(0x080b) && is_p2p_msg(0x090b) && is_p2p_msg(0x090c));
        assert!(!is_p2p_msg(0xf188) && !is_psp_msg(0x080b));
    }

    #[test]
    fn local_rules() {
        let m = mon("local");
        assert!(m.is_local("192.168.1.10"));
        assert!(m.is_local("10.0.0.5"));
        assert!(m.is_local("172.20.1.1"));
        assert!(!m.is_local("81.2.3.4"));
    }

    #[test]
    fn client_server_latency_and_loss() {
        let mut m = mon("psp");
        feed(&mut m, ME, AWS, 5000, 30123, &msg(0xe660, 20), 1_000_000);
        feed(&mut m, AWS, ME, 30123, 5000, &msg(0xf088, 20), 1_048_000);
        assert!(m.info.active && !m.info.p2p);
        assert_eq!(m.info.ip, "15.188.178.40");
        assert_eq!(m.info.initial, 48);
        assert_eq!(
            crate::cloud::lookup(&m.info.ip).map(|x| x.0),
            Some(crate::cloud::Provider::Aws)
        );
        let mut ts = 2_000_000;
        for g in 1u8..=4 {
            for _ in 0..200 {
                ts += 1000;
                let mut d = msg(0xf188, 20);
                d[9] = g;
                feed(&mut m, AWS, ME, 30123, 5000, &d, ts);
            }
        }
        assert!(
            (m.info.max_loss - 21.875).abs() < 0.01,
            "{}",
            m.info.max_loss
        );
        // > 8 s of silence ends the match
        m.info.start = ts + 9_000_000 - 120_000_000;
        m.tick(ts + 9_000_000);
        assert!(!m.info.active);
        assert_eq!(
            m.finished[0].to_string(),
            "match client-server remote=15.188.178.40:30123 provider=\"AWS eu-west-3\" \
             duration=120s initial=48ms samples=1 min=48ms avg=48ms max=48ms max_loss=21.9%"
        );
        let csv = std::fs::read_to_string(m.cfg.match_log.as_ref().unwrap()).unwrap();
        assert!(
            csv.lines().nth(1).unwrap().contains(
                ",15.188.178.40,30123,\"Paris, France\",client-server,AWS eu-west-3,00:02:00,48,"
            ),
            "{csv}"
        );
    }

    #[test]
    fn recap_every_minute_and_spikes() {
        let mut m = mon("recap");
        m.info.active = true;
        m.info.start = 1_000_000;
        m.now = 1_000_000;
        assert_eq!(stats(&[40, 1000, 20, 30]), Some((20, 30, 40)));
        assert_eq!(stats(&[1000]), None);
        for s in 0..59 {
            m.now = 1_000_000 + s * 1_000_000;
            m.print_sample(if s == 30 { 250 } else { 40 });
            assert_eq!(m.in_spike, s == 30);
        }
        assert_eq!(m.recap.len(), 59);
        m.now += 2_000_000;
        m.print_sample(40);
        assert_eq!(m.recap.len(), 1, "a new period starts after 60 s");
        assert_eq!(m.graph.len(), GRAPH_LEN);
    }

    #[test]
    fn p2p_rtt_by_transaction_id() {
        let mut m = mon("p2p");
        feed(&mut m, ME, PEER, 5000, 6000, &msg(0x080b, 20), 1_000_000);
        assert!(m.info.active && m.info.p2p);
        let mut req = msg(0x080c, 20);
        req[8..20].copy_from_slice(b"ABCDEFGHIJKL");
        let mut rsp = msg(0x090c, 20);
        rsp[8..20].copy_from_slice(b"ABCDEFGHIJKL");
        feed(&mut m, ME, PEER, 5000, 6000, &req, 2_000_000);
        feed(&mut m, PEER, ME, 6000, 5000, &rsp, 2_035_000);
        assert_eq!(m.info.initial, 35);
        rsp[19] = b'X'; // wrong tid ignored
        feed(&mut m, PEER, ME, 6000, 5000, &rsp, 2_090_000);
        assert_eq!(m.info.current, 35);
    }

    #[test]
    fn p2p_session_from_game_data() {
        let mut m = mon("game");
        let mut ts = 1_000_000;
        for g in 0u8..3 {
            for k in 0..200 {
                ts += 1000;
                if k % 5 == 0 {
                    continue; // 20% loss
                }
                let mut d = msg(0, 20);
                d[2] = g;
                feed(&mut m, PEER, ME, 6000, 5000, &d, ts);
            }
        }
        assert!(m.p2p.session_started);
        assert_eq!(m.p2p.major_ip, "81.2.3.4");
        assert!(m.info.max_loss > 30.0, "{}", m.info.max_loss);
        m.tick(ts + 4_000_000);
        assert!(!m.info.active);
    }

    #[test]
    fn speed_test_packets() {
        let mut m = mon("speed");
        feed(&mut m, ME, GCP, 5000, 5521, &[0x12, 0x34], 1_000_000);
        feed(&mut m, GCP, ME, 5521, 5000, &[0x12, 0x34], 1_025_000);
        let s = m
            .speed
            .servers
            .iter()
            .find(|s| s.ip == "34.84.43.65")
            .unwrap();
        assert_eq!((s.requests, s.responses, s.min), (1, 1, 25));
        assert!(!m.info.active);
    }
}
