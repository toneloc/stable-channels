//! Offline country lookup for peer IPs (DB-IP Country Lite, CC BY 4.0) and sighting capture.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use stable_channels::db::Database;
use tracing::warn;

pub trait CountryLookup: Send + Sync {
    /// ISO country code and English name for `ip`, when the database knows it.
    fn country(&self, ip: IpAddr) -> Option<(String, String)>;
}

pub struct NoCountry;

impl CountryLookup for NoCountry {
    fn country(&self, _ip: IpAddr) -> Option<(String, String)> {
        None
    }
}

struct DbIp(maxminddb::Reader<Vec<u8>>);

impl CountryLookup for DbIp {
    fn country(&self, ip: IpAddr) -> Option<(String, String)> {
        let found = self.0.lookup(ip).ok()?.decode::<maxminddb::geoip2::Country>().ok()??;
        let code = found.country.iso_code?.to_owned();
        let name = found.country.names.english.unwrap_or(code.as_str()).to_owned();
        Some((code, name))
    }
}

/// Opens the configured `.mmdb`; without one, sightings (if `record_ips` is on) carry no country.
pub fn load(path: Option<&str>) -> Arc<dyn CountryLookup> {
    let Some(path) = path else { return Arc::new(NoCountry) };
    match maxminddb::Reader::open_readfile(path) {
        Ok(reader) => Arc::new(DbIp(reader)),
        Err(error) => {
            warn!("[geoip] cannot open {}: {}; countries will be unknown", path, error);
            Arc::new(NoCountry)
        },
    }
}

/// The IP of an `ip:port` peer address; onion, hostnames and garbage give None.
pub fn peer_ip(address: &str) -> Option<IpAddr> {
    address.parse::<SocketAddr>().ok().map(|socket| socket.ip())
}

/// Records connected stable counterparties (node id, address, connected); looks up country until the IP has one.
pub fn record_sightings(
    db: &Database,
    lookup: &dyn CountryLookup,
    peers: &[(String, String, bool)],
    stable_nodes: &HashSet<String>,
    now: i64,
) {
    for (node, address, connected) in peers {
        if !*connected || !stable_nodes.contains(node) {
            continue;
        }
        let Some(ip) = peer_ip(address) else { continue };
        let ip_text = ip.to_string();
        let country = match db.peer_ip_has_country(node, &ip_text) {
            Ok(true) => None,
            Ok(false) => lookup.country(ip),
            Err(error) => {
                warn!("[geoip] country check failed: {}", error);
                continue;
            },
        };
        let country = country.as_ref().map(|(code, name)| (code.as_str(), name.as_str()));
        if let Err(error) = db.record_peer_sighting(node, &ip_text, country, now) {
            warn!("[geoip] record_peer_sighting failed: {}", error);
        }
    }
}

/// Drops sightings last seen before `cutoff`, or all of them once recording is off so stored IPs do not linger.
pub fn prune(db: &Database, record_ips: bool, cutoff: i64) {
    if let Err(error) = db.prune_peer_locations(if record_ips { cutoff } else { i64::MAX }) {
        warn!("[geoip] prune_peer_locations failed: {}", error);
    }
}

