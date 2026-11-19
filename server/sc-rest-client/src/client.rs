use std::sync::Mutex;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
use js_sys;

use ldk_server_grpc::api::{
	Bolt11ClaimForIdRequest, Bolt11ClaimForIdResponse, Bolt11FailForIdRequest,
	Bolt11FailForIdResponse, Bolt11ReceiveForHashRequest, Bolt11ReceiveForHashResponse,
	Bolt11ReceiveRequest, Bolt11ReceiveResponse, Bolt11ReceiveVariableAmountViaJitChannelRequest,
	Bolt11ReceiveVariableAmountViaJitChannelResponse, Bolt11ReceiveViaJitChannelRequest,
	Bolt11ReceiveViaJitChannelResponse, Bolt11SendRequest, Bolt11SendResponse,
	Bolt12ReceiveRequest, Bolt12ReceiveResponse, Bolt12SendRequest, Bolt12SendResponse,
	CloseChannelRequest, CloseChannelResponse, ConnectPeerRequest, ConnectPeerResponse,
	DisconnectPeerRequest, DisconnectPeerResponse, ExportPathfindingScoresRequest,
	ExportPathfindingScoresResponse, ForceCloseChannelRequest, ForceCloseChannelResponse,
	GetBalancesRequest, GetBalancesResponse, GetNodeInfoRequest, GetNodeInfoResponse,
	GetPaymentDetailsRequest, GetPaymentDetailsResponse, GraphGetChannelRequest,
	GraphGetChannelResponse, GraphGetNodeRequest, GraphGetNodeResponse, GraphListChannelsRequest,
	GraphListChannelsResponse, GraphListNodesRequest, GraphListNodesResponse, ListChannelsRequest,
	ListChannelsResponse, ListForwardedPaymentsRequest, ListForwardedPaymentsResponse,
	ListPaymentsRequest, ListPaymentsResponse, ListPeersRequest, ListPeersResponse,
	OnchainReceiveRequest, OnchainReceiveResponse, OnchainSendRequest, OnchainSendResponse,
	OpenChannelRequest, OpenChannelResponse, SignMessageRequest, SignMessageResponse,
	SpliceInRequest, SpliceInResponse, SpliceOutRequest, SpliceOutResponse, SpontaneousSendRequest,
	SpontaneousSendResponse, UnifiedSendRequest, UnifiedSendResponse, UpdateChannelConfigRequest,
	UpdateChannelConfigResponse, VerifySignatureRequest, VerifySignatureResponse,
};
use ldk_server_grpc::endpoints::{
	BOLT11_CLAIM_FOR_ID_PATH, BOLT11_FAIL_FOR_ID_PATH, BOLT11_RECEIVE_FOR_HASH_PATH,
	BOLT11_RECEIVE_PATH, BOLT11_RECEIVE_VARIABLE_AMOUNT_VIA_JIT_CHANNEL_PATH,
	BOLT11_RECEIVE_VIA_JIT_CHANNEL_PATH, BOLT11_SEND_PATH, BOLT12_RECEIVE_PATH, BOLT12_SEND_PATH,
	CLOSE_CHANNEL_PATH, CONNECT_PEER_PATH, DISCONNECT_PEER_PATH, EXPORT_PATHFINDING_SCORES_PATH,
	FORCE_CLOSE_CHANNEL_PATH, GET_BALANCES_PATH, GET_NODE_INFO_PATH, GET_PAYMENT_DETAILS_PATH,
	GRAPH_GET_CHANNEL_PATH, GRAPH_GET_NODE_PATH, GRAPH_LIST_CHANNELS_PATH, GRAPH_LIST_NODES_PATH,
	LIST_CHANNELS_PATH, LIST_FORWARDED_PAYMENTS_PATH, LIST_PAYMENTS_PATH, LIST_PEERS_PATH,
	ONCHAIN_RECEIVE_PATH, ONCHAIN_SEND_PATH, OPEN_CHANNEL_PATH, SIGN_MESSAGE_PATH, SPLICE_IN_PATH,
	SPLICE_OUT_PATH, SPONTANEOUS_SEND_PATH, UNIFIED_SEND_PATH, UPDATE_CHANNEL_CONFIG_PATH,
	VERIFY_SIGNATURE_PATH,
};
use ldk_server_grpc::error::{ErrorCode, ErrorResponse};
use sc_protos::stable::{
	CancelQueuedSpliceRequest, CancelQueuedSpliceResponse, EditStableChannelRequest,
	EditStableChannelResponse, GetPriceRequest, GetPriceResponse, ListChannelLedgerEventsRequest,
	ListChannelLedgerEventsResponse, ListQueuedSplicesRequest, ListQueuedSplicesResponse,
	ListSettlementPaymentsRequest, ListSettlementPaymentsResponse, ListStableChannelsRequest,
	ListStableChannelsResponse, LogRequest, LogResponse, QueueSpliceInRequest,
	QueueSpliceInResponse, ReleaseStabilityPaymentRequest, ReleaseStabilityPaymentResponse,
	AUDIT_LOG_PATH, CANCEL_QUEUED_SPLICE_PATH, EDIT_STABLE_CHANNEL_PATH, GET_PRICE_PATH,
	LDK_LOG_PATH, LIST_CHANNEL_LEDGER_EVENTS_PATH, LIST_QUEUED_SPLICES_PATH,
	LIST_SETTLEMENT_PAYMENTS_PATH, LIST_STABLE_CHANNELS_PATH, QUEUE_SPLICE_IN_PATH,
	RELEASE_STABILITY_PAYMENT_PATH,
};
use sc_protos::access::{
	session_header, session_mac, CreateTokenRequest, CreateTokenResponse, ListTokensRequest,
	ListTokensResponse, LoginRequest, LoginResponse, LogoutRequest, LogoutResponse,
	RevokeTokenRequest, RevokeTokenResponse, WhoAmIRequest, WhoAmIResponse, CREATE_TOKEN_PATH,
	LIST_TOKENS_PATH, LOGIN_PATH, LOGOUT_PATH, REVOKE_TOKEN_PATH, SESSION_UNKNOWN, WHO_AM_I_PATH,
};
use sc_protos::revenue::{
	GetRevenueRequest, GetRevenueResponse, RefundTradeFeeRequest, RefundTradeFeeResponse,
	GET_REVENUE_PATH, REFUND_TRADE_FEE_PATH,
};
use prost::Message;
use reqwest::header::CONTENT_TYPE;
#[cfg(not(target_arch = "wasm32"))]
use reqwest::Certificate;
use reqwest::Client;

