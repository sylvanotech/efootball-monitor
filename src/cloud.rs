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

/// City and country of a cloud region, as documented by Google Cloud and AWS.
/// Regions missing here are shown by their code.
pub fn region_location(region: &str) -> Option<&'static str> {
    Some(match region {
        // Google Cloud
        "africa-south1" => "Johannesburg, South Africa",
        "asia-east1" => "Changhua, Taiwan",
        "asia-east2" => "Hong Kong",
        "asia-northeast1" => "Tokyo, Japan",
        "asia-northeast2" => "Osaka, Japan",
        "asia-northeast3" => "Seoul, South Korea",
        "asia-south1" => "Mumbai, India",
        "asia-south2" => "Delhi, India",
        "asia-southeast1" => "Singapore",
        "asia-southeast2" => "Jakarta, Indonesia",
        "australia-southeast1" => "Sydney, Australia",
        "australia-southeast2" => "Melbourne, Australia",
        "europe-central2" => "Warsaw, Poland",
        "europe-north1" => "Hamina, Finland",
        "europe-north2" => "Stockholm, Sweden",
        "europe-southwest1" => "Madrid, Spain",
        "europe-west1" => "St. Ghislain, Belgium",
        "europe-west2" => "London, UK",
        "europe-west3" => "Frankfurt, Germany",
        "europe-west4" => "Eemshaven, Netherlands",
        "europe-west6" => "Zurich, Switzerland",
        "europe-west8" => "Milan, Italy",
        "europe-west9" => "Paris, France",
        "europe-west10" => "Berlin, Germany",
        "europe-west12" => "Turin, Italy",
        "me-central1" => "Doha, Qatar",
        "me-central2" => "Dammam, Saudi Arabia",
        "me-west1" => "Tel Aviv, Israel",
        "northamerica-northeast1" => "Montreal, Canada",
        "northamerica-northeast2" => "Toronto, Canada",
        "northamerica-south1" => "Queretaro, Mexico",
        "southamerica-east1" => "Sao Paulo, Brazil",
        "southamerica-west1" => "Santiago, Chile",
        "us-central1" => "Iowa, USA",
        "us-east1" => "South Carolina, USA",
        "us-east4" => "Virginia, USA",
        "us-east5" => "Ohio, USA",
        "us-south1" => "Dallas, USA",
        "us-west1" => "Oregon, USA",
        "us-west2" => "Los Angeles, USA",
        "us-west3" => "Salt Lake City, USA",
        "us-west4" => "Las Vegas, USA",
        // AWS
        "af-south-1" => "Cape Town, South Africa",
        "ap-east-1" => "Hong Kong",
        "ap-east-2" => "Taipei, Taiwan",
        "ap-northeast-1" => "Tokyo, Japan",
        "ap-northeast-2" => "Seoul, South Korea",
        "ap-northeast-3" => "Osaka, Japan",
        "ap-south-1" => "Mumbai, India",
        "ap-south-2" => "Hyderabad, India",
        "ap-southeast-1" => "Singapore",
        "ap-southeast-2" => "Sydney, Australia",
        "ap-southeast-3" => "Jakarta, Indonesia",
        "ap-southeast-4" => "Melbourne, Australia",
        "ap-southeast-5" => "Kuala Lumpur, Malaysia",
        "ap-southeast-6" => "Auckland, New Zealand",
        "ap-southeast-7" => "Bangkok, Thailand",
        "ca-central-1" => "Montreal, Canada",
        "ca-west-1" => "Calgary, Canada",
        "cn-north-1" => "Beijing, China",
        "cn-northwest-1" => "Ningxia, China",
        "eu-central-1" => "Frankfurt, Germany",
        "eu-central-2" => "Zurich, Switzerland",
        "eu-north-1" => "Stockholm, Sweden",
        "eu-south-1" => "Milan, Italy",
        "eu-south-2" => "Aragon, Spain",
        "eu-west-1" => "Dublin, Ireland",
        "eu-west-2" => "London, UK",
        "eu-west-3" => "Paris, France",
        "il-central-1" => "Tel Aviv, Israel",
        "me-central-1" => "UAE",
        "me-south-1" => "Bahrain",
        "mx-central-1" => "Queretaro, Mexico",
        "sa-east-1" => "Sao Paulo, Brazil",
        "us-east-1" => "Virginia, USA",
        "us-east-2" => "Ohio, USA",
        "us-west-1" => "California, USA",
        "us-west-2" => "Oregon, USA",
        _ => return None,
    })
}

/// City and country of a cloud server, when its region is known.
pub fn location(ip: &str) -> Option<&'static str> {
    lookup(ip).and_then(|(_, r)| region_location(r))
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

    #[test]
    fn region_locations() {
        assert_eq!(location("34.84.43.65"), Some("Tokyo, Japan"));
        assert_eq!(location("15.188.178.40"), Some("Paris, France"));
        assert_eq!(
            region_location("africa-south1"),
            Some("Johannesburg, South Africa")
        );
        assert_eq!(location("15.197.143.184"), None); // GLOBAL
    }
}
