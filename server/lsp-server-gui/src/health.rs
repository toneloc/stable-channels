//! Pure health rules behind the Overview "needs attention" list and the stable drift column.

use std::collections::HashMap;

use sc_rest_client::ldk_server_grpc::api::{
	GetNodeInfoResponse, ListChannelsResponse, ListPaymentsResponse, ListPeersResponse,
};
use sc_rest_client::sc_protos::stable::{ListStableChannelsResponse, StableChannelInfo};

use crate::format::truncate_id;

/// Mirrors STABILITY_THRESHOLD_PERCENT in src/constants.rs (the daemon's stabilization trigger).
pub const STABILITY_THRESHOLD_PERCENT: f64 = 0.1;
/// Mirrors STABILITY_THRESHOLD_USD in src/constants.rs.
pub const STABILITY_THRESHOLD_USD: f64 = 0.25;
/// Drift this far past the trigger means stabilization is not keeping up (e.g. the wallet is offline).
pub const OFF_TARGET_PERCENT: f64 = 1.0;
/// A usable channel with less than this share of its capacity on one side is flagged.
pub const LOW_LIQUIDITY_RATIO: f64 = 0.10;
/// Wallet syncs older than this are flagged.
pub const SYNC_STALE_SECS: u64 = 30 * 60;
const DAY_SECS: u64 = 86_400;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DriftLevel {
	/// Within the daemon's threshold.
	AtPar,
	/// Above the trigger; the next stability payment should correct it.
	Correcting,
	/// Well beyond the trigger.
	OffTarget,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Drift {
	/// Current USD value of the backing minus the target (positive = above target).
	pub usd: f64,
	pub percent: f64,
	pub value_usd: f64,
	pub level: DriftLevel,
}

/// Drift of a stable position at `price`: backing value vs. its USD target.
pub fn drift(backing_sats: u64, expected_usd: f64, price: f64) -> Option<Drift> {
	if expected_usd <= 0.0 || price <= 0.0 || !price.is_finite() {
		return None;
	}
	let value_usd = backing_sats as f64 / 100_000_000.0 * price;
	let usd = value_usd - expected_usd;
	let percent = usd.abs() / expected_usd * 100.0;
	let level = if percent < STABILITY_THRESHOLD_PERCENT || usd.abs() < STABILITY_THRESHOLD_USD {
		DriftLevel::AtPar
	} else if percent < OFF_TARGET_PERCENT {
		DriftLevel::Correcting
	} else {
		DriftLevel::OffTarget
	};
	Some(Drift { usd, percent, value_usd, level })
}

/// Drift in USD at which the daemon settles this target: at least $0.25 and at least 0.1% of it.
pub fn settle_threshold_usd(expected_usd: f64) -> f64 {
	if expected_usd < 0.01 {
		return STABILITY_THRESHOLD_USD;
	}
	STABILITY_THRESHOLD_USD.max(expected_usd * STABILITY_THRESHOLD_PERCENT / 100.0)
}

/// The next stability step for a stable position.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Settlement {
	/// Inside tolerance; this much more drift (USD) triggers a settlement.
	NotYet { headroom_usd: f64 },
	/// Below target: the LSP sends these sats to the user.
	LspPays { sats: u64 },
	/// Above target: the user's wallet sends these sats to the LSP.
	UserPays { sats: u64 },
}

/// What happens next for `drift` on a position with this target, valued at `price`.
pub fn settlement(drift: &Drift, expected_usd: f64, price: f64) -> Settlement {
	let threshold = settle_threshold_usd(expected_usd);
	if drift.usd.abs() < threshold {
		return Settlement::NotYet { headroom_usd: threshold - drift.usd.abs() };
	}
	let sats = (drift.usd.abs() / price * 100_000_000.0).round() as u64;
	if drift.usd < 0.0 { Settlement::LspPays { sats } } else { Settlement::UserPays { sats } }
}

/// A failed outbound 1-msat keysend is a protocol message (balance sync or trade reply), not a payment.
pub fn is_failed_protocol_message(status: i32, direction: i32, amount_msat: Option<u64>) -> bool {
	status == 2 && direction == 1 && amount_msat == Some(1)
}

/// A stable user is a channel with a USD target; routing peers and bitcoin-only wallets have none.
pub fn has_stable_position(sc: &StableChannelInfo) -> bool {
	sc.expected_usd > 0.0
}

