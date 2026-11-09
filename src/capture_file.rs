//! Minimal reader/writer for capture files, independent of libpcap/Npcap so
//! that replays and tests work everywhere.
//!
//! * Reads classic pcap (µs or ns timestamps, either byte order) and pcapng
//!   (Section Header, Interface Description and Enhanced/Simple Packet blocks),
//!   which is what Wireshark and tcpdump produce.
//! * Writes classic little-endian pcap with microsecond timestamps.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, ErrorKind, Read, Write};
use std::path::Path;

use crate::util::Micros;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Microseconds since the Unix epoch.
    pub ts: Micros,
    /// Link type (DLT_*) of the interface the packet was captured on.
    pub linktype: i32,
    pub data: Vec<u8>,
}

const PCAP_US: u32 = 0xa1b2_c3d4;
const PCAP_NS: u32 = 0xa1b2_3c4d;
const PCAPNG_SHB: u32 = 0x0a0d_0d0a;
const PCAPNG_BOM: u32 = 0x1a2b_3c4d;
const MAX_BLOCK: u32 = 64 * 1024 * 1024;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, msg.to_string())
}

#[derive(Clone, Copy)]
struct Endian {
    big: bool,
}

impl Endian {
    fn u16(self, b: &[u8]) -> u16 {
        let a = [b[0], b[1]];
        if self.big {
            u16::from_be_bytes(a)
        } else {
            u16::from_le_bytes(a)
        }
    }
    fn u32(self, b: &[u8]) -> u32 {
        let a = [b[0], b[1], b[2], b[3]];
        if self.big {
            u32::from_be_bytes(a)
        } else {
            u32::from_le_bytes(a)
        }
    }
}

struct Interface {
    linktype: i32,
    /// Timestamp units per second.
    units_per_sec: u64,
}

enum Format {
    Pcap {
        endian: Endian,
        linktype: i32,
        nanos: bool,
    },
    Pcapng {
        endian: Endian,
        interfaces: Vec<Interface>,
    },
}

pub struct Reader {
    input: BufReader<File>,
    format: Format,
}

