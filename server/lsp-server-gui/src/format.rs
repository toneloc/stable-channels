//! Pure formatting and parsing helpers shared by every screen.

use crate::state::DisplayUnit;

// Counts chars, not bytes, so pasted text with non-ASCII never panics.
pub fn truncate_id(s: &str, start: usize, end: usize) -> String {
	let len = s.chars().count();
	if len <= start + end + 2 {
		s.to_string()
	} else {
		let head: String = s.chars().take(start).collect();
		let tail: String = s.chars().skip(len - end).collect();
		format!("{head}..{tail}")
	}
}

pub fn format_sats(sats: u64) -> String {
	let s = sats.to_string();
	let mut result = String::new();
	for (i, c) in s.chars().rev().enumerate() {
		if i > 0 && i % 3 == 0 {
			result.insert(0, ',');
		}
		result.insert(0, c);
	}
	result
}

pub fn format_msat(msat: u64) -> String {
	let sats = msat / 1000;
	let remainder = msat % 1000;
	if remainder == 0 {
		format!("{} sats", format_sats(sats))
	} else {
		format!("{}.{:03} sats", format_sats(sats), remainder)
	}
}

/// Country for a peer location: "IN · India", or "Unknown" without database data.
pub fn location_label(loc: &sc_rest_client::sc_protos::stable::PeerLocation) -> String {
	match (loc.country_code.is_empty(), loc.country_name.is_empty()) {
		(false, false) => format!("{} · {}", loc.country_code, loc.country_name),
		(false, true) => loc.country_code.clone(),
		_ => "Unknown".to_owned(),
	}
}

/// Live only when the peer list says connected and the daemon's 30 s poll saw it within 90 s.
pub fn location_is_live(connected: Option<bool>, last_seen_at: i64, now_secs: i64) -> bool {
	connected == Some(true) && now_secs - last_seen_at <= 90
}

pub fn format_usd(amount: f64) -> String {
	let s = format!("{:.2}", amount);
	let (int_part, frac) = s.split_once('.').unwrap_or((s.as_str(), "00"));
	let dollars: u64 = int_part.parse().unwrap_or(0);
	format!("${}.{}", format_sats(dollars), frac)
}

pub fn format_amount_sats(sats: u64, unit: DisplayUnit, price: Option<f64>) -> String {
	match unit {
		DisplayUnit::Sats => format!("{} sats", format_sats(sats)),
		DisplayUnit::Btc => format!("{:.8} BTC", sats as f64 / 100_000_000.0),
		DisplayUnit::Usd => match price {
			Some(p) if p > 0.0 => format_usd((sats as f64 / 100_000_000.0) * p),
			_ => format!("{} sats", format_sats(sats)),
		},
	}
}

pub fn format_amount_msat(msat: u64, unit: DisplayUnit, price: Option<f64>) -> String {
	match unit {
		DisplayUnit::Sats => format_msat(msat),
		DisplayUnit::Btc => format!("{:.8} BTC", msat as f64 / 100_000_000_000.0),
		DisplayUnit::Usd => match price {
			Some(p) if p > 0.0 => format_usd((msat as f64 / 100_000_000_000.0) * p),
			_ => format_msat(msat),
		},
	}
}

/// Short label for the current entry unit, e.g. "USD" / "BTC" / "sats".
pub fn unit_label(unit: DisplayUnit) -> &'static str {
	match unit {
		DisplayUnit::Usd => "USD",
		DisplayUnit::Btc => "BTC",
		DisplayUnit::Sats => "sats",
	}
}

/// Parse a user-entered amount, interpreting it in `unit`, into whole sats.
/// Sats must be a whole number; BTC/USD accept decimals; USD needs a positive
/// price. Returns None for empty/invalid/negative input (or USD without price).
pub fn parse_amount_to_sats(input: &str, unit: DisplayUnit, price: Option<f64>) -> Option<u64> {
	let t = input.trim();
	if t.is_empty() {
		return None;
	}
	match unit {
		DisplayUnit::Sats => t.parse::<u64>().ok(),
		DisplayUnit::Btc => {
			let btc = t.parse::<f64>().ok()?;
			if !btc.is_finite() || btc < 0.0 {
				return None;
			}
			Some((btc * 100_000_000.0).round() as u64)
		},
		DisplayUnit::Usd => {
			let usd = t.parse::<f64>().ok()?;
			let p = price?;
			if !usd.is_finite() || usd < 0.0 || p <= 0.0 {
				return None;
			}
			Some(((usd / p) * 100_000_000.0).round() as u64)
		},
	}
}