/// Drift of a daemon stable-channel record, using the latest price it reports (or `fallback_price`).
pub fn stable_drift(sc: &StableChannelInfo, fallback_price: Option<f64>) -> Option<Drift> {
	let price = if sc.latest_price > 0.0 { sc.latest_price } else { fallback_price? };
	drift(sc.expected_msats / 1000, sc.expected_usd, price)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
	Danger,
	Warning,
	Info,
}

/// Where the "View" button of an attention item leads.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
	Channel(String),
	/// Channel History of a channel that is no longer listed.
	History(String),
	FailedPayments,
	Peers,
	Balances,
	NodeInfo,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Attention {
	pub severity: Severity,
	pub title: String,
	pub detail: String,
	pub target: Target,
}

/// Everything the overview looks at; all of it is already fetched by the dashboard.
pub struct Snapshot<'a> {
	pub now: u64,
	pub price: Option<f64>,
	pub node_info: Option<&'a GetNodeInfoResponse>,
	pub channels: Option<&'a ListChannelsResponse>,
	pub stable: Option<&'a ListStableChannelsResponse>,
	pub peers: Option<&'a ListPeersResponse>,
	pub payments: Option<&'a ListPaymentsResponse>,
	pub aliases: &'a HashMap<String, Option<String>>,
}

impl Snapshot<'_> {
	fn peer_name(&self, node_id: &str) -> String {
		match self.aliases.get(node_id).cloned().flatten() {
			Some(alias) => alias,
			None => truncate_id(node_id, 8, 6),
		}
	}
}

/// Build the attention list, most severe first.
pub fn attention(s: &Snapshot, fmt_sats: &dyn Fn(u64) -> String) -> Vec<Attention> {
	let mut items = Vec::new();

	if let Some(channels) = s.channels {
		for ch in &channels.channels {
			let peer = s.peer_name(&ch.counterparty_node_id);
			let target = Target::Channel(ch.user_channel_id.clone());
			// Opening and closing channels are normal work (see dashboard::in_progress), not problems.
			if !ch.is_channel_ready || matches!(ch.channel_shutdown_state, Some(2..=5)) {
				continue;
			}
			if !ch.is_usable {
				items.push(Attention {
					severity: Severity::Danger,
					title: format!("Channel with {peer} is not usable"),
					detail: "The channel is open but cannot route payments, usually because the peer is offline."
						.to_string(),
					target,
				});
			} else {
				let total = ch.outbound_capacity_msat + ch.inbound_capacity_msat;
				if total > 0 {
					let out_ratio = ch.outbound_capacity_msat as f64 / total as f64;
					if out_ratio < LOW_LIQUIDITY_RATIO {
						items.push(Attention {
							severity: Severity::Warning,
							title: format!("Low outbound liquidity with {peer}"),
							detail: format!(
								"Only {} of {} can be sent through this channel.",
								fmt_sats(ch.outbound_capacity_msat / 1000),
								fmt_sats(total / 1000)
							),
							target: target.clone(),
						});
					} else if 1.0 - out_ratio < LOW_LIQUIDITY_RATIO {
						items.push(Attention {
							severity: Severity::Info,
							title: format!("Low inbound liquidity with {peer}"),
							detail: format!(
								"Only {} of {} can be received through this channel.",
								fmt_sats(ch.inbound_capacity_msat / 1000),
								fmt_sats(total / 1000)
							),
							target,
						});
					}
				}
			}
		}
	}

	if let Some(stable) = s.stable {
		for sc in &stable.channels {
			let Some(d) = stable_drift(sc, s.price) else { continue };
			if d.level == DriftLevel::OffTarget {
				let direction = if d.usd > 0.0 { "above" } else { "below" };
				items.push(Attention {
					severity: Severity::Danger,
					title: format!("Stable channel with {} is off target", s.peer_name(&sc.counterparty)),
					detail: format!(
						"Backing is worth ${:.2}, {:.2}% ({}${:.2}) {direction} its ${:.2} target.",
						d.value_usd,
						d.percent,
						if d.usd > 0.0 { "+" } else { "-" },
						d.usd.abs(),
						sc.expected_usd
					),
					target: Target::Channel(sc.user_channel_id.clone()),
				});
			}
		}
	}

	if let Some(payments) = s.payments {
		let failed = payments
			.payments
			.iter()
			.filter(|p| p.status == 2 && !is_failed_protocol_message(p.status, p.direction, p.amount_msat))
			.filter(|p| s.now.saturating_sub(p.latest_update_timestamp) < DAY_SECS)
			.count();
		if failed > 0 {
			items.push(Attention {
				severity: Severity::Warning,
				title: format!("{failed} failed payment{} in the last 24 hours", if failed == 1 { "" } else { "s" }),
				detail: "Counted from the payments loaded so far.".to_string(),
				target: Target::FailedPayments,
			});
		}
	}

	if let Some(peers) = s.peers {
		for peer in peers.peers.iter().filter(|p| p.is_persisted && !p.is_connected) {
			items.push(Attention {
				severity: Severity::Warning,
				title: format!("Peer {} is disconnected", s.peer_name(&peer.node_id)),
				detail: format!("Persisted peer at {} is not connected.", peer.address),
				target: Target::Peers,
			});
		}
	}

	if let Some(info) = s.node_info {
		for (label, ts) in [
			("On-chain wallet", info.latest_onchain_wallet_sync_timestamp),
			("Lightning wallet", info.latest_lightning_wallet_sync_timestamp),
		] {
			if let Some(ts) = ts {
				let age = s.now.saturating_sub(ts);
				if age > SYNC_STALE_SECS {
					items.push(Attention {
						severity: Severity::Warning,
						title: format!("{label} has not synced for {} minutes", age / 60),
						detail: "Check the chain source; balances and channel state may be out of date.".to_string(),
						target: Target::NodeInfo,
					});
				}
			}
		}
	}

	items.sort_by_key(|item| item.severity);
	items
}

