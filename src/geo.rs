//! Geolocation of remote hosts from the free IP2Location LITE DB3 databases
//! (`IP2LOCATION-LITE-DB3.BIN` / `IP2LOCATION-LITE-DB3.IPV6.BIN`,
//! https://lite.ip2location.com). Falls back to the cloud provider region when
//! the databases are missing.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;

pub struct Bin {
    file: Mutex<File>,
    columns: u32,
    v4_count: u32,
    v4_addr: u32,
    v6_count: u32,
    v6_addr: u32,
}

impl Bin {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let mut f = File::open(path)?;
        let mut h = [0u8; 29];
        f.read_exact(&mut h)?;
        let u = |o: usize| u32::from_le_bytes(h[o..o + 4].try_into().unwrap());
        Ok(Self {
            columns: h[1] as u32,
            v4_count: u(5),
            v4_addr: u(9),
            v6_count: u(13),
            v6_addr: u(17),
            file: Mutex::new(f),
        })
    }

    fn read(&self, f: &mut File, pos: u64, buf: &mut [u8]) -> std::io::Result<()> {
        f.seek(SeekFrom::Start(pos))?;
        f.read_exact(buf)
    }

    fn u32_at(&self, f: &mut File, pos: u64) -> std::io::Result<u32> {
        let mut b = [0u8; 4];
        self.read(f, pos, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn string_at(&self, f: &mut File, pos: u64) -> std::io::Result<String> {
        let mut l = [0u8; 1];
        self.read(f, pos, &mut l)?;
        let mut s = vec![0u8; l[0] as usize];
        self.read(f, pos + 1, &mut s)?;
        Ok(String::from_utf8_lossy(&s).into_owned())
    }

    /// Returns (country long name, region, city).
    pub fn lookup(&self, ip: IpAddr) -> Option<(String, String, String)> {
        let (key, count, base, ipw) = match ip {
            IpAddr::V4(a) => (u32::from(a) as u128, self.v4_count, self.v4_addr, 4u64),
            IpAddr::V6(a) => match a.to_ipv4_mapped() {
                Some(v4) => (u32::from(v4) as u128, self.v4_count, self.v4_addr, 4),
                None => (u128::from(a), self.v6_count, self.v6_addr, 16),
            },
        };
        if count == 0 || self.columns < 4 {
            return None;
        }
        let row = ipw + (self.columns as u64 - 1) * 4;
        let base = base as u64 - 1;
        let mut f = self.file.lock().ok()?;
        let read_ip = |f: &mut File, pos: u64| -> Option<u128> {
            let mut b = [0u8; 16];
            self.read(f, pos, &mut b[..ipw as usize]).ok()?;
            Some(u128::from_le_bytes(b))
        };
        let (mut lo, mut hi) = (0u64, count as u64);
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let off = base + mid * row;
            let from = read_ip(&mut f, off)?;
            let to = read_ip(&mut f, off + row)?;
            if from <= key && key < to {
                // Column 2 = country: pointer to [short code][long name at +3].
                let field = |f: &mut File, col: u64| -> Option<String> {
                    let p = self.u32_at(f, off + ipw + (col - 2) * 4).ok()? as u64;
                    self.string_at(f, if col == 2 { p + 3 } else { p }).ok()
                };
                return Some((field(&mut f, 2)?, field(&mut f, 3)?, field(&mut f, 4)?));
            }
            if key < from {
                hi = mid.checked_sub(1)?;
            } else {
                lo = mid + 1;
            }
        }
        None
    }
}

#[derive(Default)]
pub struct GeoDb {
    v4: Option<Bin>,
    v6: Option<Bin>,
}

impl GeoDb {
    pub fn open(dir: &Path) -> Self {
        Self {
            v4: Bin::open(&dir.join("IP2LOCATION-LITE-DB3.BIN")).ok(),
            v6: Bin::open(&dir.join("IP2LOCATION-LITE-DB3.IPV6.BIN")).ok(),
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.v4.is_some() || self.v6.is_some()
    }

    /// `Country-Region-City` (with a few shortened country names). Anycast
    /// addresses, which IP2Location places at the provider's headquarters, and
    /// addresses missing from the database fall back to the cloud region.
    pub fn area(&self, ip: &str) -> String {
        let cloud = crate::cloud::lookup(ip);
        if let Some((p, "GLOBAL")) = cloud {
            return format!("Anycast ({})", p.name());
        }
        match self.record(ip) {
            Some((c, r, city)) if c != "-" => {
                let c = match c.as_str() {
                    c if c.contains("United Kingdom") || c.contains("Great Britain") => {
                        "UK".to_string()
                    }
                    c if c.contains("United States of America") => "USA".to_string(),
                    c if c.contains("Netherlands") => "Netherlands".to_string(),
                    _ => c,
                };
                format!("{c}-{r}-{city}")
            }
            _ => match cloud {
                Some(_) => crate::cloud::describe(ip),
                None => "unknown".into(),
            },
        }
    }

    /// Short location name without spaces (used for speed test servers).
    pub fn city(&self, ip: &str) -> String {
        if let Some((_, "GLOBAL")) = crate::cloud::lookup(ip) {
            return "Anycast".into();
        }
        match self.record(ip) {
            Some((c, _, city)) if c != "-" => city.replace(' ', "_"),
            _ => crate::cloud::lookup(ip).map_or_else(|| "unknown".into(), |(_, r)| r.to_string()),
        }
    }

    fn record(&self, ip: &str) -> Option<(String, String, String)> {
        let addr = ip.parse::<IpAddr>().ok()?;
        let db = match addr {
            IpAddr::V4(_) => self.v4.as_ref().or(self.v6.as_ref()),
            IpAddr::V6(_) => self.v6.as_ref(),
        };
        db.and_then(|d| d.lookup(addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_cloud_region_without_db() {
        let g = GeoDb::default();
        assert!(
            g.area("34.84.43.65")
                .starts_with("Google Cloud asia-northeast1"),
            "{}",
            g.area("34.84.43.65")
        );
        assert_eq!(g.area("15.197.143.184"), "Anycast (AWS)");
        assert_eq!(g.city("15.188.178.40"), "eu-west-3");
        assert_eq!(g.area("81.2.3.4"), "unknown");
    }

    /// Runs against the real databases when `EFM_GEO_DIR` points at the folder holding them.
    #[test]
    fn lookup_real_db() {
        let Ok(dir) = std::env::var("EFM_GEO_DIR") else {
            return;
        };
        let g = GeoDb::open(Path::new(&dir));
        assert!(g.is_loaded());
        assert_eq!(g.area("34.84.43.65"), "Japan-Tokyo-Tokyo");
        assert!(
            g.area("8.8.8.8").starts_with("USA-California"),
            "{}",
            g.area("8.8.8.8")
        );
        assert!(g.area("2001:4860:4860::8888").starts_with("USA"));
        assert_eq!(g.city("34.159.137.177"), "Frankfurt_am_Main");
    }
}