use crate::error::LspRestError;
use crate::error::LspRestErrorCode::{
	AuthError, InternalError, InternalServerError, InvalidRequestError, LightningError,
};

const APPLICATION_OCTET_STREAM: &str = "application/octet-stream";

/// Client to access a hosted instance of the Stable Channels LSP daemon.
///
/// The client requires the server's TLS certificate to be provided for verification.
/// This certificate can be found at `<server_storage_dir>/tls.crt` after the
/// server generates it on first startup.
pub struct LspRestClient {
	base_url: String,
	client: Client,
	token: String,
	/// If true, use relative URLs (for WASM behind reverse proxy)
	use_relative_urls: bool,
	session: Mutex<Option<Session>>,
	/// Lets one login run at a time so concurrent requests share it.
	login_gate: LoginGate,
	/// Permissions and token name from the last Login or WhoAmI.
	identity: Mutex<(Vec<String>, String)>,
	/// The last Login response, returned when a caller finds a sibling already logged in.
	last_login: Mutex<Option<LoginResponse>>,
}

/// Async lock that serialises logins without blocking a thread (works on wasm too).
#[derive(Default)]
struct LoginGate(futures_util::lock::Mutex<()>);

/// A logged-in session: its id and the 32-byte secret that signs each request.
#[derive(Clone)]
struct Session {
	id: String,
	secret: [u8; 32],
}

/// True only when the daemon says the session is gone, the one error a fresh login fixes.
pub fn needs_relogin(e: &LspRestError) -> bool {
	e.error_code == AuthError && e.message == SESSION_UNKNOWN
}

/// Builds the `X-Auth` header for one request from the session, route, time, nonce and exact body.
fn build_auth_header(
	session: &Session, route: &str, ts: u64, nonce: &[u8; 16], body: &[u8],
) -> String {
	let mac = session_mac(&session.secret, route, ts, nonce, body);
	session_header(&session.id, ts, nonce, &mac)
}

/// Returns the stored session, logging in first only if none is stored or it is still the one the caller saw fail.
async fn ensure_session<F, Fut>(
	gate: &LoginGate, store: &Mutex<Option<Session>>, seen: Option<&str>, login: F,
) -> Result<Session, LspRestError>
where
	F: FnOnce() -> Fut,
	Fut: std::future::Future<Output = Result<Session, LspRestError>>,
{
	let _turn = gate.0.lock().await;
	let current = store.lock().unwrap().clone();
	if let Some(c) = current {
		if Some(c.id.as_str()) != seen {
			return Ok(c);
		}
	}
	let fresh = login().await?;
	*store.lock().unwrap() = Some(fresh.clone());
	Ok(fresh)
}

fn now_secs() -> u64 {
	#[cfg(not(target_arch = "wasm32"))]
	{
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.expect("System time should be after Unix epoch")
			.as_secs()
	}
	#[cfg(target_arch = "wasm32")]
	{
		(js_sys::Date::now() / 1000.0) as u64
	}
}

fn decode_secret(hex: &str) -> Option<[u8; 32]> {
	if hex.len() != 64 || !hex.is_ascii() {
		return None;
	}
	let mut out = [0u8; 32];
	for (i, byte) in out.iter_mut().enumerate() {
		*byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
	}
	Some(out)
}