/// Ledger detail for PEER_CONNECTED/DISCONNECTED; the address lives in peer_locations, never the ledger.
pub fn peer_event_detail(node: &str, user_channel_ids: Option<&Vec<String>>) -> serde_json::Value {
    serde_json::json!({
        "counterparty_node_id": node,
        "user_channel_ids": user_channel_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real on-disk database in a temp dir, like the daemon's other tests.
    fn temp_db() -> (tempfile::TempDir, stable_channels::db::Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = stable_channels::db::Database::open(dir.path()).unwrap();
        (dir, db)
    }

    struct Fake;
    impl CountryLookup for Fake {
        fn country(&self, ip: IpAddr) -> Option<(String, String)> {
            (ip.to_string() == "118.95.161.42").then(|| ("IN".to_owned(), "India".to_owned()))
        }
    }

    #[test]
    fn peer_addresses_parse_to_ips_or_are_skipped() {
        assert_eq!(peer_ip("118.95.161.42:49576"), "118.95.161.42".parse().ok());
        assert_eq!(peer_ip("[2001:db8::1]:9735"), "2001:db8::1".parse().ok());
        for bad in ["", "abcdefghijklmnop.onion:9735", "lsp.example.com:9735", "garbage"] {
            assert_eq!(peer_ip(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_connected_stable_peers_are_recorded() {
        let (_dir, db) = temp_db();
        let stable: HashSet<String> = ["wallet".to_owned()].into();
        let peers = vec![
            ("wallet".to_owned(), "118.95.161.42:49576".to_owned(), true),
            ("router".to_owned(), "46.224.104.1:43362".to_owned(), true),
            ("wallet-offline".to_owned(), "9.9.9.9:1".to_owned(), false),
        ];
        record_sightings(&db, &Fake, &peers, &stable, 100);
        let rows = db.recent_peer_locations("wallet", 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].country_code.as_deref(), rows[0].country_name.as_deref()), (Some("IN"), Some("India")));
        assert!(db.recent_peer_locations("router", 10).unwrap().is_empty());
    }

    #[test]
    fn a_missing_database_still_records_the_ip() {
        let (_dir, db) = temp_db();
        let lookup = load(Some("/nonexistent/dbip.mmdb"));
        let stable: HashSet<String> = ["wallet".to_owned()].into();
        record_sightings(&db, lookup.as_ref(), &[("wallet".to_owned(), "1.2.3.4:5".to_owned(), true)], &stable, 7);
        let rows = db.recent_peer_locations("wallet", 10).unwrap();
        assert_eq!((rows[0].ip.as_str(), rows[0].country_code.as_deref()), ("1.2.3.4", None));
    }

    struct Counting(std::sync::atomic::AtomicU32);
    impl CountryLookup for Counting {
        fn country(&self, ip: IpAddr) -> Option<(String, String)> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Fake.country(ip)
        }
    }

    #[test]
    fn a_country_missed_at_first_sighting_is_filled_later_then_never_looked_up_again() {
        let (_dir, db) = temp_db();
        let stable: HashSet<String> = ["wallet".to_owned()].into();
        let peers = vec![("wallet".to_owned(), "118.95.161.42:49576".to_owned(), true)];
        record_sightings(&db, &NoCountry, &peers, &stable, 100);
        let counting = Counting(Default::default());
        record_sightings(&db, &counting, &peers, &stable, 130);
        record_sightings(&db, &counting, &peers, &stable, 160);
        let rows = db.recent_peer_locations("wallet", 10).unwrap();
        assert_eq!((rows[0].country_code.as_deref(), rows[0].last_seen_at), (Some("IN"), 160));
        assert_eq!(counting.0.load(std::sync::atomic::Ordering::Relaxed), 1, "looked up once to fill the gap, then skipped");
    }

    #[test]
    fn a_poll_prunes_by_age_while_recording_and_drops_everything_once_it_is_off() {
        let (_dir, db) = temp_db();
        let stable: HashSet<String> = ["wallet".to_owned()].into();
        record_sightings(&db, &NoCountry, &[("wallet".to_owned(), "1.2.3.4:5".to_owned(), true)], &stable, 100);
        record_sightings(&db, &NoCountry, &[("wallet".to_owned(), "5.6.7.8:5".to_owned(), true)], &stable, 300);
        prune(&db, true, 200);
        assert_eq!(db.recent_peer_locations("wallet", 10).unwrap().iter().map(|r| r.ip.as_str()).collect::<Vec<_>>(), ["5.6.7.8"]);
        prune(&db, false, 200);
        assert!(db.recent_peer_locations("wallet", 10).unwrap().is_empty(), "recording off keeps no IP");
    }

    #[test]
    fn peer_events_no_longer_carry_the_address() {
        let detail = peer_event_detail("node", Some(&vec!["uid".to_owned()]));
        assert!(detail.get("address").is_none());
        assert_eq!(detail["counterparty_node_id"], "node");
    }
}
