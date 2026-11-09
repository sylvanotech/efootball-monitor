//! Time formatting and console helpers.

use chrono::{DateTime, Local, TimeZone, Utc};
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

/// TimeString type 2: `%Y/%m/%d %H:%M:%S`, UTC.
pub fn fmt_full_utc(ts: Micros) -> String {
    Utc.timestamp_micros(ts)
        .single()
        .unwrap_or_else(Utc::now)
        .format("%Y/%m/%d %H:%M:%S")
        .to_string()
}

/// TimeString type 3: `%H:%M:%S`, local time.
pub fn fmt_hms(ts: Micros) -> String {
    local(ts).format("%H:%M:%S").to_string()
}

static QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Silence the live console output (used by `replay --summary`).
pub fn set_quiet(q: bool) {
    QUIET.store(q, std::sync::atomic::Ordering::Relaxed);
}

/// print + flush, so dots appear immediately.
pub fn out(s: impl AsRef<str>) {
    if QUIET.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(s.as_ref().as_bytes());
    let _ = o.flush();
}

pub const RED_X: &str = "\x1b[31mX\x1b[0m";

/// Latency dot: <15 <30 <50 <75 <100 <125 <150 <200 <250 <400, else X.
pub fn color_dot(delay: i64) -> &'static str {
    match delay {
        d if d < 15 => "\x1b[40;37;0m.\x1b[0m",
        d if d < 30 => "\x1b[40;32;1m.\x1b[0m",
        d if d < 50 => "\x1b[40;33;1m.\x1b[0m",
        d if d < 75 => "\x1b[40;34;1m.\x1b[0m",
        d if d < 100 => "\x1b[40;35;1m.\x1b[0m",
        d if d < 125 => "\x1b[40;36;1mo\x1b[0m",
        d if d < 150 => "\x1b[40;34;1mO\x1b[0m",
        d if d < 200 => "\x1b[40;35;1mP\x1b[0m",
        d if d < 250 => "\x1b[40;31;1mQ\x1b[0m",
        d if d < 400 => "\x1b[31mR\x1b[0m",
        _ => RED_X,
    }
}

pub const YELLOW: &str = "\x1b[40;33;1m";
pub const RESET: &str = "\x1b[0m";

/// Legend printed before the first match.
pub fn brief() -> String {
    let mut s = String::from(
        "\n\nbrief: Client-Server is a match mode that users connect to a game server\n\
         \tThe initial yellow values represent the true server processing latency\n\
         \tThe subsequent value and dots are derived from ICMP pings and may not reflect the actual gaming experience.\n\
         \tpeer-peer is a match that opponents connect to one another directly.\n\
         Colored dots represent varying levels of latency:<15ms, 30, 50, 75, 100, 125, 150, 200, 250, 400, and failed (denoted as:",
    );
    for d in [10, 20, 40, 60, 90, 110, 140, 180, 220, 300, 500] {
        s.push_str(color_dot(d));
    }
    s.push_str(")\n");
    s
}

pub const HELP: &str = "\
Commands (type then Enter):
  hh  list the known speed test servers
  sp  detailed in-game speed test results
";