/// Same as [`parse_amount_to_sats`] but scaled to msats for the Lightning APIs.
pub fn parse_amount_to_msat(input: &str, unit: DisplayUnit, price: Option<f64>) -> Option<u64> {
	parse_amount_to_sats(input, unit, price).map(|s| s.saturating_mul(1000))
}

/// Preview line under an amount field: the sats that will actually be sent
/// (USD/BTC modes), or the ≈USD value (sats mode); None if unparseable.
pub fn amount_entry_preview(input: &str, unit: DisplayUnit, price: Option<f64>) -> Option<String> {
	let sats = parse_amount_to_sats(input, unit, price)?;
	match unit {
		DisplayUnit::Sats => match price {
			Some(p) if p > 0.0 => Some(format!("≈ {}", format_amount_sats(sats, DisplayUnit::Usd, price))),
			_ => None,
		},
		_ => Some(format!("= {} sats", format_sats(sats))),
	}
}

/// Current unix time in seconds (browser clock on the web).
pub fn now_secs() -> u64 {
	chrono::Utc::now().timestamp().max(0) as u64
}

/// Long relative form used by Node Info, e.g. "5 minutes ago".
pub fn relative_long(ts: u64, now: u64) -> String {
	if now >= ts {
		let secs = now - ts;
		if secs < 60 {
			format!("{} seconds ago", secs)
		} else if secs < 3600 {
			format!("{} minutes ago", secs / 60)
		} else if secs < 86400 {
			format!("{} hours ago", secs / 3600)
		} else {
			format!("{} days ago", secs / 86400)
		}
	} else {
		format!("timestamp: {}", ts)
	}
}

/// Compact relative form used in tables, e.g. "5m ago".
pub fn relative_short(ts: u64, now: u64) -> String {
	if now >= ts {
		let secs = now - ts;
		if secs < 60 {
			format!("{}s ago", secs)
		} else if secs < 3600 {
			format!("{}m ago", secs / 60)
		} else if secs < 86400 {
			format!("{}h ago", secs / 3600)
		} else {
			format!("{}d ago", secs / 86400)
		}
	} else {
		format!("{}", ts)
	}
}

/// Local date and time, e.g. "27 Sep 2026, 12:04".
pub fn local_datetime(ts: u64) -> String {
	use chrono::TimeZone;
	chrono::Local
		.timestamp_opt(ts as i64, 0)
		.single()
		.map(|t| t.format("%d %b %Y, %H:%M").to_string())
		.unwrap_or_else(|| ts.to_string())
}

/// Local clock time, e.g. "10:42".
pub fn local_time(ts: u64) -> String {
	use chrono::TimeZone;
	chrono::Local
		.timestamp_opt(ts as i64, 0)
		.single()
		.map(|t| t.format("%H:%M").to_string())
		.unwrap_or_else(|| ts.to_string())
}

/// Short age for freshness labels, e.g. "just now", "45s ago", "3m ago".
pub fn ago(ts: u64, now: u64) -> String {
	match now.saturating_sub(ts) {
		0..=4 => "just now".to_string(),
		secs => relative_short(ts, ts + secs),
	}
}

/// Quote a CSV field when it contains a separator, quote or newline.
pub fn csv_field(value: &str) -> String {
	if value.contains([',', '"', '\n', '\r']) {
		format!("\"{}\"", value.replace('"', "\"\""))
	} else {
		value.to_string()
	}
}

