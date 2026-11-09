# Changelog

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