impl LspRestClient {
	/// Constructs a [`LspRestClient`] using `base_url` as the SC daemon REST endpoint.
	///
	/// `base_url` should not include the scheme, e.g., `localhost:3000`.
	/// `token` is the LDK token (hex) the client logs in with; the session it gets signs each request.
	/// `server_cert_pem` is the server's TLS certificate in PEM format. This can be
	/// found at `<server_storage_dir>/tls.crt` after the server starts.
	///
	/// Note: On WASM targets, the certificate parameter is ignored as the browser
	/// handles TLS verification.
	#[cfg(not(target_arch = "wasm32"))]
	pub fn new(base_url: String, token: String, server_cert_pem: &[u8]) -> Result<Self, String> {
		let cert = Certificate::from_pem(server_cert_pem)
			.map_err(|e| format!("Failed to parse server certificate: {e}"))?;

		let client = Client::builder()
			.add_root_certificate(cert)
			.connect_timeout(std::time::Duration::from_secs(10))
			.timeout(std::time::Duration::from_secs(60))
			.build()
			.map_err(|e| format!("Failed to build HTTP client: {e}"))?;

		Ok(Self::assemble(base_url, client, token, false))
	}

	/// Constructs a [`LspRestClient`] for WASM targets.
	///
	/// `base_url` should not include the scheme, e.g., `localhost:3000`.
	/// `token` is the LDK token (hex) the client logs in with; the session it gets signs each request.
	///
	/// On WASM, the browser handles TLS verification automatically.
	#[cfg(target_arch = "wasm32")]
	pub fn new(base_url: String, token: String, _server_cert_pem: &[u8]) -> Result<Self, String> {
		let client =
			Client::builder().build().map_err(|e| format!("Failed to build HTTP client: {e}"))?;

		// WASM builds default to relative URLs for reverse proxy setups
		Ok(Self::assemble(base_url, client, token, true))
	}

	/// Builds the full URL for an API endpoint.
	fn build_url(&self, path: &str) -> String {
		if self.use_relative_urls {
			// reqwest rejects relative URLs (unlike browser fetch), so resolve
			// /api/<path> against the page's own origin — the origin's reverse
			// proxy forwards it to the daemon.
			#[cfg(target_arch = "wasm32")]
			let origin = web_sys::window()
				.and_then(|w| w.location().origin().ok())
				.unwrap_or_default();
			#[cfg(not(target_arch = "wasm32"))]
			let origin = String::new();
			format!("{origin}/api/{path}")
		} else {
			format!("https://{}/{path}", self.base_url)
		}
	}

	fn assemble(base_url: String, client: Client, token: String, use_relative_urls: bool) -> Self {
		Self {
			base_url,
			client,
			token,
			use_relative_urls,
			session: Mutex::new(None),
			login_gate: LoginGate::default(),
			identity: Mutex::new((Vec::new(), String::new())),
			last_login: Mutex::new(None),
		}
	}

	/// Logs in with the token and keeps the session; concurrent calls share one Login, and a refusal comes back as the daemon sent it.
	pub async fn login(&self) -> Result<LoginResponse, LspRestError> {
		let seen = self.session.lock().unwrap().as_ref().map(|c| c.id.clone());
		self.ensure(seen.as_deref()).await?;
		let last = self.last_login.lock().unwrap().clone();
		last.ok_or_else(|| LspRestError::new(InternalError, "No login response is stored"))
	}

	async fn ensure(&self, seen: Option<&str>) -> Result<Session, LspRestError> {
		ensure_session(&self.login_gate, &self.session, seen, || self.do_login()).await
	}

	async fn do_login(&self) -> Result<Session, LspRestError> {
		let request = LoginRequest { token: self.token.clone() };
		let response: LoginResponse =
			self.send_bytes(request.encode_to_vec(), LOGIN_PATH, None).await?;
		let secret = decode_secret(&response.session_secret).ok_or_else(|| {
			LspRestError::new(InternalError, "Login answered with a bad session secret")
		})?;
		*self.identity.lock().unwrap() = (response.permissions.clone(), response.token_name.clone());
		let session = Session { id: response.session_id.clone(), secret };
		*self.last_login.lock().unwrap() = Some(response);
		Ok(session)
	}

	/// Permissions of the token, from the last Login or WhoAmI.
	pub fn permissions(&self) -> Vec<String> {
		self.identity.lock().unwrap().0.clone()
	}

	/// Name of the token, from the last Login or WhoAmI.
	pub fn token_name(&self) -> String {
		self.identity.lock().unwrap().1.clone()
	}

	/// Ends the session on the daemon and forgets it here.
	pub async fn logout(&self) -> Result<LogoutResponse, LspRestError> {
		let result = self.post_request(&LogoutRequest {}, LOGOUT_PATH).await;
		*self.session.lock().unwrap() = None;
		result
	}

	/// Who the daemon thinks this session is; refreshes the stored name and permissions.
	pub async fn who_am_i(&self) -> Result<WhoAmIResponse, LspRestError> {
		let response: WhoAmIResponse = self.post_request(&WhoAmIRequest {}, WHO_AM_I_PATH).await?;
		*self.identity.lock().unwrap() = (response.permissions.clone(), response.token_name.clone());
		Ok(response)
	}

	/// Lists the daemon's tokens.
	pub async fn list_tokens(
		&self, request: ListTokensRequest,
	) -> Result<ListTokensResponse, LspRestError> {
		self.post_request(&request, LIST_TOKENS_PATH).await
	}

