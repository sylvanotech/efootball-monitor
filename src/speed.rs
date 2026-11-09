//! In-game speed test tracking.
//!
//! Before matchmaking, the game pings Konami's servers (Google Cloud on UDP
//! 5521, AWS on UDP 10000/30000/50000) with 2-byte datagrams; the 16-bit value
//! is a request id echoed back by the server. Servers are discovered
//! automatically and remembered in `speedtest.txt`.

use std::fs;
use std::path::PathBuf;

use crate::geo::GeoDb;
use crate::util::{fmt_hms, latency_color, out, rule, Micros, DIM, RESET};

pub const SPEED_PORTS: [u16; 4] = [5521, 10000, 30000, 50000];

pub fn is_speed_port(p: u16) -> bool {
    SPEED_PORTS.contains(&p)
}

const MAX_PENDING: usize = 2000;

/// City and country for cloud servers (also for lists saved by older versions), else the saved name.
/// Includes the provider, as several servers can share a city (`Cape Town (AWS)`).
fn display_name(sv: &SpeedServer) -> String {
    use crate::cloud::{lookup, region_location, Provider};
    match lookup(&sv.ip) {
        Some((p, region)) => {
            let provider = match p {
                Provider::GoogleCloud => "Google",
                Provider::Aws => "AWS",
            };
            let place = match region {
                "GLOBAL" => "Anycast",
                r => region_location(r).unwrap_or(r),
            };
            format!("{place} ({provider})")
        }
        None => sv.region.replace('_', " "),
    }
}

/// Servers listed in the automatic summary (all of them with `speed`).
const BRIEF_ROWS: usize = 10;
/// Minimum time between two automatic summaries.
const BRIEF_COOLDOWN_S: i64 = 120;
const PENDING_TTL_MS: i64 = 2000;

#[derive(Clone, Debug, Default)]
pub struct SpeedServer {
    pub ip: String,
    pub region: String,
    pub port: u16,
    pub requests: u32,
    pub responses: u32,
    pub min: i64,
    pub max: i64,
    pub sum: i64,
    /// Totals since start (not reset after each printed burst).
    pub total_requests: u32,
    pub total_responses: u32,
    pub total_sum: i64,
    pub best: i64,
}

impl SpeedServer {
    fn new(ip: &str, region: &str, port: u16) -> Self {
        Self {
            ip: ip.into(),
            region: region.into(),
            port,
            min: -1,
            best: -1,
            ..Default::default()
        }
    }
    fn reset(&mut self) {
        self.requests = 0;
        self.responses = 0;
        self.min = -1;
        self.max = 0;
        self.sum = 0;
    }
    pub fn avg(&self) -> f64 {
        if self.responses > 0 {
            self.sum as f64 / self.responses as f64
        } else {
            0.0
        }
    }
}

struct Pending {
    server: usize,
    id: u16,
    ts: Micros,
}

pub struct SpeedServers {
    pub servers: Vec<SpeedServer>,
    path: PathBuf,
    pending: Vec<Pending>,
    response_count: u32,
    pub should_print: bool,
    pub last_packet: Micros,
    pub last_print: Micros,
    /// Save discovered servers to `path` (false for replays).
    pub persist: bool,
}

