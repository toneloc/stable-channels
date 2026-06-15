pub mod balances;
pub mod channels;
pub mod connection;
pub mod forwarded_payments;
pub mod ldk_log;
pub mod lightning;
pub mod network_graph;
pub mod node_info;
pub mod onchain;
pub mod payments;
pub mod peers;
pub mod stable_channels;
pub mod settings;
pub mod tools;
pub mod widgets;

use crate::state::DisplayUnit;

pub fn truncate_id(s: &str, start: usize, end: usize) -> String {
	if s.len() <= start + end + 2 {
		s.to_string()
	} else {
		format!("{}..{}", &s[..start], &s[s.len() - end..])
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

fn format_usd(amount: f64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DisplayUnit;

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
