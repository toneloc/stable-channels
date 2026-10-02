//! Operator revenue: what the LSP earned, spent and settled for the peg.

/// Totals for one category over the requested window.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RevenueLine {
	#[prost(string, tag = "1")]
	pub category: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub direction: ::prost::alloc::string::String,
	#[prost(uint64, tag = "3")]
	pub count: u64,
	#[prost(uint64, tag = "4")]
	pub total_msat: u64,
}

/// One money movement; `occurred_at` is unix seconds.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RevenueItem {
	#[prost(int64, tag = "1")]
	pub occurred_at: i64,
	#[prost(string, tag = "2")]
	pub category: ::prost::alloc::string::String,
	#[prost(string, tag = "3")]
	pub direction: ::prost::alloc::string::String,
	#[prost(uint64, tag = "4")]
	pub amount_msat: u64,
	#[prost(string, tag = "5")]
	pub node_id: ::prost::alloc::string::String,
	#[prost(string, tag = "6")]
	pub user_channel_id: ::prost::alloc::string::String,
	#[prost(string, tag = "7")]
	pub payment_id: ::prost::alloc::string::String,
	#[prost(string, tag = "8")]
	pub txid: ::prost::alloc::string::String,
	#[prost(bool, tag = "9")]
	pub approximate_time: bool,
	#[prost(bool, tag = "10")]
	pub trade_rejected: bool,
	/// "", "pending", "succeeded", "failed" or "unknown".
	#[prost(string, tag = "11")]
	pub refund_status: ::prost::alloc::string::String,
	/// Stable per-item key used by the cursor.
	#[prost(string, tag = "12")]
	pub key: ::prost::alloc::string::String,
}

/// `since` is unix seconds (0 = all time); `limit` 0 means the default page size.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRevenueRequest {
	#[prost(int64, tag = "1")]
	pub since: i64,
	#[prost(string, repeated, tag = "2")]
	pub categories: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
	#[prost(string, optional, tag = "3")]
	pub cursor: ::core::option::Option<::prost::alloc::string::String>,
	#[prost(uint32, tag = "4")]
	pub limit: u32,
}

/// `lines` cover the whole window; `items` honour the category filter and cursor.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRevenueResponse {
	#[prost(message, repeated, tag = "1")]
	pub lines: ::prost::alloc::vec::Vec<RevenueLine>,
	#[prost(message, repeated, tag = "2")]
	pub items: ::prost::alloc::vec::Vec<RevenueItem>,
	#[prost(string, optional, tag = "3")]
	pub next_cursor: ::core::option::Option<::prost::alloc::string::String>,
	#[prost(int64, tag = "4")]
	pub snapshot_at: i64,
	/// Totals for this window miss older history: a capped payment scan, dropped items or pruned forwards.
	#[prost(bool, tag = "5")]
	pub partial: bool,
	/// Categories the node cannot report; show them as not tracked rather than zero.
	#[prost(string, repeated, tag = "6")]
	pub untracked: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
	/// How many items the snapshot holds in total (the newest `REVENUE_MAX_ITEMS` at most).
	#[prost(uint64, tag = "7")]
	pub item_count: u64,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RefundTradeFeeRequest {
	#[prost(string, tag = "1")]
	pub trade_payment_id: ::prost::alloc::string::String,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RefundTradeFeeResponse {
	#[prost(string, tag = "1")]
	pub refund_payment_id: ::prost::alloc::string::String,
	#[prost(uint64, tag = "2")]
	pub amount_msat: u64,
}

pub const GET_REVENUE_PATH: &str = "GetRevenue";
pub const REFUND_TRADE_FEE_PATH: &str = "RefundTradeFee";


#[cfg(test)]
mod tests {
	use super::*;
	use prost::Message;

	#[test]
	fn revenue_messages_round_trip() {
		let response = GetRevenueResponse {
			lines: vec![RevenueLine { category: "trade_fee".into(), direction: "in".into(), count: 2, total_msat: 2_000_000 }],
			items: vec![RevenueItem {
				occurred_at: 1_790_000_000,
				category: "trade_fee".into(),
				direction: "in".into(),
				amount_msat: 1_000_000,
				node_id: "02ab".into(),
				user_channel_id: "u".into(),
				payment_id: "p1".into(),
				txid: String::new(),
				approximate_time: false,
				trade_rejected: true,
				refund_status: "failed".into(),
				key: "p1".into(),
			}],
			next_cursor: Some("1790000000:p1".into()),
			snapshot_at: 1_790_000_060,
			partial: false,
			untracked: vec!["onchain_fee".into()],
			item_count: 1,
		};
		assert_eq!(GetRevenueResponse::decode(response.encode_to_vec().as_slice()).unwrap(), response);
		let refund = RefundTradeFeeResponse { refund_payment_id: "r1".into(), amount_msat: 1_000_000 };
		assert_eq!(RefundTradeFeeResponse::decode(refund.encode_to_vec().as_slice()).unwrap(), refund);
	}

	#[test]
	fn an_empty_request_means_all_time_everything_default_page() {
		let request = GetRevenueRequest::decode(&[][..]).unwrap();
		assert_eq!((request.since, request.categories.len(), request.cursor, request.limit), (0, 0, None, 0));
	}
}
