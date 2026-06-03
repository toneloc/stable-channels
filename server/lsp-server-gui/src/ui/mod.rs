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
pub mod tools;

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
