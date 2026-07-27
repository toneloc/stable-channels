// Stable Channels protobuf types (manually written prost structs)

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetPriceRequest {}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetPriceResponse {
	#[prost(double, tag = "1")]
	pub price: f64,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct StableChannelInfo {
	#[prost(string, tag = "1")]
	pub channel_id: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub counterparty: ::prost::alloc::string::String,
	#[prost(double, tag = "3")]
	pub expected_usd: f64,
	#[prost(uint64, tag = "4")]
	pub expected_msats: u64,
	#[prost(double, tag = "5")]
	pub latest_price: f64,
	#[prost(string, tag = "6")]
	pub note: ::prost::alloc::string::String,
	#[prost(bool, tag = "7")]
	pub is_stable_receiver: bool,
	#[prost(string, tag = "8")]
	pub user_channel_id: ::prost::alloc::string::String,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListStableChannelsRequest {}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListStableChannelsResponse {
	#[prost(message, repeated, tag = "1")]
	pub channels: ::prost::alloc::vec::Vec<StableChannelInfo>,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EditStableChannelRequest {
	#[prost(string, tag = "1")]
	pub channel_id: ::prost::alloc::string::String,
	#[prost(double, optional, tag = "2")]
	pub expected_usd: ::core::option::Option<f64>,
	#[prost(string, optional, tag = "3")]
	pub note: ::core::option::Option<::prost::alloc::string::String>,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EditStableChannelResponse {
	#[prost(bool, tag = "1")]
	pub ok: bool,
	#[prost(string, tag = "2")]
	pub status: ::prost::alloc::string::String,
}

// RegisterPush
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RegisterPushRequest {
	#[prost(string, tag = "1")]
	pub token: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub platform: ::prost::alloc::string::String,
	#[prost(string, tag = "3")]
	pub node_id: ::prost::alloc::string::String,
	#[prost(string, tag = "4")]
	pub environment: ::prost::alloc::string::String,
	#[prost(string, tag = "5")]
	pub signature: ::prost::alloc::string::String,
	#[prost(uint64, tag = "6")]
	pub timestamp: u64,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RegisterPushResponse {
	#[prost(bool, tag = "1")]
	pub ok: bool,
}

// Log endpoints
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LogRequest {
	#[prost(uint32, tag = "1")]
	pub max_lines: u32,
	#[prost(string, tag = "2")]
	pub filter: ::prost::alloc::string::String,
	#[prost(bool, tag = "3")]
	pub full: bool,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LogResponse {
	#[prost(string, tag = "1")]
	pub content: ::prost::alloc::string::String,
}

// Authoritative channel ledger
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LedgerRef {
	#[prost(string, tag = "1")]
	pub role: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub value: ::prost::alloc::string::String,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AccountingSnapshot {
	#[prost(double, optional, tag = "1")]
	pub expected_usd: ::core::option::Option<f64>,
	#[prost(uint64, optional, tag = "2")]
	pub backing_sats: ::core::option::Option<u64>,
	#[prost(uint64, optional, tag = "3")]
	pub native_sats: ::core::option::Option<u64>,
	#[prost(uint64, optional, tag = "4")]
	pub live_receiver_sats: ::core::option::Option<u64>,
	#[prost(double, optional, tag = "5")]
	pub btc_price: ::core::option::Option<f64>,
	#[prost(uint64, optional, tag = "6")]
	pub amount_sats: ::core::option::Option<u64>,
	#[prost(uint64, optional, tag = "7")]
	pub amount_msat: ::core::option::Option<u64>,
	#[prost(double, optional, tag = "8")]
	pub amount_usd: ::core::option::Option<f64>,
	#[prost(uint64, optional, tag = "9")]
	pub fee_sats: ::core::option::Option<u64>,
	#[prost(uint64, optional, tag = "10")]
	pub fee_msat: ::core::option::Option<u64>,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ChannelLedgerEvent {
	#[prost(int64, tag = "1")]
	pub id: i64,
	#[prost(string, tag = "2")]
	pub event_type: ::prost::alloc::string::String,
	#[prost(string, tag = "3")]
	pub category: ::prost::alloc::string::String,
	#[prost(string, tag = "4")]
	pub severity: ::prost::alloc::string::String,
	#[prost(string, tag = "5")]
	pub status: ::prost::alloc::string::String,
	#[prost(string, tag = "6")]
	pub source: ::prost::alloc::string::String,
	#[prost(string, tag = "7")]
	pub completeness: ::prost::alloc::string::String,
	#[prost(int64, tag = "8")]
	pub occurred_at_ms: i64,
	#[prost(int64, tag = "9")]
	pub recorded_at_ms: i64,
	#[prost(string, optional, tag = "10")]
	pub dedup_key: ::core::option::Option<::prost::alloc::string::String>,
	#[prost(message, optional, tag = "11")]
	pub before: ::core::option::Option<AccountingSnapshot>,
	#[prost(message, optional, tag = "12")]
	pub after: ::core::option::Option<AccountingSnapshot>,
	#[prost(string, tag = "13")]
	pub detail_json: ::prost::alloc::string::String,
	#[prost(message, repeated, tag = "14")]
	pub refs: ::prost::alloc::vec::Vec<LedgerRef>,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListChannelLedgerEventsRequest {
	/// Exact match against any stored reference value; empty matches all.
	#[prost(string, tag = "1")]
	pub identifier: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub category: ::prost::alloc::string::String,
	#[prost(string, tag = "3")]
	pub status: ::prost::alloc::string::String,
	#[prost(string, tag = "4")]
	pub completeness: ::prost::alloc::string::String,
	/// Opaque cursor returned by the previous page; empty starts at newest.
	#[prost(string, tag = "5")]
	pub cursor: ::prost::alloc::string::String,
	#[prost(uint32, tag = "6")]
	pub page_size: u32,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListChannelLedgerEventsResponse {
	/// Chronological inside this newest-selected page.
	#[prost(message, repeated, tag = "1")]
	pub events: ::prost::alloc::vec::Vec<ChannelLedgerEvent>,
	#[prost(string, optional, tag = "2")]
	pub next_cursor: ::core::option::Option<::prost::alloc::string::String>,
}

// Settlement payments
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListSettlementPaymentsRequest {}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SettlementPayment {
	#[prost(string, tag = "1")]
	pub payment_id: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub kind: ::prost::alloc::string::String,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListSettlementPaymentsResponse {
	#[prost(message, repeated, tag = "1")]
	pub settlements: ::prost::alloc::vec::Vec<SettlementPayment>,
}

// SC-specific REST route paths.
pub const GET_PRICE_PATH: &str = "GetPrice";
pub const LIST_STABLE_CHANNELS_PATH: &str = "ListStableChannels";
pub const EDIT_STABLE_CHANNEL_PATH: &str = "EditStableChannel";
pub const REGISTER_PUSH_PATH: &str = "RegisterPush";
pub const AUDIT_LOG_PATH: &str = "AuditLog";
pub const LDK_LOG_PATH: &str = "LdkLog";
pub const LIST_SETTLEMENT_PAYMENTS_PATH: &str = "ListSettlementPayments";
pub const LIST_CHANNEL_LEDGER_EVENTS_PATH: &str = "ListChannelLedgerEvents";

#[cfg(test)]
mod tests {
	use super::*;
	use prost::Message;

	#[test]
	fn settlement_messages_roundtrip() {
		let resp = ListSettlementPaymentsResponse {
			settlements: vec![
				SettlementPayment { payment_id: "pay_a".into(), kind: "stability".into() },
				SettlementPayment { payment_id: "pay_b".into(), kind: "sync".into() },
			],
		};
		let bytes = resp.encode_to_vec();
		let decoded = ListSettlementPaymentsResponse::decode(&bytes[..]).unwrap();
		assert_eq!(decoded, resp);
		assert_eq!(decoded.settlements[0].kind, "stability");
	}

	#[test]
	fn ledger_messages_roundtrip() {
		let response = ListChannelLedgerEventsResponse {
			events: vec![ChannelLedgerEvent {
				id: 7,
				event_type: "SYNC_V1_APPLIED".into(),
				category: "stability".into(),
				severity: "info".into(),
				status: "completed".into(),
				source: "signed_sync".into(),
				completeness: "observed".into(),
				occurred_at_ms: 1,
				recorded_at_ms: 2,
				dedup_key: Some("sync:1".into()),
				before: None,
				after: Some(AccountingSnapshot { backing_sats: Some(4), native_sats: Some(2), live_receiver_sats: Some(6), ..Default::default() }),
				detail_json: "{}".into(),
				refs: vec![LedgerRef { role: "user_channel_id".into(), value: "42".into() }],
			}],
			next_cursor: Some("7".into()),
		};
		let decoded = ListChannelLedgerEventsResponse::decode(response.encode_to_vec().as_slice()).unwrap();
		assert_eq!(decoded, response);
	}
}