	/// Creates a token with the given name, permissions and expiry.
	pub async fn create_token(
		&self, request: CreateTokenRequest,
	) -> Result<CreateTokenResponse, LspRestError> {
		self.post_request(&request, CREATE_TOKEN_PATH).await
	}

	/// Revokes a token by id.
	pub async fn revoke_token(
		&self, request: RevokeTokenRequest,
	) -> Result<RevokeTokenResponse, LspRestError> {
		self.post_request(&request, REVOKE_TOKEN_PATH).await
	}

	/// Retrieve the latest node info like `node_id`, `current_best_block` etc.
	/// For API contract/usage, refer to docs for [`GetNodeInfoRequest`] and [`GetNodeInfoResponse`].
	pub async fn get_node_info(
		&self, request: GetNodeInfoRequest,
	) -> Result<GetNodeInfoResponse, LspRestError> {
		self.post_request(&request, GET_NODE_INFO_PATH).await
	}

	/// Retrieves an overview of all known balances.
	/// For API contract/usage, refer to docs for [`GetBalancesRequest`] and [`GetBalancesResponse`].
	pub async fn get_balances(
		&self, request: GetBalancesRequest,
	) -> Result<GetBalancesResponse, LspRestError> {
		self.post_request(&request, GET_BALANCES_PATH).await
	}

	/// Retrieve a new on-chain funding address.
	/// For API contract/usage, refer to docs for [`OnchainReceiveRequest`] and [`OnchainReceiveResponse`].
	pub async fn onchain_receive(
		&self, request: OnchainReceiveRequest,
	) -> Result<OnchainReceiveResponse, LspRestError> {
		self.post_request(&request, ONCHAIN_RECEIVE_PATH).await
	}

	/// Send an on-chain payment to the given address.
	/// For API contract/usage, refer to docs for [`OnchainSendRequest`] and [`OnchainSendResponse`].
	pub async fn onchain_send(
		&self, request: OnchainSendRequest,
	) -> Result<OnchainSendResponse, LspRestError> {
		self.post_request(&request, ONCHAIN_SEND_PATH).await
	}

	/// Retrieve a new BOLT11 payable invoice.
	/// For API contract/usage, refer to docs for [`Bolt11ReceiveRequest`] and [`Bolt11ReceiveResponse`].
	pub async fn bolt11_receive(
		&self, request: Bolt11ReceiveRequest,
	) -> Result<Bolt11ReceiveResponse, LspRestError> {
		self.post_request(&request, BOLT11_RECEIVE_PATH).await
	}

	/// Retrieve a new BOLT11 payable invoice for a given payment hash.
	/// The inbound payment will NOT be automatically claimed upon arrival.
	/// For API contract/usage, refer to docs for [`Bolt11ReceiveForHashRequest`] and [`Bolt11ReceiveForHashResponse`].
	pub async fn bolt11_receive_for_hash(
		&self, request: Bolt11ReceiveForHashRequest,
	) -> Result<Bolt11ReceiveForHashResponse, LspRestError> {
		self.post_request(&request, BOLT11_RECEIVE_FOR_HASH_PATH).await
	}

	/// Manually claim a claimable payment by its payment ID.
	/// For API contract/usage, refer to docs for [`Bolt11ClaimForIdRequest`] and [`Bolt11ClaimForIdResponse`].
	pub async fn bolt11_claim_for_id(
		&self, request: Bolt11ClaimForIdRequest,
	) -> Result<Bolt11ClaimForIdResponse, LspRestError> {
		self.post_request(&request, BOLT11_CLAIM_FOR_ID_PATH).await
	}

	/// Manually fail a claimable payment by its payment ID.
	/// For API contract/usage, refer to docs for [`Bolt11FailForIdRequest`] and [`Bolt11FailForIdResponse`].
	pub async fn bolt11_fail_for_id(
		&self, request: Bolt11FailForIdRequest,
	) -> Result<Bolt11FailForIdResponse, LspRestError> {
		self.post_request(&request, BOLT11_FAIL_FOR_ID_PATH).await
	}

	/// Retrieve a new fixed-amount BOLT11 invoice for receiving via an LSPS2 JIT channel.
	/// For API contract/usage, refer to docs for [`Bolt11ReceiveViaJitChannelRequest`] and
	/// [`Bolt11ReceiveViaJitChannelResponse`].
	pub async fn bolt11_receive_via_jit_channel(
		&self, request: Bolt11ReceiveViaJitChannelRequest,
	) -> Result<Bolt11ReceiveViaJitChannelResponse, LspRestError> {
		self.post_request(&request, BOLT11_RECEIVE_VIA_JIT_CHANNEL_PATH).await
	}

	/// Retrieve a new variable-amount BOLT11 invoice for receiving via an LSPS2 JIT channel.
	/// For API contract/usage, refer to docs for
	/// [`Bolt11ReceiveVariableAmountViaJitChannelRequest`] and
	/// [`Bolt11ReceiveVariableAmountViaJitChannelResponse`].
	pub async fn bolt11_receive_variable_amount_via_jit_channel(
		&self, request: Bolt11ReceiveVariableAmountViaJitChannelRequest,
	) -> Result<Bolt11ReceiveVariableAmountViaJitChannelResponse, LspRestError> {
		self.post_request(&request, BOLT11_RECEIVE_VARIABLE_AMOUNT_VIA_JIT_CHANNEL_PATH).await
	}