#[cfg(test)]
mod tests {
	use super::*;
	use sc_rest_client::ldk_server_grpc::types::{Channel, Payment, Peer};

	fn fmt(sats: u64) -> String {
		format!("{sats} sats")
	}

	fn snapshot<'a>(aliases: &'a HashMap<String, Option<String>>) -> Snapshot<'a> {
		Snapshot {
			now: 1_000_000,
			price: Some(100_000.0),
			node_info: None,
			channels: None,
			stable: None,
			peers: None,
			payments: None,
			aliases,
		}
	}

	#[test]
	fn drift_levels_follow_daemon_thresholds() {
		// 50,000 sats at $100k = $50.
		assert_eq!(drift(50_000, 50.0, 100_000.0).unwrap().level, DriftLevel::AtPar);
		// $50.30 vs $50 target: 0.6% and above $0.25 -> correcting.
		assert_eq!(drift(50_300, 50.0, 100_000.0).unwrap().level, DriftLevel::Correcting);
		// $48 vs $50 target: 4% below.
		let d = drift(48_000, 50.0, 100_000.0).unwrap();
		assert_eq!(d.level, DriftLevel::OffTarget);
		assert!(d.usd < 0.0);
		assert!((d.percent - 4.0).abs() < 1e-9);
		// Tiny dollar drift is at par even when the percentage is large.
		assert_eq!(drift(1_200, 1.0, 100_000.0).unwrap().level, DriftLevel::AtPar);
		assert!(drift(1, 0.0, 100_000.0).is_none());
	}

	#[test]
	fn settlements_start_at_the_larger_of_a_quarter_dollar_and_a_tenth_of_a_percent() {
		assert_eq!(settle_threshold_usd(50.0), 0.25);
		assert_eq!(settle_threshold_usd(1_000.0), 1.0);
		assert_eq!(settle_threshold_usd(0.001), 0.25, "dust targets use the dollar floor, like the daemon");
	}

	#[test]
	fn the_next_settlement_follows_the_sign_and_size_of_the_drift() {
		let calm = settlement(&drift(49_900, 50.0, 100_000.0).unwrap(), 50.0, 100_000.0);
		match calm {
			Settlement::NotYet { headroom_usd } => assert!((headroom_usd - 0.15).abs() < 1e-9),
			other => panic!("expected no settlement, got {other:?}"),
		}
		assert_eq!(settlement(&drift(49_700, 50.0, 100_000.0).unwrap(), 50.0, 100_000.0), Settlement::LspPays { sats: 300 });
		assert_eq!(settlement(&drift(50_400, 50.0, 100_000.0).unwrap(), 50.0, 100_000.0), Settlement::UserPays { sats: 400 });
	}

	#[test]
	fn channel_problems_are_flagged_with_peer_names() {
		let aliases: HashMap<String, Option<String>> = [("02aa".to_string(), Some("ACINQ".to_string()))].into();
		let channels = ListChannelsResponse {
			channels: vec![
				Channel { user_channel_id: "1".into(), counterparty_node_id: "02aa".into(), is_channel_ready: true, is_usable: false, ..Default::default() },
				Channel { user_channel_id: "2".into(), counterparty_node_id: "02bb".into(), ..Default::default() },
				Channel {
					user_channel_id: "3".into(),
					counterparty_node_id: "02cc".into(),
					is_channel_ready: true,
					is_usable: true,
					outbound_capacity_msat: 50_000,
					inbound_capacity_msat: 950_000,
					..Default::default()
				},
				Channel {
					user_channel_id: "4".into(),
					counterparty_node_id: "02dd".into(),
					is_channel_ready: true,
					is_usable: true,
					outbound_capacity_msat: 500_000,
					inbound_capacity_msat: 500_000,
					..Default::default()
				},
				// Shutting down: not usable by definition, so not a problem.
				Channel { user_channel_id: "5".into(), counterparty_node_id: "02ee".into(), is_channel_ready: true, channel_shutdown_state: Some(3), ..Default::default() },
			],
		};
		let mut s = snapshot(&aliases);
		s.channels = Some(&channels);
		let items = attention(&s, &fmt);
		assert_eq!(items.len(), 2);
		assert_eq!(items[0].severity, Severity::Danger);
		assert_eq!(items[0].title, "Channel with ACINQ is not usable");
		assert_eq!(items[0].target, Target::Channel("1".into()));
		assert!(items.iter().any(|i| i.title.starts_with("Low outbound liquidity")));
		assert!(!items.iter().any(|i| i.title.contains("is opening")), "opening channels are in-progress work, not problems");
	}

	#[test]
	fn recent_failures_and_offline_peers_are_counted() {
		let aliases = HashMap::new();
		let payments = ListPaymentsResponse {
			payments: vec![
				Payment { status: 2, latest_update_timestamp: 999_000, ..Default::default() },
				Payment { status: 2, latest_update_timestamp: 1, ..Default::default() },
				Payment { status: 1, latest_update_timestamp: 999_000, ..Default::default() },
			],
			next_page_token: None,
		};
		let peers = ListPeersResponse {
			peers: vec![
				Peer { node_id: "02ee".into(), is_persisted: true, is_connected: false, ..Default::default() },
				Peer { node_id: "02ff".into(), is_persisted: false, is_connected: false, ..Default::default() },
			],
		};
		let mut s = snapshot(&aliases);
		s.payments = Some(&payments);
		s.peers = Some(&peers);
		let items = attention(&s, &fmt);
		assert_eq!(items.len(), 2);
		assert!(items.iter().any(|i| i.title == "1 failed payment in the last 24 hours"));
		assert!(items.iter().any(|i| i.target == Target::Peers));
	}

	#[test]
	fn failed_protocol_messages_are_not_failed_payments() {
		assert!(is_failed_protocol_message(2, 1, Some(1)));
		assert!(!is_failed_protocol_message(2, 1, Some(5_000)), "a failed real send is still a failed payment");
		assert!(!is_failed_protocol_message(2, 0, Some(1)));
		assert!(!is_failed_protocol_message(1, 1, Some(1)));
		let aliases = HashMap::new();
		let payments = ListPaymentsResponse {
			payments: vec![
				Payment { status: 2, direction: 1, amount_msat: Some(1), latest_update_timestamp: 999_000, ..Default::default() },
				Payment { status: 2, direction: 1, amount_msat: Some(5_000), latest_update_timestamp: 999_000, ..Default::default() },
			],
			next_page_token: None,
		};
		let mut s = snapshot(&aliases);
		s.payments = Some(&payments);
		let items = attention(&s, &fmt);
		assert!(items.iter().any(|i| i.title == "1 failed payment in the last 24 hours"));
	}

	#[test]
	fn only_a_usd_target_makes_a_stable_user() {
		let routing = StableChannelInfo { user_channel_id: "8".into(), expected_usd: 0.0, ..Default::default() };
		let user = StableChannelInfo { user_channel_id: "9".into(), expected_usd: 43.63, ..Default::default() };
		assert!(!has_stable_position(&routing));
		assert!(has_stable_position(&user));
	}

	#[test]
	fn off_target_stable_channels_are_flagged() {
		let aliases = HashMap::new();
		let stable = ListStableChannelsResponse {
			channels: vec![
				StableChannelInfo { user_channel_id: "9".into(), expected_usd: 50.0, expected_msats: 48_000_000, latest_price: 100_000.0, ..Default::default() },
				StableChannelInfo { user_channel_id: "8".into(), expected_usd: 50.0, expected_msats: 50_000_000, latest_price: 100_000.0, ..Default::default() },
			],
		};
		let mut s = snapshot(&aliases);
		s.stable = Some(&stable);
		let items = attention(&s, &fmt);
		assert_eq!(items.len(), 1);
		assert!(items[0].detail.contains("4.00%"));
		assert!(items[0].detail.contains("below"));
	}
}