impl SpeedServers {
    /// Loads `speedtest.txt` (`<ip> <region> [port]` per line, port defaults to 5521).
    /// A missing file just means servers will be discovered during the next speed test.
    pub fn load(path: PathBuf) -> Self {
        let mut servers = Vec::new();
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines().take(100) {
                let mut it = line.trim_start_matches('\u{feff}').split_whitespace();
                let Some(ip) = it.next() else { continue };
                let region = it.next().unwrap_or("");
                let port = it.next().and_then(|p| p.parse().ok()).unwrap_or(5521);
                servers.push(SpeedServer::new(ip, region, port));
            }
        }
        Self {
            servers,
            path,
            pending: Vec::new(),
            response_count: 0,
            should_print: false,
            last_packet: 0,
            last_print: 0,
            persist: true,
        }
    }

    /// Empty list that is never saved (replays).
    pub fn in_memory() -> Self {
        let mut s = Self::load(PathBuf::new());
        s.persist = false;
        s
    }

    fn find(&self, ip: &str) -> Option<usize> {
        self.servers.iter().position(|s| s.ip == ip)
    }

    /// Local host → server.
    pub fn on_request(&mut self, server_ip: &str, port: u16, id: u16, ts: Micros, geo: &GeoDb) {
        self.last_packet = ts;
        let idx = match self.find(server_ip) {
            Some(i) => i,
            None => {
                if self.servers.len() >= 100 {
                    return;
                }
                self.servers
                    .push(SpeedServer::new(server_ip, &geo.city(server_ip), port));
                self.should_print = true;
                self.servers.len() - 1
            }
        };
        if self.pending.len() >= MAX_PENDING {
            out("  ! speed test: too many pending requests, some are ignored\n");
            return;
        }
        self.servers[idx].requests += 1;
        self.servers[idx].total_requests += 1;
        self.pending.push(Pending {
            server: idx,
            id,
            ts,
        });
    }

    /// Server → local host.
    pub fn on_response(&mut self, server_ip: &str, id: u16, ts: Micros) {
        self.last_packet = ts;
        let Some(idx) = self.find(server_ip) else {
            return;
        };
        let Some(pos) = self
            .pending
            .iter()
            .position(|p| p.server == idx && p.id == id)
        else {
            return;
        };
        let p = self.pending.swap_remove(pos);
        let ms = (ts - p.ts) / 1000;
        let s = &mut self.servers[idx];
        s.responses += 1;
        if s.min == -1 || ms < s.min {
            s.min = ms;
        }
        s.max = s.max.max(ms);
        s.sum += ms;
        s.total_responses += 1;
        s.total_sum += ms;
        if s.best == -1 || ms < s.best {
            s.best = ms;
        }
        self.response_count += 1;
        if !self.should_print
            && (self.response_count >= self.servers.len() as u32 * 200
                || self
                    .servers
                    .iter()
                    .any(|s| s.responses >= 200 && s.responses % 200 == 0))
        {
            self.should_print = true;
            self.response_count = 0;
        }
        self.pending
            .retain(|p| (ts - p.ts) / 1000 <= PENDING_TTL_MS);
    }

    fn sorted(&self) -> Vec<&SpeedServer> {
        let mut v: Vec<&SpeedServer> = self.servers.iter().collect();
        // No-response servers last, otherwise by average latency.
        v.sort_by(|a, b| {
            (a.responses == 0, a.avg())
                .partial_cmp(&(b.responses == 0, b.avg()))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    }

    /// Short table printed automatically after a test burst.
    pub fn print_brief(&self, now: Micros) {
        let mut s = format!("\n{}\n", rule(&format!("Speed test - {}", fmt_hms(now))));
        let answered: Vec<&SpeedServer> = self
            .sorted()
            .into_iter()
            .filter(|s| s.responses > 0)
            .collect();
        let width = answered
            .iter()
            .take(BRIEF_ROWS)
            .map(|s| display_name(s).chars().count())
            .max()
            .unwrap_or(0);
        for sv in answered.iter().take(BRIEF_ROWS) {
            let avg = sv.avg().round() as i64;
            let lost = sv.requests.saturating_sub(sv.responses);
            s.push_str(&format!(
                "  {:<width$} {}{:>4} ms{RESET}  {DIM}min {:>3}{RESET}  {}/{}",
                display_name(sv),
                latency_color(avg),
                avg,
                sv.min,
                sv.responses,
                sv.requests
            ));
            if lost > 0 {
                s.push_str(&format!(
                    "  {}% lost",
                    (lost as f64 * 100.0 / sv.requests as f64).round()
                ));
            }
            s.push('\n');
        }
        let more = answered.len().saturating_sub(BRIEF_ROWS);
        if more > 0 {
            s.push_str(&format!(
                "  {DIM}... and {more} more servers. Type 'speed' for the full list.{RESET}\n"
            ));
        } else {
            s.push_str(&format!("  {DIM}Type 'speed' for details.{RESET}\n"));
        }
        out(s);
    }

    /// Detailed table (`speed` command).
    pub fn print_detail(&self, geo: &GeoDb) {
        let mut s = format!(
            "\n  {:<44} {:>5} {:>5} {:>5} {:>6}  {}\n",
            "Server", "min", "avg", "max", "loss", "answered"
        );
        for sv in self.sorted() {
            let loss = if sv.requests > 0 {
                (1.0 - sv.responses as f64 / sv.requests as f64) * 100.0
            } else {
                100.0
            };
            let mut name = geo.area(&sv.ip);
            let cloud = crate::cloud::describe(&sv.ip);
            if crate::cloud::lookup(&sv.ip).is_some() && cloud != name {
                name = format!("{name} ({cloud})");
            }
            let name: String = name.chars().take(44).collect();
            if sv.responses > 0 {
                let avg = sv.avg().round() as i64;
                s.push_str(&format!(
                    "  {:<44} {:>5} {}{:>5}{RESET} {:>5} {:>5.0}%  {}/{}\n",
                    name,
                    sv.min,
                    latency_color(avg),
                    avg,
                    sv.max,
                    loss,
                    sv.responses,
                    sv.requests
                ));
            } else {
                s.push_str(&format!(
                    "  {DIM}{:<44} {:>5} {:>5} {:>5} {:>5.0}%  {}/{} no answer{RESET}\n",
                    name, "-", "-", "-", loss, sv.responses, sv.requests
                ));
            }
        }
        if self.servers.is_empty() {
            s.push_str("  No speed test seen yet. It runs when you start searching for a match.\n");
        }
        out(s);
    }

    /// `servers` command.
    pub fn print_list(&self) {
        let mut s = String::from("\n");
        for sv in &self.servers {
            s.push_str(&format!(
                "  {:<40} {}\n",
                format!("{}:{}", sv.ip, sv.port),
                display_name(sv)
            ));
        }
        if self.servers.is_empty() {
            s.push_str("  No speed test server known yet.\n");
        }
        out(s);
    }

    pub fn save(&self) {
        if !self.persist {
            return;
        }
        let mut text = String::new();
        for sv in self.sorted() {
            text.push_str(&format!("{} {} {}\n", sv.ip, sv.region, sv.port));
        }
        let _ = fs::write(&self.path, text);
    }

    /// Stable per-server totals, sorted by address (replays and regression tests).
    pub fn summary(&self) -> Vec<String> {
        let mut v: Vec<&SpeedServer> = self
            .servers
            .iter()
            .filter(|s| s.total_requests > 0)
            .collect();
        v.sort_by(|a, b| a.ip.cmp(&b.ip));
        v.iter()
            .map(|s| {
                let avg = if s.total_responses > 0 {
                    s.total_sum / s.total_responses as i64
                } else {
                    -1
                };
                format!(
                    "speed {}:{} sent={} answered={} min={}ms avg={}ms",
                    s.ip, s.port, s.total_requests, s.total_responses, s.best, avg
                )
            })
            .collect()
    }

    pub fn reset(&mut self) {
        self.servers.iter_mut().for_each(SpeedServer::reset);
        self.pending.clear();
    }

    /// Periodic check (1 Hz): print the results once a test burst is over.
    pub fn tick(&mut self, now: Micros, server_match_running: bool) {
        // Print at least once every 75 minutes, and right away for the first burst.
        if self.last_print == 0 || (now - self.last_print) / 1_000_000 > 75 * 60 {
            self.should_print = true;
        }
        let quiet_s = (now - self.last_packet) / 1_000_000;
        let requests: u32 = self.servers.iter().map(|s| s.requests).sum();
        let responses: u32 = self.servers.iter().map(|s| s.responses).sum();
        if quiet_s > 5
            && self.last_packet != 0
            && !server_match_running
            && self.should_print
            && requests > 20
            && (self.last_print == 0 || (now - self.last_print) / 1_000_000 >= BRIEF_COOLDOWN_S)
        {
            self.print_brief(now);
            if responses > 10 {
                self.save();
            }
            self.should_print = false;
            self.reset();
            self.last_print = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_servers_and_matches_ids() {
        let mut s = SpeedServers::load(PathBuf::from("/nonexistent/speedtest.txt"));
        assert!(s.servers.is_empty());
        let geo = GeoDb::default();
        s.on_request("34.92.192.14", 5521, 7, 1_000_000, &geo);
        assert_eq!(s.servers[0].region, "Hong_Kong");
        s.on_request("34.92.192.14", 5521, 8, 1_001_000, &geo);
        s.on_response("34.92.192.14", 8, 1_031_000);
        s.on_response("34.92.192.14", 7, 1_042_000);
        let hk = &s.servers[0];
        assert_eq!((hk.requests, hk.responses, hk.min, hk.max), (2, 2, 30, 42));
        s.on_request("1.2.3.4", 50000, 1, 0, &geo);
        assert_eq!(s.servers.last().unwrap().port, 50000);
        assert!(s.should_print);
    }

    #[test]
    fn parses_file_with_and_without_port() {
        let p = std::env::temp_dir().join(format!("efm-speed-{}.txt", std::process::id()));
        fs::write(
            &p,
            "\u{feff}34.92.192.14 Hong_Kong 5521\n15.188.178.40 Paris 50000\n9.9.9.9 Foo\n",
        )
        .unwrap();
        let s = SpeedServers::load(p.clone());
        let _ = fs::remove_file(p);
        assert_eq!(s.servers.len(), 3);
        assert_eq!(s.servers[0].ip, "34.92.192.14");
        assert_eq!(s.servers[1].port, 50000);
        assert_eq!(s.servers[2].port, 5521);
    }
}
