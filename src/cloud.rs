//! Cloud provider lookup for game servers, based on the providers' published
//! IP ranges (see `tools/update_cloud_ranges.py`).

use std::net::IpAddr;

use crate::cloud_ranges::{AWS, GOOGLE_CLOUD, REGIONS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    GoogleCloud,
    Aws,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::GoogleCloud => "Google Cloud",
            Provider::Aws => "AWS",
        }
    }
}

fn find(table: &[(u32, u32, u16)], v: u32) -> Option<&'static str> {
    // Ranges are sorted by first address; a few may overlap, so look back a little.
    let end = table.partition_point(|&(lo, _, _)| lo <= v);
    table[..end]
        .iter()
        .rev()
        .take(8)
        .find(|&&(lo, hi, _)| lo <= v && v <= hi)
        .map(|&(_, _, r)| REGIONS[r as usize])
}

/// Provider and provider region (e.g. `asia-northeast1`, `eu-west-3`, `GLOBAL`) of an IPv4 address.
pub fn lookup(ip: &str) -> Option<(Provider, &'static str)> {
    let v = u32::from(match ip.parse::<IpAddr>().ok()? {
        IpAddr::V4(a) => a,
        IpAddr::V6(a) => a.to_ipv4_mapped()?,
    });
    if let Some(r) = find(GOOGLE_CLOUD, v) {
        return Some((Provider::GoogleCloud, r));
    }
    find(AWS, v).map(|r| (Provider::Aws, r))
}

/// "Google Cloud asia-northeast1", "AWS eu-west-3", or "unknown host".
pub fn describe(ip: &str) -> String {
    match lookup(ip) {
        Some((p, r)) if !r.is_empty() => format!("{} {}", p.name(), r),
        Some((p, _)) => p.name().to_string(),
        None => "unknown host".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted() {
        assert!(GOOGLE_CLOUD.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(AWS.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn known_game_servers() {
        // Observed eFootball servers: Google Cloud Tokyo, AWS Paris, AWS Global Accelerator.
        assert_eq!(
            lookup("34.84.43.65").map(|x| x.0),
            Some(Provider::GoogleCloud)
        );
        assert_eq!(lookup("15.188.178.40").map(|x| x.0), Some(Provider::Aws));
        assert_eq!(lookup("15.197.143.184").map(|x| x.0), Some(Provider::Aws));
        assert_eq!(lookup("81.2.3.4"), None);
        assert_eq!(lookup("192.168.1.1"), None);
    }
}