/// Join fields into one CSV line.
pub fn csv_row(fields: &[String]) -> String {
	fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_location_reads_live_only_while_connected_and_recently_seen() {
		assert!(location_is_live(Some(true), 1_000, 1_030));
		assert!(!location_is_live(Some(true), 1_000, 1_000 + 3_600), "a stale peer list must not hide an hour-old sighting");
		assert!(!location_is_live(None, 1_000, 1_030));
		assert!(!location_is_live(Some(false), 1_000, 1_030));
	}

	#[test]
	fn locations_read_as_code_and_name_or_unknown() {
		let mut loc = sc_rest_client::sc_protos::stable::PeerLocation {
			country_code: "IN".into(),
			country_name: "India".into(),
			..Default::default()
		};
		assert_eq!(location_label(&loc), "IN · India");
		loc.country_code.clear();
		loc.country_name.clear();
		assert_eq!(location_label(&loc), "Unknown");
	}
	use crate::state::DisplayUnit;

	#[test]
	fn truncate_id_is_char_safe() {
		assert_eq!(truncate_id("0123456789abcdef", 4, 4), "0123..cdef");
		assert_eq!(truncate_id("short", 4, 4), "short");
		// Pasted text can end in a multi-byte char; slicing bytes here used to panic.
		assert_eq!(truncate_id("lnbc1234567890abcdef…", 6, 4), "lnbc12..def…");
	}
	#[test]
	fn sats_unit_groups_digits() {
		assert_eq!(format_amount_sats(50_000, DisplayUnit::Sats, None), "50,000 sats");
	}
	#[test]
	fn btc_unit_eight_decimals() {
		assert_eq!(format_amount_sats(100_000_000, DisplayUnit::Btc, None), "1.00000000 BTC");
	}
	#[test]
	fn usd_unit_uses_price() {
		assert_eq!(format_amount_sats(100_000_000, DisplayUnit::Usd, Some(100_000.0)), "$100,000.00");
	}
	#[test]
	fn usd_falls_back_to_sats_without_price() {
		assert_eq!(format_amount_sats(50_000, DisplayUnit::Usd, None), "50,000 sats");
	}
	#[test]
	fn usd_falls_back_to_sats_on_zero_price() {
		assert_eq!(format_amount_sats(50_000, DisplayUnit::Usd, Some(0.0)), "50,000 sats");
	}
	#[test]
	fn msat_usd_converts() {
		assert_eq!(format_amount_msat(100_000_000_000, DisplayUnit::Usd, Some(100_000.0)), "$100,000.00");
	}
	#[test]
	fn msat_sats_keeps_remainder_behavior() {
		assert_eq!(format_amount_msat(1_500, DisplayUnit::Sats, None), "1.500 sats");
	}

	#[test]
	fn parse_sats_is_whole_number() {
		assert_eq!(parse_amount_to_sats("50000", DisplayUnit::Sats, None), Some(50_000));
		assert_eq!(parse_amount_to_sats("1.5", DisplayUnit::Sats, None), None);
	}
	#[test]
	fn parse_btc_scales_to_sats() {
		assert_eq!(parse_amount_to_sats("1.5", DisplayUnit::Btc, None), Some(150_000_000));
	}
	#[test]
	fn parse_usd_uses_price() {
		assert_eq!(parse_amount_to_sats("65860", DisplayUnit::Usd, Some(65_860.0)), Some(100_000_000));
	}
	#[test]
	fn parse_usd_needs_positive_price() {
		assert_eq!(parse_amount_to_sats("10", DisplayUnit::Usd, None), None);
		assert_eq!(parse_amount_to_sats("10", DisplayUnit::Usd, Some(0.0)), None);
	}
	#[test]
	fn parse_rejects_empty_and_negative() {
		assert_eq!(parse_amount_to_sats("   ", DisplayUnit::Sats, None), None);
		assert_eq!(parse_amount_to_sats("-1", DisplayUnit::Btc, None), None);
	}
	#[test]
	fn parse_msat_scales_by_1000() {
		assert_eq!(parse_amount_to_msat("100", DisplayUnit::Sats, None), Some(100_000));
	}
}

#[cfg(test)]
mod preview_tests {
	use super::*;

	#[test]
	fn preview_shows_sats_for_fiat_and_usd_for_sats() {
		assert_eq!(amount_entry_preview("1", DisplayUnit::Btc, None).as_deref(), Some("= 100,000,000 sats"));
		assert_eq!(amount_entry_preview("100000", DisplayUnit::Sats, Some(100_000.0)).as_deref(), Some("≈ $100.00"));
		assert_eq!(amount_entry_preview("100000", DisplayUnit::Sats, None), None);
		assert_eq!(amount_entry_preview("x", DisplayUnit::Sats, None), None);
	}

	#[test]
	fn relative_forms_match_previous_wording() {
		assert_eq!(relative_long(100, 130), "30 seconds ago");
		assert_eq!(relative_long(0, 7200), "2 hours ago");
		assert_eq!(relative_long(200, 100), "timestamp: 200");
		assert_eq!(relative_short(0, 90), "1m ago");
		assert_eq!(relative_short(0, 200_000), "2d ago");
		assert_eq!(relative_short(200, 100), "200");
	}
}

#[cfg(test)]
mod export_tests {
	use super::*;

	#[test]
	fn csv_fields_are_quoted_only_when_needed() {
		assert_eq!(csv_field("plain"), "plain");
		assert_eq!(csv_field("a,b"), "\"a,b\"");
		assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
		assert_eq!(csv_row(&["1".into(), "x\ny".into()]), "1,\"x\ny\"");
	}

	#[test]
	fn freshness_reads_naturally() {
		assert_eq!(ago(100, 102), "just now");
		assert_eq!(ago(100, 145), "45s ago");
		assert_eq!(ago(0, 180), "3m ago");
	}

	#[test]
	fn local_datetime_includes_year() {
		assert!(local_datetime(1_790_000_000).contains("2026"));
	}
}

