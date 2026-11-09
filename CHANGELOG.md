# Changelog

## Unreleased

* New terminal display: one block per match with a GOOD / OK / FAR hint (`--max-ping`), a live ping graph on a
  single line, readable loss alerts and end-of-match summary, speed test table.
* Exact figures next to the graph: min / avg / max over the last 30 s, a recap line every minute, and a
  `ping spike` alert when the ping goes over `--max-ping`.
* From the first real capture: quieter speed test (no line per discovered server, 10 best servers in the automatic
  summary, at most every 2 minutes, provider shown to tell servers in the same city apart); the initial latency is
  the first handshake; matches under 60 s are reported as not played; `asia-southeast3` (Bangkok) added.
* IP addresses hidden by default (`--show-ip` to show them).
* Commands renamed: `speed`, `servers`, `help` (the old `sp` and `hh` still work).
* `--ascii` for consoles without Unicode block characters.
* Servers are shown by city and country (`Johannesburg, South Africa`) instead of the cloud region code, which is
  also more reliable than IP geolocation for cloud addresses. The `Region` column of `matchlogs.csv` changes accordingly.

## 0.1.0-beta.1 — 2026-11-09

First public version (beta: not yet validated on a large number of real matches).

* Detects client-server matches (Google Cloud and AWS relays) and peer-to-peer matches.
* Server latency probe (TCP connect, ICMP for port 5735), packet loss per message group.
* In-game speed test results for every Konami server, saved to `speedtest.txt`.
* Match history in `matchlogs.csv`.
* `--overlay` (Windows): small always-on-top window showing the server/opponent location and latency of the
  current match, to decide whether to accept it.
* Cloud region of servers from the providers' published IP ranges; optional IP2Location LITE geolocation.
* Windows (Npcap), Linux and macOS (libpcap).
* `--record` saves the game traffic to a pcap file; `replay` plays a pcap/pcapng capture back (no Npcap needed);
  `anonymize` makes a capture safe to share.
* Captures in `fixtures/` are replayed by `cargo test` as regression tests.
