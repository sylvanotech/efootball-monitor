//! efootball-monitor: watches eFootball's UDP traffic to show the match mode
//! (client-server or peer-to-peer), where the server/opponent is, latency,
//! packet loss and the in-game speed test results.
//!
//! Threads: capture loop (main thread), a 1 Hz worker (server latency probe,
//! speed test report, end-of-match detection) and a keyboard command reader.

mod capture_file;
mod cloud;
mod cloud_ranges;
mod geo;
mod monitor;
mod overlay;
mod packet;
mod probe;
mod replay;
mod speed;
mod util;

use std::io::BufRead;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use monitor::{Config, Monitor};
use util::{now_micros, out};

/// Capture filter: UDP only, without common LAN noise (NTP, DNS, mDNS, NetBIOS, broadcast).
const BPF_FILTER: &str = "(udp and not port 123 and not port 12345 and not port 53 and not port 12346 \
                          and not port 5353 and not port 8000 and not port 137) and not host 255.255.255.255";

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Shows eFootball match mode (client-server or peer-to-peer), server/opponent location, latency, packet loss and in-game speed test results",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    live: LiveArgs,
    #[command(flatten)]
    display: DisplayArgs,
}

#[derive(Args, Debug)]
struct DisplayArgs {
    /// Flag matches at or above this ping (ms) as far
    #[arg(long, global = true, default_value_t = 120)]
    max_ping: i64,
    /// Show IP addresses and ports (hidden by default, e.g. when streaming)
    #[arg(long, global = true)]
    show_ip: bool,
    /// Plain ASCII output, for old consoles that show the ping graph as boxes
    #[arg(long, global = true)]
    ascii: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Replay a capture file (.pcap or .pcapng) as if it were live traffic
    Replay {
        /// Capture file, e.g. made with --record or Wireshark
        file: PathBuf,
        /// Address of the gaming device in the capture (default: the most frequent one)
        #[arg(long = "local-ip")]
        local_ips: Vec<String>,
        /// Respect the original timing instead of replaying as fast as possible
        #[arg(long)]
        realtime: bool,
        /// Show the overlay window while replaying (Windows, best with --realtime)
        #[arg(long)]
        overlay: bool,
        /// Only print the final summary (stable format used by regression tests)
        #[arg(long)]
        summary: bool,
        /// Folder containing the IP2Location .BIN files
        #[arg(long)]
        geo_dir: Option<PathBuf>,
    },
    /// Make a capture safe to share: replace personal IP addresses, blank MAC
    /// addresses, drop non-game traffic. Konami server addresses are kept.
    Anonymize {
        /// Capture to read (.pcap or .pcapng)
        input: PathBuf,
        /// Anonymized .pcap to write
        output: PathBuf,
    },
}

#[derive(Args, Debug)]
struct LiveArgs {
    /// Interface number to use (1-based, as listed at startup)
    adapter: Option<usize>,
    /// Folder containing IP2LOCATION-LITE-DB3.BIN / IP2LOCATION-LITE-DB3.IPV6.BIN
    #[arg(long)]
    geo_dir: Option<PathBuf>,
    /// Show the current match in a small always-on-top window (Windows)
    #[arg(long)]
    overlay: bool,
    /// Write live status to this text file (OBS text source, custom overlays)
    #[arg(long)]
    overlay_file: Option<PathBuf>,
    /// Match log CSV path (matches longer than 90 s)
    #[arg(long, default_value = "matchlogs.csv")]
    match_log: PathBuf,
    /// Speed test server list (`ip region port` per line), updated automatically
    #[arg(long, default_value = "speedtest.txt")]
    speed_servers: PathBuf,
    /// Also save the captured game traffic to this .pcap file (see `replay` and `anonymize`)
    #[arg(long)]
    record: Option<PathBuf>,
    /// List interfaces and exit
    #[arg(long)]
    list: bool,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

fn find_geo_dir(arg: Option<PathBuf>) -> Option<PathBuf> {
    if arg.is_some() {
        return arg;
    }
    [
        exe_dir(),
        exe_dir().join("iparea"),
        PathBuf::from("."),
        PathBuf::from("iparea"),
    ]
    .into_iter()
    .find(|d| {
        d.join("IP2LOCATION-LITE-DB3.BIN").exists()
            || d.join("IP2LOCATION-LITE-DB3.IPV6.BIN").exists()
    })
}

/// Interfaces with an address (IPv4 or IPv6, not link-local) and an Ethernet/RAW link type.
fn capture_interfaces() -> Result<Vec<(pcap::Device, i32)>> {
    let devs =
        pcap::Device::list().context("pcap_findalldevs failed (is Npcap/libpcap installed?)")?;
    let mut res = Vec::new();
    for d in devs {
        let usable = d.addresses.iter().any(|a| match a.addr {
            IpAddr::V4(v) => !v.is_link_local() && !v.is_loopback(),
            IpAddr::V6(v) => !v.is_loopback() && (v.segments()[0] & 0xffc0) != 0xfe80,
        });
        if !usable {
            continue;
        }
        let Ok(cap) = pcap::Capture::from_device(d.clone()).and_then(|c| c.open()) else {
            continue;
        };
        let lt = cap.get_datalink().0;
        if packet::supported_linktype(lt) {
            res.push((d, lt));
        }
    }
    Ok(res)
}

#[cfg(windows)]
mod npcap {
    //! Npcap installs wpcap.dll in %SystemRoot%\System32\Npcap, which is not on the
    //! DLL search path. wpcap.dll is delay-loaded (see build.rs), so add that
    //! directory before the first pcap call, as recommended by the Npcap docs.
    use anyhow::{bail, Result};
    use std::path::PathBuf;

