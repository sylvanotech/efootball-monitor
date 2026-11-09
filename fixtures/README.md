# Regression captures

Every `<name>.pcap` or `<name>.pcapng` in this folder is replayed by `cargo test`, and the result must match
`<name>.expected` exactly. This is how we make sure that the detection still works on real eFootball traffic after a
code change, and how a detection bug gets a permanent test once it is fixed.

| File | Content |
|---|---|
| `synthetic-session.pcap` | Generated traffic (not a real game): speed test, one client-server match, one peer-to-peer match. Regenerate with `cargo test write_synthetic_fixture -- --ignored`. |
| `real-2026-11-client-server.pcap` | Real PC session: in-game speed test (~100 servers) and one client-server match in Johannesburg with a loss burst. Packets cut to 62 bytes (headers + the start of the game payload, all the detection needs). |

## Adding a real capture

1. Record a session: `efootball-monitor --record session.pcap` (or capture UDP with Wireshark).
2. Anonymize it: `efootball-monitor anonymize session.pcap fixtures/<short-name>.pcap`.
   Keep it small: one or two matches is enough (a few MB at most). Cutting every packet to its first 62 bytes
   (`editcap -s 62` from Wireshark) keeps everything the detection needs and divides the size by about 8.
3. Create the expected result from the replay, then **check by hand that it is correct** (right match mode, server,
   plausible latency and loss):

   ```sh
   efootball-monitor replay fixtures/<short-name>.pcap --summary > fixtures/<short-name>.expected
   ```

4. Add a comment line at the top of the `.expected` file (`# ...`) describing the capture: game version, date,
   platform (PC/console), what happened in the match.

If the tool is wrong on your capture, write the **correct** expected lines by hand instead and open a pull request or an
issue: the failing test documents the bug.

## Expected file format

```text
# comment
match <client-server|peer-peer> remote=<ip>:<port> [provider="<cloud region>"] duration=<s>s initial=<ms>ms samples=<n> min=<ms>ms avg=<ms>ms max=<ms>ms max_loss=<pct>%
speed <server ip>:<port> sent=<n> answered=<n> min=<ms>ms avg=<ms>ms
```

During a replay the active latency probes (TCP connect/ICMP) are not performed, so `samples` only counts latency values
that come from the captured traffic itself.
