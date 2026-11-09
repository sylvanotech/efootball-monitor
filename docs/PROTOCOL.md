# How matches are recognized

This document describes the eFootball UDP traffic patterns the tool relies on. They are not documented by Konami and
may change with any game update; contributions with packet captures of new patterns are welcome (record with
`--record`, share with `anonymize`, see `fixtures/README.md`).

All message types below are the **first two bytes of the UDP payload, big endian**. Datagrams sent *from* the
standard STUN ports 3478 and 5478 are ignored. "Local" means one of the host's own addresses or any private
(RFC 1918) address, so that a console seen through port mirroring counts as local.

## Client-server matches

Both players connect to a Konami relay server (Google Cloud or AWS).

| Type | Direction | Meaning |
|---|---|---|
| `0xe660` | local → server | Pairing request. Its time is remembered while no client-server match is running. |
| `0xf088` | server → local | First answer from the server. **Match found.** Initial latency = time since the last `0xe660`. |
| `0xf188` | server → local | Relayed game data. Byte 9 is a group id; each group should contain 256 messages. |

Packet loss is computed when the group id changes: `loss = (256 − messages received in the group) / 256`. Groups
with 10 messages or fewer are ignored. The match is considered over after **8 s** without client-server messages.

During the match the tool measures the server latency once per second with a TCP connection to the server's game port
(`connect()` time; 1000 ms when it fails). Servers using port 5735 do not accept TCP and are measured with an ICMP echo.

## Peer-to-peer matches

The players talk to each other directly after a STUN-like handshake.

| Type | Direction | Meaning |
|---|---|---|
| `0x080b` | local → peer | Match request. **Match found** (peer = destination). |
| `0x090b` | peer → local | Match response. **Match found** (peer = source). |
| `0x080c` | local → peer | Keep-alive request. Bytes 8–19 are a 12-byte transaction id. |
| `0x090c` | peer → local | Keep-alive response with the same transaction id ⇒ round-trip time. |
| `0x0000` | peer → local | Game data. Byte 2 is a group id (256 messages per group). |

After 100 game-data messages from the peer, the session is considered started and the peer's address is fixed (later
`0x080b`/`0x090b` from other addresses are ignored). Packet loss is computed per group as above. The match is over
after **3 s** without peer-to-peer messages.

## In-game speed test

Before matchmaking the game measures its latency to many servers.

* UDP length field = 10 (2-byte payload), on server port **5521** (Google Cloud) or **10000 / 30000 / 50000** (AWS).
* local → server: request; the payload is a 16-bit request id.
* server → local: response echoing the same id.

Requests are matched with responses by (server, id); unanswered requests expire after 2 s. A summary is printed
5 s after the last speed test packet, at most once per test burst. Discovered servers are saved to `speedtest.txt`
(`ip region port`).

## Capture filter

```text
(udp and not port 123 and not port 12345 and not port 53 and not port 12346
 and not port 5353 and not port 8000 and not port 137) and not host 255.255.255.255
```

Supported link types: Ethernet (with 802.1Q/802.1ad VLAN tags), raw IP (e.g. WireGuard), Linux cooked capture
v1/v2.

## Known limitations

* TCP matches (the game's fallback when UDP fails) are not detected.
* Only IPv4 addresses are classified as Google Cloud / AWS.