    extern "system" {
        fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
        fn SetDllDirectoryW(path: *const u16) -> i32;
    }

    pub fn prepare() -> Result<()> {
        let mut buf = [0u16; 512];
        // SAFETY: the buffer is valid for `buf.len()` u16 values.
        let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 || n >= buf.len() {
            bail!("GetSystemDirectoryW failed");
        }
        let system = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
        let npcap = system.join("Npcap");
        if npcap.join("wpcap.dll").exists() {
            let mut w: Vec<u16> = npcap.to_string_lossy().encode_utf16().collect();
            w.push(0);
            // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call.
            unsafe { SetDllDirectoryW(w.as_ptr()) };
            Ok(())
        } else if system.join("wpcap.dll").exists() {
            Ok(())
        } else {
            bail!("Npcap is not installed. Download and install it from https://npcap.com/#download, then run this program again.")
        }
    }
}

fn open_geo(dir: Option<PathBuf>) -> geo::GeoDb {
    let geo = find_geo_dir(dir)
        .as_deref()
        .map(geo::GeoDb::open)
        .unwrap_or_default();
    if !geo.is_loaded() {
        out("IP2Location databases not found (use --geo-dir); locations will use cloud regions or 'unknown'\n");
    }
    geo
}

fn main() -> Result<()> {
    #[cfg(windows)]
    let _ = enable_ansi_support::enable_ansi_support();

    let cli = Cli::parse();
    util::set_ascii(cli.display.ascii);
    match cli.command {
        None => run_live(cli.live, &cli.display),
        Some(Command::Replay {
            file,
            local_ips,
            realtime,
            overlay,
            summary,
            geo_dir,
        }) => {
            util::set_quiet(summary);
            let opts = replay::ReplayOptions {
                local_ips,
                realtime,
                geo: open_geo(geo_dir),
                max_ping: cli.display.max_ping,
                show_ip: cli.display.show_ip,
                overlay: if overlay {
                    overlay::Overlay::start()
                } else {
                    None
                },
            };
            let result = replay::replay(&file, opts)?;
            util::set_quiet(false);
            if !summary {
                out(format!(
                    "\n\n=== Replay summary ({} UDP packets) ===\n",
                    result.packets
                ));
            }
            for line in result.lines() {
                println!("{line}");
            }
            Ok(())
        }
        Some(Command::Anonymize { input, output }) => {
            let (kept, dropped) = replay::anonymize(&input, &output)?;
            out(format!(
                "Wrote {}: {kept} packets kept, {dropped} dropped.\n\
                 Server addresses are kept; your address, other players' addresses and MAC addresses were replaced.\n\
                 The game payload is unchanged: only share captures you are comfortable sharing.\n",
                output.display()
            ));
            Ok(())
        }
    }
}

fn run_live(args: LiveArgs, display: &DisplayArgs) -> Result<()> {
    out(format!("efootball-monitor {}\n", env!("CARGO_PKG_VERSION")));
    #[cfg(windows)]
    npcap::prepare()?;

    let ifaces = capture_interfaces()?;
    if ifaces.is_empty() {
        bail!("No network interfaces found (run as Administrator/root, install Npcap)");
    }
    for (i, (d, _)) in ifaces.iter().enumerate() {
        let ips: Vec<String> = d.addresses.iter().map(|a| a.addr.to_string()).collect();
        out(format!(
            " {}: {}  [{}]\n",
            i + 1,
            d.desc.as_deref().unwrap_or(&d.name),
            ips.join(", ")
        ));
    }
    if args.list {
        return Ok(());
    }
    let n = match args.adapter {
        Some(n) if n >= 1 && n <= ifaces.len() => n,
        Some(_) => 1, // out of range: fall back to the first interface
        None if ifaces.len() == 1 => 1,
        None => {
            out(format!("Select interface (1-{}): ", ifaces.len()));
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            match line.trim().parse::<usize>() {
                Ok(n) if n >= 1 && n <= ifaces.len() => n,
                _ => bail!("Exiting..."),
            }
        }
    };
    let (dev, linktype) = ifaces[n - 1].clone();

    // Local addresses of every interface.
    let local_ips: Vec<String> = pcap::Device::list()
        .unwrap_or_default()
        .iter()
        .flat_map(|d| d.addresses.iter().map(|a| a.addr.to_string()))
        .collect();
    out(format!(
        "IP addresses: {}\n",
        dev.addresses
            .iter()
            .map(|a| a.addr.to_string())
            .collect::<Vec<_>>()
            .join("  ")
    ));

    let mut cap = pcap::Capture::from_device(dev.clone())?
        .promisc(true)
        .snaplen(65536)
        .timeout(1000)
        .open()
        .context("Couldn't open device")?;
    cap.filter(BPF_FILTER, true)
        .context("Couldn't install filter")?;
    out(format!(
        "\x1b[40;32;1mCapture started on {} ...\x1b[0m\n",
        dev.desc.as_deref().unwrap_or(&dev.name)
    ));

    let geo = open_geo(args.geo_dir.clone());
    let mut recorder = match &args.record {
        Some(p) => {
            let w = capture_file::Writer::create(p, linktype)
                .with_context(|| format!("cannot create {}", p.display()))?;
            out(format!("Recording game traffic to {}\n", p.display()));
            Some(w)
        }
        None => None,
    };
    let speed = speed::SpeedServers::load(args.speed_servers.clone());
    out(util::HELP);

    let cfg = Config {
        overlay_file: args.overlay_file.clone(),
        match_log: Some(args.match_log.clone()),
        max_ping: display.max_ping,
        show_ip: display.show_ip,
    };
    let mut mon = Monitor::new(cfg, local_ips, geo, speed);
    if args.overlay {
        mon.overlay = overlay::Overlay::start();
    }
    let mon = Arc::new(Mutex::new(mon));

    // Idle/probe worker, ~1 Hz: server latency during client-server matches,
    // speed test report, end-of-match detection.
    {
        let mon = mon.clone();
        thread::spawn(move || loop {
            let target = mon.lock().unwrap().probe_target();
            let mut spent = 0;
            if let Some((ip, port)) = target {
                let ms = if port == monitor::PORT_ICMP_ONLY {
                    probe::icmp_ping(&ip)
                } else {
                    probe::tcping(&ip, port)
                };
                let ms = ms.min(1000);
                mon.lock().unwrap().after_probe(ms);
                spent = ms as u64;
            }
            thread::sleep(Duration::from_millis(1000u64.saturating_sub(spent)));
            mon.lock().unwrap().tick(now_micros());
        });
    }

    // Keyboard commands (line based): hh, sp.
    {
        let mon = mon.clone();
        thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                let m = mon.lock().unwrap();
                match line.trim().to_ascii_lowercase().as_str() {
                    "servers" | "hh" => m.speed.print_list(),
                    "speed" | "sp" => m.speed.print_detail(&m.geo),
                    "help" | "h" | "?" => out(util::HELP),
                    "" => {}
                    other => out(format!("Unknown command '{other}'.\n{}", util::HELP)),
                }
            }
        });
    }

    // Capture loop. The recording is flushed at least once per second so that
    // closing the window loses at most the last second.
    let mut last_flush = now_micros();
    loop {
        match cap.next_packet() {
            Ok(p) => {
                #[allow(clippy::unnecessary_cast)] // tv_sec/tv_usec are i32 on Windows
                let ts = p.header.ts.tv_sec as i64 * 1_000_000 + p.header.ts.tv_usec as i64;
                if let Some(w) = recorder.as_mut() {
                    w.write(ts, p.data).context("cannot write the recording")?;
                }
                if let Some(udp) = packet::parse(linktype, p.data) {
                    mon.lock().unwrap().on_packet(&udp, ts);
                }
            }
            Err(pcap::Error::TimeoutExpired) => {}
            Err(e) => bail!("capture error: {e}"),
        }
        if let Some(w) = recorder.as_mut() {
            let now = now_micros();
            if now - last_flush >= 1_000_000 {
                w.flush().context("cannot write the recording")?;
                last_flush = now;
            }
        }
    }
}
