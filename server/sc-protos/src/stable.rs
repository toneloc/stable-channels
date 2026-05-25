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
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LogResponse {
	#[prost(string, tag = "1")]
	pub content: ::prost::alloc::string::String,
}

// SC-specific REST route paths.
pub const GET_PRICE_PATH: &str = "GetPrice";
pub const LIST_STABLE_CHANNELS_PATH: &str = "ListStableChannels";
pub const EDIT_STABLE_CHANNEL_PATH: &str = "EditStableChannel";
pub const REGISTER_PUSH_PATH: &str = "RegisterPush";
pub const AUDIT_LOG_PATH: &str = "AuditLog";
pub const LDK_LOG_PATH: &str = "LdkLog";
