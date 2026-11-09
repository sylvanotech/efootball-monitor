//! Time formatting and console helpers.

use chrono::{DateTime, Local, TimeZone};
use std::io::Write;

/// Microseconds since the Unix epoch (pcap timestamps and wall clock share this base).
pub type Micros = i64;

pub fn now_micros() -> Micros {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    d.as_micros() as Micros
}

fn local(ts: Micros) -> DateTime<Local> {
    Local
        .timestamp_opt(
            ts.div_euclid(1_000_000),
            (ts.rem_euclid(1_000_000) * 1000) as u32,
        )
        .single()
        .unwrap_or_else(Local::now)
}

/// TimeString type 1: `%Y/%m/%d %H:%M:%S`, local time.
pub fn fmt_full(ts: Micros) -> String {
    local(ts).format("%Y/%m/%d %H:%M:%S").to_string()
}

/// TimeString type 3: `%H:%M:%S`, local time.
pub fn fmt_hms(ts: Micros) -> String {
    local(ts).format("%H:%M:%S").to_string()
}

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

static QUIET: AtomicBool = AtomicBool::new(false);
static ASCII: AtomicBool = AtomicBool::new(false);
/// A live status line is on screen (no newline yet).
static STATUS: AtomicBool = AtomicBool::new(false);

/// Silence the live console output (used by `replay --summary`).
pub fn set_quiet(q: bool) {
    QUIET.store(q, Relaxed);
}

/// Plain ASCII output for consoles without Unicode block characters.
pub fn set_ascii(a: bool) {
    ASCII.store(a, Relaxed);
}

fn ascii() -> bool {
    ASCII.load(Relaxed)
}

const CLEAR_LINE: &str = "\r\x1b[2K";

/// print + flush. Clears the live status line first, if any.
pub fn out(s: impl AsRef<str>) {
    if QUIET.load(Relaxed) {
        return;
    }
    let mut o = std::io::stdout().lock();
    if STATUS.swap(false, Relaxed) {
        let _ = o.write_all(CLEAR_LINE.as_bytes());
    }
    let _ = o.write_all(s.as_ref().as_bytes());
    let _ = o.flush();
}

/// Redraw the single live status line (no newline).
pub fn status_line(s: &str) {
    if QUIET.load(Relaxed) {
        return;
    }
    let mut o = std::io::stdout().lock();
    let _ = write!(o, "{CLEAR_LINE}{s}");
    let _ = o.flush();
    STATUS.store(true, Relaxed);
}

/// Keep the current status line on screen (ends it with a newline).
pub fn keep_status_line() {
    if STATUS.swap(false, Relaxed) && !QUIET.load(Relaxed) {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(b"\n");
        let _ = o.flush();
    }
}

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const GREEN: &str = "\x1b[32;1m";
pub const YELLOW: &str = "\x1b[33;1m";
pub const ORANGE: &str = "\x1b[38;5;208m";
pub const RED: &str = "\x1b[31;1m";

/// Latency level 0..=7: <30 <50 <75 <100 <150 <200 <400, else 7.
pub fn level(ms: i64) -> usize {
    [30, 50, 75, 100, 150, 200, 400]
        .iter()
        .position(|&t| ms < t)
        .unwrap_or(7)
}

pub fn latency_color(ms: i64) -> &'static str {
    match level(ms) {
        0 | 1 => GREEN,
        2 | 3 => YELLOW,
        4 | 5 => ORANGE,
        _ => RED,
    }
}

/// `41 ms` colored by latency, or `no answer`.
pub fn fmt_ms(ms: i64) -> String {
    if ms >= 1000 {
        format!("{RED}no answer{RESET}")
    } else {
        format!("{}{ms} ms{RESET}", latency_color(ms))
    }
}

/// One colored bar per sample, taller = slower.
pub fn sparkline(samples: &[i64]) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const PLAIN: [char; 8] = ['_', '.', ',', '-', '=', '+', '*', '#'];
    let set = if ascii() { &PLAIN } else { &BLOCKS };
    let mut s = String::new();
    let mut color = "";
    for &ms in samples {
        let c = latency_color(ms);
        if c != color {
            s.push_str(c);
            color = c;
        }
        s.push(set[level(ms)]);
    }
    s.push_str(RESET);
    s
}

/// `── title ─────…` (60 columns).
pub fn rule(title: &str) -> String {
    let c = if ascii() { "-" } else { "─" };
    let head = format!("{c}{c} {title} ");
    let n = 60usize.saturating_sub(head.chars().count());
    format!("{DIM}{head}{}{RESET}", c.repeat(n))
}

/// Accept/decline hint from the latency at match start.
pub fn verdict(ms: i64, max_ping: i64) -> String {
    if ms <= 0 {
        return format!("{DIM}measuring...{RESET}");
    }
    if ms >= max_ping {
        format!("{RED}FAR{RESET} - consider declining")
    } else if ms < max_ping.min(60) {
        format!("{GREEN}GOOD{RESET}")
    } else {
        format!("{YELLOW}OK{RESET}")
    }
}

/// `9 min 28 s`, `45 s`.
pub fn fmt_duration(s: i64) -> String {
    if s >= 60 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

/// `mm:ss`.
pub fn fmt_clock(s: i64) -> String {
    format!("{:02}:{:02}", s / 60, s % 60)
}

pub const HELP: &str = "\
Commands (type, then Enter):
  speed     in-game speed test results for every Konami server
  servers   speed test servers found so far
  help      this help

Ping graph: one bar per second, taller = slower. Green < 50 ms, yellow < 100 ms,
orange < 200 ms, red above (or no answer).
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_verdicts() {
        assert_eq!(level(10), 0);
        assert_eq!(level(99), 3);
        assert_eq!(level(5000), 7);
        assert!(verdict(30, 120).contains("GOOD"));
        assert!(verdict(90, 120).contains("OK"));
        assert!(verdict(180, 120).contains("FAR"));
        assert_eq!(fmt_duration(568), "9 min 28 s");
        assert_eq!(fmt_duration(45), "45 s");
    }
}
