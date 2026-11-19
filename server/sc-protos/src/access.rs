// Route permissions and per-session request signing shared by the daemon and the GUI client.

use bitcoin_hashes::{sha256, Hash, HashEngine, Hmac, HmacEngine};

pub const LOGIN_PATH: &str = "Login";
pub const LOGOUT_PATH: &str = "Logout";
pub const WHO_AM_I_PATH: &str = "WhoAmI";
pub const LIST_TOKENS_PATH: &str = "ListTokens";
pub const CREATE_TOKEN_PATH: &str = "CreateToken";
pub const REVOKE_TOKEN_PATH: &str = "RevokeToken";

/// The error text that tells a client its session is gone and it should log in again.
pub const SESSION_UNKNOWN: &str = "session unknown or expired";

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LoginRequest {
	#[prost(string, tag = "1")]
	pub token: ::prost::alloc::string::String,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LoginResponse {
	#[prost(string, tag = "1")]
	pub session_id: ::prost::alloc::string::String,
	/// Hex of the 32-byte session secret.
	#[prost(string, tag = "2")]
	pub session_secret: ::prost::alloc::string::String,
	#[prost(string, tag = "3")]
	pub token_id: ::prost::alloc::string::String,
	#[prost(string, tag = "4")]
	pub token_name: ::prost::alloc::string::String,
	#[prost(string, repeated, tag = "5")]
	pub permissions: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
	#[prost(uint64, tag = "6")]
	pub expires_at: u64,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LogoutRequest {}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LogoutResponse {}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct WhoAmIRequest {}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct WhoAmIResponse {
	#[prost(string, tag = "1")]
	pub token_id: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub token_name: ::prost::alloc::string::String,
	#[prost(string, repeated, tag = "3")]
	pub permissions: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TokenInfo {
	#[prost(string, tag = "1")]
	pub id: ::prost::alloc::string::String,
	#[prost(string, tag = "2")]
	pub name: ::prost::alloc::string::String,
	#[prost(string, repeated, tag = "3")]
	pub permissions: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
	#[prost(string, repeated, tag = "4")]
	pub caveats: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
	#[prost(bool, tag = "5")]
	pub used_by_daemon: bool,
	#[prost(bool, tag = "6")]
	pub is_you: bool,
	#[prost(bool, tag = "7")]
	pub is_gui_token: bool,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListTokensRequest {}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ListTokensResponse {
	#[prost(message, repeated, tag = "1")]
	pub tokens: ::prost::alloc::vec::Vec<TokenInfo>,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CreateTokenRequest {
	#[prost(string, tag = "1")]
	pub name: ::prost::alloc::string::String,
	#[prost(string, repeated, tag = "2")]
	pub permissions: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
	/// Unix seconds when the token stops working; 0 means never.
	#[prost(uint64, tag = "3")]
	pub expires_at: u64,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CreateTokenResponse {
	#[prost(message, optional, tag = "1")]
	pub token: ::core::option::Option<TokenInfo>,
	#[prost(string, tag = "2")]
	pub token_hex: ::prost::alloc::string::String,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RevokeTokenRequest {
	#[prost(string, tag = "1")]
	pub id: ::prost::alloc::string::String,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RevokeTokenResponse {}

/// What a route needs from the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
	/// The session's token must hold this permission (or "admin").
	Permission(&'static str),
	/// Any valid session.
	AnySession,
	/// No session (the route proves itself, like Login).
	NoSession,
}

/// The permission a route needs; None for a route not in the table, which is refused.
pub fn required(route: &str) -> Option<Requirement> {
	use Requirement::{AnySession, NoSession, Permission};
	Some(match route {
		"GetNodeInfo" | "GetBalances" | "ExportPathfindingScores" | "GetPrice" => {
			Permission("node:read")
		},
		"OnchainReceive" => Permission("onchain:receive"),
		"OnchainSend" => Permission("onchain:send"),
		"Bolt11Receive" | "Bolt12Receive" => Permission("invoices:create"),
		"Bolt11Send" | "Bolt12Send" | "SpontaneousSend" | "SpliceOut"
		| "ReleaseStabilityPayment" | "RefundTradeFee" => Permission("payments:send"),
		"GetPaymentDetails" | "ListPayments" | "ListForwardedPayments"
		| "ListSettlementPayments" | "AuditLog" | "GetRevenue" => Permission("payments:read"),
		"ListChannels" | "ListStableChannels" | "ListChannelLedgerEvents"
		| "ListQueuedSplices" => Permission("channels:read"),
		"OpenChannel" | "UpdateChannelConfig" | "CloseChannel" | "SpliceIn"
		| "EditStableChannel" | "QueueSpliceIn" | "CancelQueuedSplice" => {
			Permission("channels:manage")
		},
		"ForceCloseChannel" => Permission("channels:force_close"),
		"ListPeers" => Permission("peers:read"),
		"ConnectPeer" | "DisconnectPeer" => Permission("peers:manage"),
		"SignMessage" => Permission("messages:sign"),
		"VerifySignature" => Permission("messages:verify"),
		"GraphListChannels" | "GraphGetChannel" | "GraphListNodes" | "GraphGetNode" => {
			Permission("graph:read")
		},
		"LdkLog" | "RegisterPush" => Permission("admin"),
		LIST_TOKENS_PATH | CREATE_TOKEN_PATH | REVOKE_TOKEN_PATH => Permission("macaroons:manage"),
		LOGOUT_PATH | WHO_AM_I_PATH => AnySession,
		LOGIN_PATH => NoSession,
		_ => return None,
	})
}

/// True when the list holds the permission or "admin".
pub fn allows(permissions: &[String], permission: &str) -> bool {
	permissions.iter().any(|p| p == permission || p == "admin")
}

fn to_hex(bytes: &[u8]) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let mut s = String::with_capacity(bytes.len() * 2);
	for b in bytes {
		s.push(HEX[(b >> 4) as usize] as char);
		s.push(HEX[(b & 0xf) as usize] as char);
	}
	s
}

fn from_hex_16(s: &str) -> Option<[u8; 16]> {
	if s.len() != 32 || !s.is_ascii() {
		return None;
	}
	let mut out = [0u8; 16];
	for (i, byte) in out.iter_mut().enumerate() {
		*byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
	}
	Some(out)
}

/// Lowercase hex HMAC-SHA256 over route || 0x00 || ts (8-byte BE) || nonce || body.
pub fn session_mac(
	secret: &[u8; 32], route: &str, ts: u64, nonce: &[u8; 16], body: &[u8],
) -> String {
	let mut engine = HmacEngine::<sha256::Hash>::new(secret);
	engine.input(route.as_bytes());
	engine.input(&[0u8]);
	engine.input(&ts.to_be_bytes());
	engine.input(nonce);
	engine.input(body);
	let mac = Hmac::<sha256::Hash>::from_engine(engine);
	to_hex(mac.as_byte_array())
}

/// The X-Auth header value: `SESSION <id>:<ts>:<nonce_hex>:<mac>`.
pub fn session_header(session_id: &str, ts: u64, nonce: &[u8; 16], mac: &str) -> String {
	format!("SESSION {}:{}:{}:{}", session_id, ts, to_hex(nonce), mac)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSession {
	pub session_id: String,
	pub ts: u64,
	pub nonce: [u8; 16],
	pub mac: String,
}

/// Parses a `SESSION` header value; None when it is malformed.
pub fn parse_session_header(h: &str) -> Option<ParsedSession> {
	let rest = h.strip_prefix("SESSION ")?;
	let mut parts = rest.split(':');
	let session_id = parts.next()?;
	let ts = parts.next()?.parse::<u64>().ok()?;
	let nonce = from_hex_16(parts.next()?)?;
	let mac = parts.next()?;
	if parts.next().is_some() || session_id.is_empty() || mac.is_empty() {
		return None;
	}
	Some(ParsedSession { session_id: session_id.to_string(), ts, nonce, mac: mac.to_string() })
}

#[cfg(test)]
mod tests {
	use super::Requirement::{AnySession, NoSession, Permission};
	use super::*;

	#[test]
	fn every_route_in_the_spec_table_has_its_permission() {
		let rows: &[(&str, &str)] = &[
			("GetNodeInfo", "node:read"),
			("GetBalances", "node:read"),
			("ExportPathfindingScores", "node:read"),
			("GetPrice", "node:read"),
			("OnchainReceive", "onchain:receive"),
			("OnchainSend", "onchain:send"),
			("Bolt11Receive", "invoices:create"),
			("Bolt12Receive", "invoices:create"),
			("Bolt11Send", "payments:send"),
			("Bolt12Send", "payments:send"),
			("SpontaneousSend", "payments:send"),
			("SpliceOut", "payments:send"),
			("ReleaseStabilityPayment", "payments:send"),
			("RefundTradeFee", "payments:send"),
			("GetPaymentDetails", "payments:read"),
			("ListPayments", "payments:read"),
			("ListForwardedPayments", "payments:read"),
			("ListSettlementPayments", "payments:read"),
			("AuditLog", "payments:read"),
			("GetRevenue", "payments:read"),
			("ListChannels", "channels:read"),
			("ListStableChannels", "channels:read"),
			("ListChannelLedgerEvents", "channels:read"),
			("ListQueuedSplices", "channels:read"),
			("OpenChannel", "channels:manage"),
			("UpdateChannelConfig", "channels:manage"),
			("CloseChannel", "channels:manage"),
			("SpliceIn", "channels:manage"),
			("EditStableChannel", "channels:manage"),
			("QueueSpliceIn", "channels:manage"),
			("CancelQueuedSplice", "channels:manage"),
			("ForceCloseChannel", "channels:force_close"),
			("ListPeers", "peers:read"),
			("ConnectPeer", "peers:manage"),
			("DisconnectPeer", "peers:manage"),
			("SignMessage", "messages:sign"),
			("VerifySignature", "messages:verify"),
			("GraphListChannels", "graph:read"),
			("GraphGetChannel", "graph:read"),
			("GraphListNodes", "graph:read"),
			("GraphGetNode", "graph:read"),
			("LdkLog", "admin"),
			("RegisterPush", "admin"),
			("ListTokens", "macaroons:manage"),
			("CreateToken", "macaroons:manage"),
			("RevokeToken", "macaroons:manage"),
		];
		for (route, perm) in rows {
			assert_eq!(required(route), Some(Permission(perm)), "{route}");
		}
		assert_eq!(required("ReleaseStabilityPayment"), Some(Permission("payments:send")));
		assert_eq!(required("Logout"), Some(AnySession));
		assert_eq!(required("WhoAmI"), Some(AnySession));
		assert_eq!(required("Login"), Some(NoSession));
		assert_eq!(required("NoSuchRoute"), None);
	}

	#[test]
	fn admin_allows_everything_and_others_only_what_they_list() {
		assert!(allows(&["admin".into()], "payments:send"));
		assert!(allows(&["payments:send".into()], "payments:send"));
		assert!(!allows(&["node:read".into()], "payments:send"));
		assert!(!allows(&[], "node:read"));
	}

	#[test]
	fn the_session_mac_matches_a_fixed_vector() {
		let mac = session_mac(&[7u8; 32], "GetNodeInfo", 1_800_000_000, &[9u8; 16], b"");
		assert_eq!(mac, "dcdcfebcae48a490d0772961a7658290221901b583f17a9144a1b96efc565105");
	}

	#[test]
	fn the_mac_changes_with_route_ts_nonce_and_body() {
		let base = session_mac(&[7u8; 32], "GetNodeInfo", 100, &[9u8; 16], b"x");
		assert_eq!(base, session_mac(&[7u8; 32], "GetNodeInfo", 100, &[9u8; 16], b"x"));
		assert_ne!(base, session_mac(&[8u8; 32], "GetNodeInfo", 100, &[9u8; 16], b"x"));
		assert_ne!(base, session_mac(&[7u8; 32], "GetBalances", 100, &[9u8; 16], b"x"));
		assert_ne!(base, session_mac(&[7u8; 32], "GetNodeInfo", 101, &[9u8; 16], b"x"));
		assert_ne!(base, session_mac(&[7u8; 32], "GetNodeInfo", 100, &[10u8; 16], b"x"));
		assert_ne!(base, session_mac(&[7u8; 32], "GetNodeInfo", 100, &[9u8; 16], b"y"));
	}

	#[test]
	fn a_session_header_round_trips_and_garbage_does_not_parse() {
		let nonce = [0xabu8; 16];
		let mac = session_mac(&[7u8; 32], "GetNodeInfo", 5, &nonce, b"");
		let h = session_header("sess1", 5, &nonce, &mac);
		assert_eq!(h, format!("SESSION sess1:5:{}:{}", "ab".repeat(16), mac));
		let p = parse_session_header(&h).unwrap();
		assert_eq!(p, ParsedSession { session_id: "sess1".into(), ts: 5, nonce, mac });
		assert!(parse_session_header("HMAC 1:ab").is_none());
		assert!(parse_session_header("SESSION s:5:abcd:mac").is_none());
		assert!(parse_session_header(&format!("SESSION s:xx:{}:mac", "ab".repeat(16))).is_none());
		assert!(parse_session_header(&format!("SESSION s:5:{}:mac:extra", "ab".repeat(16)))
			.is_none());
	}
}