/// Reads exactly `buf.len()` bytes; Ok(false) on a clean or truncated end of file.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<bool> {
    let mut done = 0;
    while done < buf.len() {
        match r.read(&mut buf[done..]) {
            Ok(0) => return Ok(false),
            Ok(n) => done += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

impl Reader {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut input = BufReader::new(File::open(path)?);
        let mut head = [0u8; 4];
        if !read_full(&mut input, &mut head)? {
            return Err(invalid("empty file"));
        }
        let le = u32::from_le_bytes(head);
        let be = u32::from_be_bytes(head);
        let format = if le == PCAPNG_SHB {
            let mut rest = [0u8; 8];
            if !read_full(&mut input, &mut rest)? {
                return Err(invalid("truncated pcapng header"));
            }
            let endian = match (
                u32::from_le_bytes(rest[4..8].try_into().unwrap()),
                u32::from_be_bytes(rest[4..8].try_into().unwrap()),
            ) {
                (PCAPNG_BOM, _) => Endian { big: false },
                (_, PCAPNG_BOM) => Endian { big: true },
                _ => return Err(invalid("bad pcapng byte-order magic")),
            };
            let len = endian.u32(&rest[0..4]);
            if !(28..=MAX_BLOCK).contains(&len) {
                return Err(invalid("bad pcapng section header length"));
            }
            let mut skip = vec![0u8; len as usize - 12];
            if !read_full(&mut input, &mut skip)? {
                return Err(invalid("truncated pcapng header"));
            }
            Format::Pcapng {
                endian,
                interfaces: Vec::new(),
            }
        } else {
            let (endian, nanos) = match (le, be) {
                (PCAP_US, _) => (Endian { big: false }, false),
                (PCAP_NS, _) => (Endian { big: false }, true),
                (_, PCAP_US) => (Endian { big: true }, false),
                (_, PCAP_NS) => (Endian { big: true }, true),
                _ => return Err(invalid("not a pcap or pcapng file")),
            };
            let mut rest = [0u8; 20];
            if !read_full(&mut input, &mut rest)? {
                return Err(invalid("truncated pcap header"));
            }
            let linktype = (endian.u32(&rest[16..20]) & 0x0fff_ffff) as i32;
            Format::Pcap {
                endian,
                linktype,
                nanos,
            }
        };
        Ok(Self { input, format })
    }

    fn next_pcap(&mut self) -> io::Result<Option<Record>> {
        let Format::Pcap {
            endian,
            linktype,
            nanos,
        } = self.format
        else {
            unreachable!()
        };
        let mut h = [0u8; 16];
        if !read_full(&mut self.input, &mut h)? {
            return Ok(None);
        }
        let sec = endian.u32(&h[0..4]) as i64;
        let frac = endian.u32(&h[4..8]) as i64;
        let caplen = endian.u32(&h[8..12]);
        if caplen > MAX_BLOCK {
            return Err(invalid("bad pcap record length"));
        }
        let mut data = vec![0u8; caplen as usize];
        if !read_full(&mut self.input, &mut data)? {
            return Ok(None); // truncated last record (e.g. capture interrupted)
        }
        let ts = sec * 1_000_000 + if nanos { frac / 1000 } else { frac };
        Ok(Some(Record { ts, linktype, data }))
    }

    fn next_pcapng(&mut self) -> io::Result<Option<Record>> {
        loop {
            let Format::Pcapng { endian, .. } = self.format else {
                unreachable!()
            };
            let mut h = [0u8; 8];
            if !read_full(&mut self.input, &mut h)? {
                return Ok(None);
            }
            let raw_type = u32::from_le_bytes(h[0..4].try_into().unwrap());
            if raw_type == PCAPNG_SHB {
                // New section: may switch byte order; interfaces are reset.
                let mut bom = [0u8; 4];
                if !read_full(&mut self.input, &mut bom)? {
                    return Ok(None);
                }
                let endian = match (u32::from_le_bytes(bom), u32::from_be_bytes(bom)) {
                    (PCAPNG_BOM, _) => Endian { big: false },
                    (_, PCAPNG_BOM) => Endian { big: true },
                    _ => return Err(invalid("bad pcapng byte-order magic")),
                };
                let len = endian.u32(&h[4..8]);
                if !(28..=MAX_BLOCK).contains(&len) {
                    return Err(invalid("bad pcapng block length"));
                }
                let mut skip = vec![0u8; len as usize - 12];
                if !read_full(&mut self.input, &mut skip)? {
                    return Ok(None);
                }
                self.format = Format::Pcapng {
                    endian,
                    interfaces: Vec::new(),
                };
                continue;
            }
            let btype = endian.u32(&h[0..4]);
            let len = endian.u32(&h[4..8]);
            if !(12..=MAX_BLOCK).contains(&len) || len % 4 != 0 {
                return Err(invalid("bad pcapng block length"));
            }
            let mut body = vec![0u8; len as usize - 8];
            if !read_full(&mut self.input, &mut body)? {
                return Ok(None);
            }
            let body = &body[..body.len() - 4]; // trailing length copy
            let Format::Pcapng { interfaces, .. } = &mut self.format else {
                unreachable!()
            };
            match btype {
                // Interface Description Block
                1 if body.len() >= 8 => {
                    let linktype = endian.u16(&body[0..2]) as i32;
                    let mut units_per_sec = 1_000_000u64;
                    let mut opts = &body[8..];
                    while opts.len() >= 4 {
                        let code = endian.u16(&opts[0..2]);
                        let olen = endian.u16(&opts[2..4]) as usize;
                        if code == 0 || opts.len() < 4 + olen {
                            break;
                        }
                        if code == 9 && olen >= 1 {
                            let v = opts[4];
                            let exp = (v & 0x7f) as u32;
                            units_per_sec = if v & 0x80 == 0 {
                                10u64.checked_pow(exp).unwrap_or(1_000_000)
                            } else {
                                1u64.checked_shl(exp).unwrap_or(1_000_000)
                            };
                        }
                        opts = &opts[((4 + olen + 3) & !3).min(opts.len())..];
                    }
                    interfaces.push(Interface {
                        linktype,
                        units_per_sec,
                    });
                }
                // Enhanced Packet Block
                6 if body.len() >= 20 => {
                    let iface = endian.u32(&body[0..4]) as usize;
                    let ts_raw =
                        ((endian.u32(&body[4..8]) as u64) << 32) | endian.u32(&body[8..12]) as u64;
                    let caplen = endian.u32(&body[12..16]) as usize;
                    let Some(i) = interfaces.get(iface) else {
                        continue;
                    };
                    let data = body
                        .get(20..20 + caplen)
                        .ok_or_else(|| invalid("bad packet length"))?;
                    let ts = (ts_raw as u128 * 1_000_000 / i.units_per_sec as u128) as Micros;
                    return Ok(Some(Record {
                        ts,
                        linktype: i.linktype,
                        data: data.to_vec(),
                    }));
                }
                // Simple Packet Block: no timestamp, not usable for timing.
                _ => {}
            }
        }
    }
}

impl Iterator for Reader {
    type Item = io::Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        let r = match self.format {
            Format::Pcap { .. } => self.next_pcap(),
            Format::Pcapng { .. } => self.next_pcapng(),
        };
        r.transpose()
    }
}