	/// Send a payment for a BOLT11 invoice.
	/// For API contract/usage, refer to docs for [`Bolt11SendRequest`] and [`Bolt11SendResponse`].
	pub async fn bolt11_send(
		&self, request: Bolt11SendRequest,
	) -> Result<Bolt11SendResponse, LspRestError> {
		self.post_request(&request, BOLT11_SEND_PATH).await
	}

	/// Retrieve a new BOLT11 payable offer.
	/// For API contract/usage, refer to docs for [`Bolt12ReceiveRequest`] and [`Bolt12ReceiveResponse`].
	pub async fn bolt12_receive(
		&self, request: Bolt12ReceiveRequest,
	) -> Result<Bolt12ReceiveResponse, LspRestError> {
		self.post_request(&request, BOLT12_RECEIVE_PATH).await
	}

	/// Send a payment for a BOLT12 offer.
	/// For API contract/usage, refer to docs for [`Bolt12SendRequest`] and [`Bolt12SendResponse`].
	pub async fn bolt12_send(
		&self, request: Bolt12SendRequest,
	) -> Result<Bolt12SendResponse, LspRestError> {
		self.post_request(&request, BOLT12_SEND_PATH).await
	}

	/// Creates a new outbound channel.
	/// For API contract/usage, refer to docs for [`OpenChannelRequest`] and [`OpenChannelResponse`].
	pub async fn open_channel(
		&self, request: OpenChannelRequest,
	) -> Result<OpenChannelResponse, LspRestError> {
		self.post_request(&request, OPEN_CHANNEL_PATH).await
	}

	/// Splices funds into the channel specified by given request.
	/// For API contract/usage, refer to docs for [`SpliceInRequest`] and [`SpliceInResponse`].
	pub async fn splice_in(
		&self, request: SpliceInRequest,
	) -> Result<SpliceInResponse, LspRestError> {
		self.post_request(&request, SPLICE_IN_PATH).await
	}

	/// Splices funds out of the channel specified by given request.
	/// For API contract/usage, refer to docs for [`SpliceOutRequest`] and [`SpliceOutResponse`].
	pub async fn splice_out(
		&self, request: SpliceOutRequest,
	) -> Result<SpliceOutResponse, LspRestError> {
		self.post_request(&request, SPLICE_OUT_PATH).await
	}

	/// Closes the channel specified by given request.
	/// For API contract/usage, refer to docs for [`CloseChannelRequest`] and [`CloseChannelResponse`].
	pub async fn close_channel(
		&self, request: CloseChannelRequest,
	) -> Result<CloseChannelResponse, LspRestError> {
		self.post_request(&request, CLOSE_CHANNEL_PATH).await
	}

	/// Force closes the channel specified by given request.
	/// For API contract/usage, refer to docs for [`ForceCloseChannelRequest`] and [`ForceCloseChannelResponse`].
	pub async fn force_close_channel(
		&self, request: ForceCloseChannelRequest,
	) -> Result<ForceCloseChannelResponse, LspRestError> {
		self.post_request(&request, FORCE_CLOSE_CHANNEL_PATH).await
	}

	/// Retrieves list of known channels.
	/// For API contract/usage, refer to docs for [`ListChannelsRequest`] and [`ListChannelsResponse`].
	pub async fn list_channels(
		&self, request: ListChannelsRequest,
	) -> Result<ListChannelsResponse, LspRestError> {
		self.post_request(&request, LIST_CHANNELS_PATH).await
	}

	/// Retrieves list of all payments sent or received by us.
	/// For API contract/usage, refer to docs for [`ListPaymentsRequest`] and [`ListPaymentsResponse`].
	pub async fn list_payments(
		&self, request: ListPaymentsRequest,
	) -> Result<ListPaymentsResponse, LspRestError> {
		self.post_request(&request, LIST_PAYMENTS_PATH).await
	}

	/// Updates the config for a previously opened channel.
	/// For API contract/usage, refer to docs for [`UpdateChannelConfigRequest`] and [`UpdateChannelConfigResponse`].
	pub async fn update_channel_config(
		&self, request: UpdateChannelConfigRequest,
	) -> Result<UpdateChannelConfigResponse, LspRestError> {
		self.post_request(&request, UPDATE_CHANNEL_CONFIG_PATH).await
	}

	/// Retrieves payment details for a given payment id.
	/// For API contract/usage, refer to docs for [`GetPaymentDetailsRequest`] and [`GetPaymentDetailsResponse`].
	pub async fn get_payment_details(
		&self, request: GetPaymentDetailsRequest,
	) -> Result<GetPaymentDetailsResponse, LspRestError> {
		self.post_request(&request, GET_PAYMENT_DETAILS_PATH).await
	}

	/// Retrieves list of all forwarded payments.
	/// For API contract/usage, refer to docs for [`ListForwardedPaymentsRequest`] and [`ListForwardedPaymentsResponse`].
	pub async fn list_forwarded_payments(
		&self, request: ListForwardedPaymentsRequest,
	) -> Result<ListForwardedPaymentsResponse, LspRestError> {
		self.post_request(&request, LIST_FORWARDED_PAYMENTS_PATH).await
	}

