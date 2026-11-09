---
name: Detection problem
about: A match was not detected, detected wrongly, or the numbers look wrong
labels: detection
---

**What happened**
<!-- e.g. "client-server match on Google Cloud, but the tool showed nothing" -->

**Setup**
- efootball-monitor version (`efootball-monitor --version`):
- eFootball version / date:
- Platform: PC / PlayStation / Xbox (port mirroring) / other
- OS:

**Capture (very helpful)**
1. Run `efootball-monitor --record session.pcap` and play until the problem happens.
2. Run `efootball-monitor anonymize session.pcap session-anon.pcap`.
3. Attach `session-anon.pcap` (zip it if GitHub refuses the extension).

Anonymization replaces your IP address, other players' addresses and MAC addresses, and removes non-game traffic.
The content of game packets is kept as is.

**Output of** `efootball-monitor replay session-anon.pcap --summary`
```
paste here
```