/// Classic pcap writer (little endian, microseconds).
pub struct Writer {
    out: BufWriter<File>,
}

impl Writer {
    pub fn create(path: &Path, linktype: i32) -> io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&PCAP_US.to_le_bytes())?;
        out.write_all(&2u16.to_le_bytes())?;
        out.write_all(&4u16.to_le_bytes())?;
        out.write_all(&0i32.to_le_bytes())?;
        out.write_all(&0u32.to_le_bytes())?;
        out.write_all(&65535u32.to_le_bytes())?;
        out.write_all(&(linktype as u32).to_le_bytes())?;
        Ok(Self { out })
    }

    pub fn write(&mut self, ts: Micros, data: &[u8]) -> io::Result<()> {
        let sec = ts.div_euclid(1_000_000) as u32;
        let usec = ts.rem_euclid(1_000_000) as u32;
        self.out.write_all(&sec.to_le_bytes())?;
        self.out.write_all(&usec.to_le_bytes())?;
        self.out.write_all(&(data.len() as u32).to_le_bytes())?;
        self.out.write_all(&(data.len() as u32).to_le_bytes())?;
        self.out.write_all(data)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("efm-cf-{}-{name}", std::process::id()))
    }

    #[test]
    fn pcap_roundtrip_and_truncation() {
        let p = tmp("rt.pcap");
        let mut w = Writer::create(&p, 1).unwrap();
        w.write(1_700_000_000_123_456, &[1, 2, 3]).unwrap();
        w.write(1_700_000_001_000_000, &[4; 60]).unwrap();
        w.flush().unwrap();
        drop(w);
        // Simulate an interrupted capture: append half a record header.
        let mut bytes = std::fs::read(&p).unwrap();
        bytes.extend_from_slice(&[0u8; 7]);
        std::fs::write(&p, bytes).unwrap();
        let recs: Vec<Record> = Reader::open(&p).unwrap().map(Result::unwrap).collect();
        std::fs::remove_file(&p).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(
            recs[0],
            Record {
                ts: 1_700_000_000_123_456,
                linktype: 1,
                data: vec![1, 2, 3]
            }
        );
        assert_eq!(recs[1].data.len(), 60);
    }

    fn block(t: u32, body: &[u8]) -> Vec<u8> {
        let len = (12 + body.len() + 3) & !3;
        let mut v = t.to_le_bytes().to_vec();
        v.extend_from_slice(&(len as u32).to_le_bytes());
        v.extend_from_slice(body);
        v.resize(len - 4, 0);
        v.extend_from_slice(&(len as u32).to_le_bytes());
        v
    }

    #[test]
    fn reads_pcapng_with_nanosecond_resolution() {
        let mut f = Vec::new();
        let mut shb = PCAPNG_BOM.to_le_bytes().to_vec();
        shb.extend_from_slice(&[1, 0, 0, 0]);
        shb.extend_from_slice(&(-1i64).to_le_bytes());
        f.extend(block(PCAPNG_SHB, &shb));
        let mut idb = vec![101, 0, 0, 0, 0xff, 0xff, 0, 0]; // LINKTYPE_RAW
        idb.extend_from_slice(&[9, 0, 1, 0, 9, 0, 0, 0]); // if_tsresol = 10^-9
        idb.extend_from_slice(&[0, 0, 0, 0]); // opt_endofopt
        f.extend(block(1, &idb));
        let ts_ns: u64 = 1_700_000_000_123_456_789;
        let mut epb = 0u32.to_le_bytes().to_vec();
        epb.extend_from_slice(&((ts_ns >> 32) as u32).to_le_bytes());
        epb.extend_from_slice(&(ts_ns as u32).to_le_bytes());
        epb.extend_from_slice(&5u32.to_le_bytes());
        epb.extend_from_slice(&5u32.to_le_bytes());
        epb.extend_from_slice(&[9, 8, 7, 6, 5]);
        f.extend(block(6, &epb));
        let p = tmp("ng.pcapng");
        std::fs::write(&p, f).unwrap();
        let recs: Vec<Record> = Reader::open(&p).unwrap().map(Result::unwrap).collect();
        std::fs::remove_file(&p).unwrap();
        assert_eq!(
            recs,
            vec![Record {
                ts: 1_700_000_000_123_456,
                linktype: 101,
                data: vec![9, 8, 7, 6, 5]
            }]
        );
    }

    #[test]
    fn rejects_other_files() {
        let p = tmp("bad.txt");
        std::fs::write(&p, b"hello world, not a capture").unwrap();
        assert!(Reader::open(&p).is_err());
        std::fs::remove_file(&p).unwrap();
    }
}