	/// Connect to a peer on the Lightning Network.
	/// For API contract/usage, refer to docs for [`ConnectPeerRequest`] and [`ConnectPeerResponse`].
	pub async fn connect_peer(
		&self, request: ConnectPeerRequest,
	) -> Result<ConnectPeerResponse, LspRestError> {
		self.post_request(&request, CONNECT_PEER_PATH).await
	}

	/// Disconnect from a peer and remove it from the peer store.
	/// For API contract/usage, refer to docs for [`DisconnectPeerRequest`] and [`DisconnectPeerResponse`].
	pub async fn disconnect_peer(
		&self, request: DisconnectPeerRequest,
	) -> Result<DisconnectPeerResponse, LspRestError> {
		self.post_request(&request, DISCONNECT_PEER_PATH).await
	}

	/// Retrieves a list of all known peers.
	/// For API contract/usage, refer to docs for [`ListPeersRequest`] and [`ListPeersResponse`].
	pub async fn list_peers(
		&self, request: ListPeersRequest,
	) -> Result<ListPeersResponse, LspRestError> {
		self.post_request(&request, LIST_PEERS_PATH).await
	}

	/// Send a spontaneous payment (keysend) to a node.
	/// For API contract/usage, refer to docs for [`SpontaneousSendRequest`] and [`SpontaneousSendResponse`].
	pub async fn spontaneous_send(
		&self, request: SpontaneousSendRequest,
	) -> Result<SpontaneousSendResponse, LspRestError> {
		self.post_request(&request, SPONTANEOUS_SEND_PATH).await
	}

	/// Send a payment given a BIP 21 URI or BIP 353 Human-Readable Name.
	/// For API contract/usage, refer to docs for [`UnifiedSendRequest`] and [`UnifiedSendResponse`].
	pub async fn unified_send(
		&self, request: UnifiedSendRequest,
	) -> Result<UnifiedSendResponse, LspRestError> {
		self.post_request(&request, UNIFIED_SEND_PATH).await
	}

	/// Sign a message with the node's secret key.
	/// For API contract/usage, refer to docs for [`SignMessageRequest`] and [`SignMessageResponse`].
	pub async fn sign_message(
		&self, request: SignMessageRequest,
	) -> Result<SignMessageResponse, LspRestError> {
		self.post_request(&request, SIGN_MESSAGE_PATH).await
	}

	/// Verify a signature against a message and public key.
	/// For API contract/usage, refer to docs for [`VerifySignatureRequest`] and [`VerifySignatureResponse`].
	pub async fn verify_signature(
		&self, request: VerifySignatureRequest,
	) -> Result<VerifySignatureResponse, LspRestError> {
		self.post_request(&request, VERIFY_SIGNATURE_PATH).await
	}

	/// Export the pathfinding scores used by the router.
	/// For API contract/usage, refer to docs for [`ExportPathfindingScoresRequest`] and [`ExportPathfindingScoresResponse`].
	pub async fn export_pathfinding_scores(
		&self, request: ExportPathfindingScoresRequest,
	) -> Result<ExportPathfindingScoresResponse, LspRestError> {
		self.post_request(&request, EXPORT_PATHFINDING_SCORES_PATH).await
	}

	/// Returns a list of all known short channel IDs in the network graph.
	/// For API contract/usage, refer to docs for [`GraphListChannelsRequest`] and [`GraphListChannelsResponse`].
	pub async fn graph_list_channels(
		&self, request: GraphListChannelsRequest,
	) -> Result<GraphListChannelsResponse, LspRestError> {
		self.post_request(&request, GRAPH_LIST_CHANNELS_PATH).await
	}

	/// Returns information on a channel with the given short channel ID from the network graph.
	/// For API contract/usage, refer to docs for [`GraphGetChannelRequest`] and [`GraphGetChannelResponse`].
	pub async fn graph_get_channel(
		&self, request: GraphGetChannelRequest,
	) -> Result<GraphGetChannelResponse, LspRestError> {
		self.post_request(&request, GRAPH_GET_CHANNEL_PATH).await
	}

	/// Returns a list of all known node IDs in the network graph.
	/// For API contract/usage, refer to docs for [`GraphListNodesRequest`] and [`GraphListNodesResponse`].
	pub async fn graph_list_nodes(
		&self, request: GraphListNodesRequest,
	) -> Result<GraphListNodesResponse, LspRestError> {
		self.post_request(&request, GRAPH_LIST_NODES_PATH).await
	}

	/// Returns information on a node with the given ID from the network graph.
	/// For API contract/usage, refer to docs for [`GraphGetNodeRequest`] and [`GraphGetNodeResponse`].
	pub async fn graph_get_node(
		&self, request: GraphGetNodeRequest,
	) -> Result<GraphGetNodeResponse, LspRestError> {
		self.post_request(&request, GRAPH_GET_NODE_PATH).await
	}

	/// Retrieves the current cached BTC/USD price.
	pub async fn get_price(
		&self, request: GetPriceRequest,
	) -> Result<GetPriceResponse, LspRestError> {
		self.post_request(&request, GET_PRICE_PATH).await
	}

