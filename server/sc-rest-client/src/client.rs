#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
use js_sys;

use bitcoin_hashes::hmac::{Hmac, HmacEngine};
use bitcoin_hashes::{sha256, Hash, HashEngine};
use ldk_server_client::ldk_server_grpc::api::{
	Bolt11ClaimForHashRequest, Bolt11ClaimForHashResponse, Bolt11FailForHashRequest,
	Bolt11FailForHashResponse, Bolt11ReceiveForHashRequest, Bolt11ReceiveForHashResponse,
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
use ldk_server_client::ldk_server_grpc::endpoints::{
	BOLT11_CLAIM_FOR_HASH_PATH, BOLT11_FAIL_FOR_HASH_PATH, BOLT11_RECEIVE_FOR_HASH_PATH,
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
use ldk_server_client::ldk_server_grpc::error::{ErrorCode, ErrorResponse};
use sc_protos::stable::{
	EditStableChannelRequest, EditStableChannelResponse, GetPriceRequest, GetPriceResponse,
	ListSettlementPaymentsRequest, ListSettlementPaymentsResponse, ListStableChannelsRequest,
	ListStableChannelsResponse, LogRequest, LogResponse, EDIT_STABLE_CHANNEL_PATH,
	GET_PRICE_PATH, LDK_LOG_PATH, LIST_SETTLEMENT_PAYMENTS_PATH, LIST_STABLE_CHANNELS_PATH,
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
#[derive(Clone)]
pub struct LspRestClient {
	base_url: String,
	client: Client,
	api_key: String,
	/// If true, use relative URLs (for WASM behind reverse proxy)
	use_relative_urls: bool,
}

impl LspRestClient {
	/// Constructs a [`LspRestClient`] using `base_url` as the SC daemon REST endpoint.
	///
	/// `base_url` should not include the scheme, e.g., `localhost:3000`.
	/// `api_key` is used for HMAC-based authentication.
	/// `server_cert_pem` is the server's TLS certificate in PEM format. This can be
	/// found at `<server_storage_dir>/tls.crt` after the server starts.
	///
	/// Note: On WASM targets, the certificate parameter is ignored as the browser
	/// handles TLS verification.
	#[cfg(not(target_arch = "wasm32"))]
	pub fn new(base_url: String, api_key: String, server_cert_pem: &[u8]) -> Result<Self, String> {
		let cert = Certificate::from_pem(server_cert_pem)
			.map_err(|e| format!("Failed to parse server certificate: {e}"))?;

		let client = Client::builder()
			.add_root_certificate(cert)
			.build()
			.map_err(|e| format!("Failed to build HTTP client: {e}"))?;

		Ok(Self { base_url, client, api_key, use_relative_urls: false })
	}

	/// Constructs a [`LspRestClient`] for WASM targets.
	///
	/// `base_url` should not include the scheme, e.g., `localhost:3000`.
	/// `api_key` is used for HMAC-based authentication.
	///
	/// On WASM, the browser handles TLS verification automatically.
	#[cfg(target_arch = "wasm32")]
	pub fn new(base_url: String, api_key: String, _server_cert_pem: &[u8]) -> Result<Self, String> {
		let client =
			Client::builder().build().map_err(|e| format!("Failed to build HTTP client: {e}"))?;

		// WASM builds default to relative URLs for reverse proxy setups
		Ok(Self { base_url, client, api_key, use_relative_urls: true })
	}

	/// Constructs a [`LspRestClient`] without requiring a TLS certificate.
	///
	/// This is useful for WASM targets where the browser handles TLS, or for
	/// development environments where certificate verification is handled differently.
	pub fn new_without_cert(base_url: String, api_key: String) -> Result<Self, String> {
		let client =
			Client::builder().build().map_err(|e| format!("Failed to build HTTP client: {e}"))?;

		#[cfg(target_arch = "wasm32")]
		let use_relative_urls = true;
		#[cfg(not(target_arch = "wasm32"))]
		let use_relative_urls = false;

		Ok(Self { base_url, client, api_key, use_relative_urls })
	}

	/// Builds the full URL for an API endpoint.
	fn build_url(&self, path: &str) -> String {
		if self.use_relative_urls {
			format!("/api/{path}")
		} else {
			format!("https://{}/{path}", self.base_url)
		}
	}

	/// Computes the HMAC-SHA256 authentication header value.
	/// Format: "HMAC <timestamp>:<hmac_hex>"
	fn compute_auth_header(&self, body: &[u8]) -> String {
		#[cfg(not(target_arch = "wasm32"))]
		let timestamp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.expect("System time should be after Unix epoch")
			.as_secs();

		#[cfg(target_arch = "wasm32")]
		let timestamp = (js_sys::Date::now() / 1000.0) as u64;

		// Compute HMAC-SHA256(api_key, timestamp_bytes || body)
		let mut hmac_engine: HmacEngine<sha256::Hash> = HmacEngine::new(self.api_key.as_bytes());
		hmac_engine.input(&timestamp.to_be_bytes());
		hmac_engine.input(body);
		let hmac_result = Hmac::<sha256::Hash>::from_engine(hmac_engine);

		format!("HMAC {}:{}", timestamp, hmac_result)
	}

	/// Retrieve the latest node info like `node_id`, `current_best_block` etc.
	/// For API contract/usage, refer to docs for [`GetNodeInfoRequest`] and [`GetNodeInfoResponse`].
	pub async fn get_node_info(
		&self, request: GetNodeInfoRequest,
	) -> Result<GetNodeInfoResponse, LspRestError> {
		let url = self.build_url(GET_NODE_INFO_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieves an overview of all known balances.
	/// For API contract/usage, refer to docs for [`GetBalancesRequest`] and [`GetBalancesResponse`].
	pub async fn get_balances(
		&self, request: GetBalancesRequest,
	) -> Result<GetBalancesResponse, LspRestError> {
		let url = self.build_url(GET_BALANCES_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieve a new on-chain funding address.
	/// For API contract/usage, refer to docs for [`OnchainReceiveRequest`] and [`OnchainReceiveResponse`].
	pub async fn onchain_receive(
		&self, request: OnchainReceiveRequest,
	) -> Result<OnchainReceiveResponse, LspRestError> {
		let url = self.build_url(ONCHAIN_RECEIVE_PATH);
		self.post_request(&request, &url).await
	}

	/// Send an on-chain payment to the given address.
	/// For API contract/usage, refer to docs for [`OnchainSendRequest`] and [`OnchainSendResponse`].
	pub async fn onchain_send(
		&self, request: OnchainSendRequest,
	) -> Result<OnchainSendResponse, LspRestError> {
		let url = self.build_url(ONCHAIN_SEND_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieve a new BOLT11 payable invoice.
	/// For API contract/usage, refer to docs for [`Bolt11ReceiveRequest`] and [`Bolt11ReceiveResponse`].
	pub async fn bolt11_receive(
		&self, request: Bolt11ReceiveRequest,
	) -> Result<Bolt11ReceiveResponse, LspRestError> {
		let url = self.build_url(BOLT11_RECEIVE_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieve a new BOLT11 payable invoice for a given payment hash.
	/// The inbound payment will NOT be automatically claimed upon arrival.
	/// For API contract/usage, refer to docs for [`Bolt11ReceiveForHashRequest`] and [`Bolt11ReceiveForHashResponse`].
	pub async fn bolt11_receive_for_hash(
		&self, request: Bolt11ReceiveForHashRequest,
	) -> Result<Bolt11ReceiveForHashResponse, LspRestError> {
		let url = format!("https://{}/{BOLT11_RECEIVE_FOR_HASH_PATH}", self.base_url);
		self.post_request(&request, &url).await
	}

	/// Manually claim a payment for a given payment hash with the corresponding preimage.
	/// For API contract/usage, refer to docs for [`Bolt11ClaimForHashRequest`] and [`Bolt11ClaimForHashResponse`].
	pub async fn bolt11_claim_for_hash(
		&self, request: Bolt11ClaimForHashRequest,
	) -> Result<Bolt11ClaimForHashResponse, LspRestError> {
		let url = format!("https://{}/{BOLT11_CLAIM_FOR_HASH_PATH}", self.base_url);
		self.post_request(&request, &url).await
	}

	/// Manually fail a payment for a given payment hash.
	/// For API contract/usage, refer to docs for [`Bolt11FailForHashRequest`] and [`Bolt11FailForHashResponse`].
	pub async fn bolt11_fail_for_hash(
		&self, request: Bolt11FailForHashRequest,
	) -> Result<Bolt11FailForHashResponse, LspRestError> {
		let url = format!("https://{}/{BOLT11_FAIL_FOR_HASH_PATH}", self.base_url);
		self.post_request(&request, &url).await
	}

	/// Retrieve a new fixed-amount BOLT11 invoice for receiving via an LSPS2 JIT channel.
	/// For API contract/usage, refer to docs for [`Bolt11ReceiveViaJitChannelRequest`] and
	/// [`Bolt11ReceiveViaJitChannelResponse`].
	pub async fn bolt11_receive_via_jit_channel(
		&self, request: Bolt11ReceiveViaJitChannelRequest,
	) -> Result<Bolt11ReceiveViaJitChannelResponse, LspRestError> {
		let url = format!("https://{}/{BOLT11_RECEIVE_VIA_JIT_CHANNEL_PATH}", self.base_url);
		self.post_request(&request, &url).await
	}

	/// Retrieve a new variable-amount BOLT11 invoice for receiving via an LSPS2 JIT channel.
	/// For API contract/usage, refer to docs for
	/// [`Bolt11ReceiveVariableAmountViaJitChannelRequest`] and
	/// [`Bolt11ReceiveVariableAmountViaJitChannelResponse`].
	pub async fn bolt11_receive_variable_amount_via_jit_channel(
		&self, request: Bolt11ReceiveVariableAmountViaJitChannelRequest,
	) -> Result<Bolt11ReceiveVariableAmountViaJitChannelResponse, LspRestError> {
		let url = format!(
			"https://{}/{BOLT11_RECEIVE_VARIABLE_AMOUNT_VIA_JIT_CHANNEL_PATH}",
			self.base_url,
		);
		self.post_request(&request, &url).await
	}

	/// Send a payment for a BOLT11 invoice.
	/// For API contract/usage, refer to docs for [`Bolt11SendRequest`] and [`Bolt11SendResponse`].
	pub async fn bolt11_send(
		&self, request: Bolt11SendRequest,
	) -> Result<Bolt11SendResponse, LspRestError> {
		let url = self.build_url(BOLT11_SEND_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieve a new BOLT11 payable offer.
	/// For API contract/usage, refer to docs for [`Bolt12ReceiveRequest`] and [`Bolt12ReceiveResponse`].
	pub async fn bolt12_receive(
		&self, request: Bolt12ReceiveRequest,
	) -> Result<Bolt12ReceiveResponse, LspRestError> {
		let url = self.build_url(BOLT12_RECEIVE_PATH);
		self.post_request(&request, &url).await
	}

	/// Send a payment for a BOLT12 offer.
	/// For API contract/usage, refer to docs for [`Bolt12SendRequest`] and [`Bolt12SendResponse`].
	pub async fn bolt12_send(
		&self, request: Bolt12SendRequest,
	) -> Result<Bolt12SendResponse, LspRestError> {
		let url = self.build_url(BOLT12_SEND_PATH);
		self.post_request(&request, &url).await
	}

	/// Creates a new outbound channel.
	/// For API contract/usage, refer to docs for [`OpenChannelRequest`] and [`OpenChannelResponse`].
	pub async fn open_channel(
		&self, request: OpenChannelRequest,
	) -> Result<OpenChannelResponse, LspRestError> {
		let url = self.build_url(OPEN_CHANNEL_PATH);
		self.post_request(&request, &url).await
	}

	/// Splices funds into the channel specified by given request.
	/// For API contract/usage, refer to docs for [`SpliceInRequest`] and [`SpliceInResponse`].
	pub async fn splice_in(
		&self, request: SpliceInRequest,
	) -> Result<SpliceInResponse, LspRestError> {
		let url = self.build_url(SPLICE_IN_PATH);
		self.post_request(&request, &url).await
	}

	/// Splices funds out of the channel specified by given request.
	/// For API contract/usage, refer to docs for [`SpliceOutRequest`] and [`SpliceOutResponse`].
	pub async fn splice_out(
		&self, request: SpliceOutRequest,
	) -> Result<SpliceOutResponse, LspRestError> {
		let url = self.build_url(SPLICE_OUT_PATH);
		self.post_request(&request, &url).await
	}

	/// Closes the channel specified by given request.
	/// For API contract/usage, refer to docs for [`CloseChannelRequest`] and [`CloseChannelResponse`].
	pub async fn close_channel(
		&self, request: CloseChannelRequest,
	) -> Result<CloseChannelResponse, LspRestError> {
		let url = self.build_url(CLOSE_CHANNEL_PATH);
		self.post_request(&request, &url).await
	}

	/// Force closes the channel specified by given request.
	/// For API contract/usage, refer to docs for [`ForceCloseChannelRequest`] and [`ForceCloseChannelResponse`].
	pub async fn force_close_channel(
		&self, request: ForceCloseChannelRequest,
	) -> Result<ForceCloseChannelResponse, LspRestError> {
		let url = self.build_url(FORCE_CLOSE_CHANNEL_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieves list of known channels.
	/// For API contract/usage, refer to docs for [`ListChannelsRequest`] and [`ListChannelsResponse`].
	pub async fn list_channels(
		&self, request: ListChannelsRequest,
	) -> Result<ListChannelsResponse, LspRestError> {
		let url = self.build_url(LIST_CHANNELS_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieves list of all payments sent or received by us.
	/// For API contract/usage, refer to docs for [`ListPaymentsRequest`] and [`ListPaymentsResponse`].
	pub async fn list_payments(
		&self, request: ListPaymentsRequest,
	) -> Result<ListPaymentsResponse, LspRestError> {
		let url = self.build_url(LIST_PAYMENTS_PATH);
		self.post_request(&request, &url).await
	}

	/// Updates the config for a previously opened channel.
	/// For API contract/usage, refer to docs for [`UpdateChannelConfigRequest`] and [`UpdateChannelConfigResponse`].
	pub async fn update_channel_config(
		&self, request: UpdateChannelConfigRequest,
	) -> Result<UpdateChannelConfigResponse, LspRestError> {
		let url = self.build_url(UPDATE_CHANNEL_CONFIG_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieves payment details for a given payment id.
	/// For API contract/usage, refer to docs for [`GetPaymentDetailsRequest`] and [`GetPaymentDetailsResponse`].
	pub async fn get_payment_details(
		&self, request: GetPaymentDetailsRequest,
	) -> Result<GetPaymentDetailsResponse, LspRestError> {
		let url = self.build_url(GET_PAYMENT_DETAILS_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieves list of all forwarded payments.
	/// For API contract/usage, refer to docs for [`ListForwardedPaymentsRequest`] and [`ListForwardedPaymentsResponse`].
	pub async fn list_forwarded_payments(
		&self, request: ListForwardedPaymentsRequest,
	) -> Result<ListForwardedPaymentsResponse, LspRestError> {
		let url = self.build_url(LIST_FORWARDED_PAYMENTS_PATH);
		self.post_request(&request, &url).await
	}

	/// Connect to a peer on the Lightning Network.
	/// For API contract/usage, refer to docs for [`ConnectPeerRequest`] and [`ConnectPeerResponse`].
	pub async fn connect_peer(
		&self, request: ConnectPeerRequest,
	) -> Result<ConnectPeerResponse, LspRestError> {
		let url = self.build_url(CONNECT_PEER_PATH);
		self.post_request(&request, &url).await
	}

	/// Disconnect from a peer and remove it from the peer store.
	/// For API contract/usage, refer to docs for [`DisconnectPeerRequest`] and [`DisconnectPeerResponse`].
	pub async fn disconnect_peer(
		&self, request: DisconnectPeerRequest,
	) -> Result<DisconnectPeerResponse, LspRestError> {
		let url = self.build_url(DISCONNECT_PEER_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieves a list of all known peers.
	/// For API contract/usage, refer to docs for [`ListPeersRequest`] and [`ListPeersResponse`].
	pub async fn list_peers(
		&self, request: ListPeersRequest,
	) -> Result<ListPeersResponse, LspRestError> {
		let url = self.build_url(LIST_PEERS_PATH);
		self.post_request(&request, &url).await
	}

	/// Send a spontaneous payment (keysend) to a node.
	/// For API contract/usage, refer to docs for [`SpontaneousSendRequest`] and [`SpontaneousSendResponse`].
	pub async fn spontaneous_send(
		&self, request: SpontaneousSendRequest,
	) -> Result<SpontaneousSendResponse, LspRestError> {
		let url = self.build_url(SPONTANEOUS_SEND_PATH);
		self.post_request(&request, &url).await
	}

	/// Send a payment given a BIP 21 URI or BIP 353 Human-Readable Name.
	/// For API contract/usage, refer to docs for [`UnifiedSendRequest`] and [`UnifiedSendResponse`].
	pub async fn unified_send(
		&self, request: UnifiedSendRequest,
	) -> Result<UnifiedSendResponse, LspRestError> {
		let url = format!("https://{}/{UNIFIED_SEND_PATH}", self.base_url);
		self.post_request(&request, &url).await
	}

	/// Sign a message with the node's secret key.
	/// For API contract/usage, refer to docs for [`SignMessageRequest`] and [`SignMessageResponse`].
	pub async fn sign_message(
		&self, request: SignMessageRequest,
	) -> Result<SignMessageResponse, LspRestError> {
		let url = self.build_url(SIGN_MESSAGE_PATH);
		self.post_request(&request, &url).await
	}

	/// Verify a signature against a message and public key.
	/// For API contract/usage, refer to docs for [`VerifySignatureRequest`] and [`VerifySignatureResponse`].
	pub async fn verify_signature(
		&self, request: VerifySignatureRequest,
	) -> Result<VerifySignatureResponse, LspRestError> {
		let url = self.build_url(VERIFY_SIGNATURE_PATH);
		self.post_request(&request, &url).await
	}

	/// Export the pathfinding scores used by the router.
	/// For API contract/usage, refer to docs for [`ExportPathfindingScoresRequest`] and [`ExportPathfindingScoresResponse`].
	pub async fn export_pathfinding_scores(
		&self, request: ExportPathfindingScoresRequest,
	) -> Result<ExportPathfindingScoresResponse, LspRestError> {
		let url = self.build_url(EXPORT_PATHFINDING_SCORES_PATH);
		self.post_request(&request, &url).await
	}

	/// Returns a list of all known short channel IDs in the network graph.
	/// For API contract/usage, refer to docs for [`GraphListChannelsRequest`] and [`GraphListChannelsResponse`].
	pub async fn graph_list_channels(
		&self, request: GraphListChannelsRequest,
	) -> Result<GraphListChannelsResponse, LspRestError> {
		let url = self.build_url(GRAPH_LIST_CHANNELS_PATH);
		self.post_request(&request, &url).await
	}

	/// Returns information on a channel with the given short channel ID from the network graph.
	/// For API contract/usage, refer to docs for [`GraphGetChannelRequest`] and [`GraphGetChannelResponse`].
	pub async fn graph_get_channel(
		&self, request: GraphGetChannelRequest,
	) -> Result<GraphGetChannelResponse, LspRestError> {
		let url = self.build_url(GRAPH_GET_CHANNEL_PATH);
		self.post_request(&request, &url).await
	}

	/// Returns a list of all known node IDs in the network graph.
	/// For API contract/usage, refer to docs for [`GraphListNodesRequest`] and [`GraphListNodesResponse`].
	pub async fn graph_list_nodes(
		&self, request: GraphListNodesRequest,
	) -> Result<GraphListNodesResponse, LspRestError> {
		let url = self.build_url(GRAPH_LIST_NODES_PATH);
		self.post_request(&request, &url).await
	}

	/// Returns information on a node with the given ID from the network graph.
	/// For API contract/usage, refer to docs for [`GraphGetNodeRequest`] and [`GraphGetNodeResponse`].
	pub async fn graph_get_node(
		&self, request: GraphGetNodeRequest,
	) -> Result<GraphGetNodeResponse, LspRestError> {
		let url = self.build_url(GRAPH_GET_NODE_PATH);
		self.post_request(&request, &url).await
	}

	/// Retrieves the current cached BTC/USD price.
	pub async fn get_price(
		&self, request: GetPriceRequest,
	) -> Result<GetPriceResponse, LspRestError> {
		let url = self.build_url(GET_PRICE_PATH);
		self.post_request(&request, &url).await
	}

	/// Lists all stable channels with their current state.
	pub async fn list_stable_channels(
		&self, request: ListStableChannelsRequest,
	) -> Result<ListStableChannelsResponse, LspRestError> {
		let url = self.build_url(LIST_STABLE_CHANNELS_PATH);
		self.post_request(&request, &url).await
	}

	/// Lists settlement payments recorded by the SC daemon.
	pub async fn list_settlement_payments(
		&self, request: ListSettlementPaymentsRequest,
	) -> Result<ListSettlementPaymentsResponse, LspRestError> {
		let url = self.build_url(LIST_SETTLEMENT_PAYMENTS_PATH);
		self.post_request(&request, &url).await
	}

	/// Edits a stable channel's target USD amount or note.
	pub async fn edit_stable_channel(
		&self, request: EditStableChannelRequest,
	) -> Result<EditStableChannelResponse, LspRestError> {
		let url = self.build_url(EDIT_STABLE_CHANNEL_PATH);
		self.post_request(&request, &url).await
	}

	/// Tail LDK Server's log file via the SC daemon.
	pub async fn ldk_log(&self, request: LogRequest) -> Result<LogResponse, LspRestError> {
		let url = self.build_url(LDK_LOG_PATH);
		self.post_request(&request, &url).await
	}

	async fn post_request<Rq: Message, Rs: Message + Default>(
		&self, request: &Rq, url: &str,
	) -> Result<Rs, LspRestError> {
		let request_body = request.encode_to_vec();
		let auth_header = self.compute_auth_header(&request_body);
		let response_raw = self
			.client
			.post(url)
			.header(CONTENT_TYPE, APPLICATION_OCTET_STREAM)
			.header("X-Auth", auth_header)
			.body(request_body)
			.send()
			.await
			.map_err(|e| {
				LspRestError::new(InternalError, format!("HTTP request failed: {}", e))
			})?;

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