	/// Lists all stable channels with their current state.
	pub async fn list_stable_channels(
		&self, request: ListStableChannelsRequest,
	) -> Result<ListStableChannelsResponse, LspRestError> {
		self.post_request(&request, LIST_STABLE_CHANNELS_PATH).await
	}

	/// Lists settlement payments recorded by the SC daemon.
	pub async fn list_settlement_payments(
		&self, request: ListSettlementPaymentsRequest,
	) -> Result<ListSettlementPaymentsResponse, LspRestError> {
		self.post_request(&request, LIST_SETTLEMENT_PAYMENTS_PATH).await
	}

	/// Query the authoritative SQLite channel ledger with exact reference matching.
	pub async fn list_channel_ledger_events(
		&self, request: ListChannelLedgerEventsRequest,
	) -> Result<ListChannelLedgerEventsResponse, LspRestError> {
		self.post_request(&request, LIST_CHANNEL_LEDGER_EVENTS_PATH).await
	}

	/// Edits a stable channel's target USD amount or note.
	pub async fn edit_stable_channel(
		&self, request: EditStableChannelRequest,
	) -> Result<EditStableChannelResponse, LspRestError> {
		self.post_request(&request, EDIT_STABLE_CHANNEL_PATH).await
	}

	/// Counts a stability payment the node will not report on as arrived or as not arrived; the first call returns the figures to confirm.
	pub async fn release_stability_payment(
		&self, request: ReleaseStabilityPaymentRequest,
	) -> Result<ReleaseStabilityPaymentResponse, LspRestError> {
		self.post_request(&request, RELEASE_STABILITY_PAYMENT_PATH).await
	}

	/// Queues an LSP splice-in that the daemon finishes once the phone is online, waking it by push first.
	pub async fn queue_splice_in(
		&self, request: QueueSpliceInRequest,
	) -> Result<QueueSpliceInResponse, LspRestError> {
		self.post_request(&request, QUEUE_SPLICE_IN_PATH).await
	}

	/// Every channel's queued splice-in and where it got to.
	pub async fn list_queued_splices(
		&self, request: ListQueuedSplicesRequest,
	) -> Result<ListQueuedSplicesResponse, LspRestError> {
		self.post_request(&request, LIST_QUEUED_SPLICES_PATH).await
	}

	/// Cancels a queued splice-in that has not locked.
	pub async fn cancel_queued_splice(
		&self, request: CancelQueuedSpliceRequest,
	) -> Result<CancelQueuedSpliceResponse, LspRestError> {
		self.post_request(&request, CANCEL_QUEUED_SPLICE_PATH).await
	}

	/// Revenue summary and activity for a window, from the SC daemon's snapshot.
	pub async fn get_revenue(&self, request: GetRevenueRequest) -> Result<GetRevenueResponse, LspRestError> {
		self.post_request(&request, GET_REVENUE_PATH).await
	}

	/// Sends a rejected trade's fee back to the user, once.
	pub async fn refund_trade_fee(
		&self, request: RefundTradeFeeRequest,
	) -> Result<RefundTradeFeeResponse, LspRestError> {
		self.post_request(&request, REFUND_TRADE_FEE_PATH).await
	}

	/// Tail LDK Server's log file via the SC daemon.
	pub async fn ldk_log(&self, request: LogRequest) -> Result<LogResponse, LspRestError> {
		self.post_request(&request, LDK_LOG_PATH).await
	}

	/// Tail the SC daemon's own audit log.
	pub async fn audit_log(&self, request: LogRequest) -> Result<LogResponse, LspRestError> {
		self.post_request(&request, AUDIT_LOG_PATH).await
	}

	async fn post_request<Rq: Message, Rs: Message + Default>(
		&self, request: &Rq, route: &str,
	) -> Result<Rs, LspRestError> {
		let known = self.session.lock().unwrap().clone();
		let used = match known {
			Some(c) => c,
			None => self.ensure(None).await?,
		};
		match self.send_signed(request, route, &used).await {
			Err(e) if needs_relogin(&e) => {
				let fresh = self.ensure(Some(&used.id)).await?;
				self.send_signed(request, route, &fresh).await
			},
			other => other,
		}
	}

	async fn send_signed<Rq: Message, Rs: Message + Default>(
		&self, request: &Rq, route: &str, session: &Session,
	) -> Result<Rs, LspRestError> {
		let body = request.encode_to_vec();
		let mut nonce = [0u8; 16];
		getrandom::getrandom(&mut nonce)
			.map_err(|e| LspRestError::new(InternalError, format!("No random nonce: {e}")))?;
		let header = build_auth_header(session, route, now_secs(), &nonce, &body);
		self.send_bytes(body, route, Some(header)).await
	}

	async fn send_bytes<Rs: Message + Default>(
		&self, request_body: Vec<u8>, route: &str, header: Option<String>,
	) -> Result<Rs, LspRestError> {
		let url = self.build_url(route);
		let mut builder = self.client.post(url).header(CONTENT_TYPE, APPLICATION_OCTET_STREAM);
		if let Some(h) = header {
			builder = builder.header("X-Auth", h);
		}
		let response_raw = builder
			.body(request_body)
			.send()
			.await
			.map_err(|e| LspRestError::new(InternalError, format!("HTTP request failed: {}", e)))?;

		let status = response_raw.status();
		let payload = response_raw.bytes().await.map_err(|e| {
			LspRestError::new(InternalError, format!("Failed to read response body: {}", e))
		})?;

		if status.is_success() {
			Ok(Rs::decode(&payload[..]).map_err(|e| {
				LspRestError::new(
					InternalError,
					format!("Failed to decode success response: {}", e),
				)
			})?)
		} else {
			// An unknown route (daemon or nginx) answers with no body; carry the status, not an empty message.
			if payload.is_empty() {
				return Err(LspRestError::new(InternalError, format!("HTTP {}", status)));
			}
			let error_response = ErrorResponse::decode(&payload[..]).map_err(|e| {
				LspRestError::new(
					InternalError,
					format!("Failed to decode error response (status {}): {}", status, e),
				)
			})?;

			let error_code = match ErrorCode::from_i32(error_response.error_code) {
				Some(ErrorCode::InvalidRequestError) => InvalidRequestError,
				Some(ErrorCode::AuthError) => AuthError,
				Some(ErrorCode::LightningError) => LightningError,
				Some(ErrorCode::InternalServerError) => InternalServerError,
				Some(ErrorCode::UnknownError) | None => InternalError,
			};

			Err(LspRestError::new(error_code, error_response.message))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sc_protos::access::{parse_session_header, session_mac, SESSION_UNKNOWN};

	#[test]
	fn only_the_session_unknown_error_triggers_a_relogin() {
		assert!(needs_relogin(&LspRestError::new(AuthError, SESSION_UNKNOWN)));
		assert!(!needs_relogin(&LspRestError::new(AuthError, "token revoked or expired")));
		assert!(!needs_relogin(&LspRestError::new(AuthError, "this token lacks payments:send")));
		assert!(!needs_relogin(&LspRestError::new(InternalServerError, SESSION_UNKNOWN)));
		assert!(!needs_relogin(&LspRestError::new(InternalServerError, "boom")));
	}

	#[test]
	fn the_client_signs_exactly_like_the_daemon_checks() {
		let session = Session { id: "sid".to_string(), secret: [7u8; 32] };
		let header = build_auth_header(&session, "GetNodeInfo", 1_800_000_000, &[9u8; 16], b"");
		let parsed = parse_session_header(&header).expect("header parses");
		assert_eq!(parsed.session_id, "sid");
		assert_eq!(parsed.ts, 1_800_000_000);
		assert_eq!(parsed.nonce, [9u8; 16]);
		assert_eq!(parsed.mac, "dcdcfebcae48a490d0772961a7658290221901b583f17a9144a1b96efc565105");
		assert_eq!(parsed.mac, session_mac(&[7u8; 32], "GetNodeInfo", 1_800_000_000, &[9u8; 16], b""));
	}

	struct YieldOnce(bool);

	impl std::future::Future for YieldOnce {
		type Output = ();
		fn poll(
			mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>,
		) -> std::task::Poll<()> {
			if self.0 {
				return std::task::Poll::Ready(());
			}
			self.0 = true;
			cx.waker().wake_by_ref();
			std::task::Poll::Pending
		}
	}

	// Runs 10 callers at once against one gate; each fake login counts itself and yields so they interleave.
	fn run_callers(store: &Mutex<Option<Session>>, seen: Option<&str>) -> (usize, Vec<String>) {
		let gate = LoginGate::default();
		let logins = std::sync::atomic::AtomicUsize::new(0);
		let calls = (0..10).map(|_| {
			ensure_session(&gate, store, seen, || async {
				let n = logins.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
				YieldOnce(false).await;
				Ok(Session { id: format!("s{n}"), secret: [1u8; 32] })
			})
		});
		let results = futures_executor::block_on(futures_util::future::join_all(calls));
		let ids = results.into_iter().map(|r| r.unwrap().id).collect();
		(logins.load(std::sync::atomic::Ordering::SeqCst), ids)
	}

	#[test]
	fn concurrent_requests_without_a_session_log_in_once() {
		let store = Mutex::new(None);
		let (logins, ids) = run_callers(&store, None);
		assert_eq!(logins, 1);
		assert!(ids.iter().all(|i| i == &ids[0]));
	}

	#[test]
	fn requests_failing_on_the_same_old_session_log_in_once() {
		let store = Mutex::new(Some(Session { id: "old".to_string(), secret: [2u8; 32] }));
		let (logins, ids) = run_callers(&store, Some("old"));
		assert_eq!(logins, 1);
		assert!(ids.iter().all(|i| i == "s0"));
	}

	#[test]
	fn a_request_whose_session_was_already_replaced_does_not_log_in() {
		let store = Mutex::new(Some(Session { id: "new".to_string(), secret: [3u8; 32] }));
		let (logins, ids) = run_callers(&store, Some("old"));
		assert_eq!(logins, 0);
		assert!(ids.iter().all(|i| i == "new"));
	}
}
