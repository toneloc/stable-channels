use std::sync::Arc;
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::config::{ChainSourceConfig, ChainSourceType};
use crate::task::ChannelTaskHandle;
use sc_rest_client::client::LspRestClient;
use sc_rest_client::ldk_server_grpc::api::{
	Bolt11ReceiveResponse, Bolt11SendResponse, Bolt12ReceiveResponse, Bolt12SendResponse,
	CloseChannelResponse, ConnectPeerResponse, DisconnectPeerResponse,
	ExportPathfindingScoresResponse, ForceCloseChannelResponse, GetBalancesResponse,
	GetNodeInfoResponse, GetPaymentDetailsResponse, GraphGetChannelResponse, GraphGetNodeResponse,
	GraphListChannelsResponse, GraphListNodesResponse, ListChannelsResponse,
	ListForwardedPaymentsResponse, ListPaymentsResponse, ListPeersResponse, OnchainReceiveResponse,
	OnchainSendResponse, OpenChannelResponse, SignMessageResponse, SpliceInResponse,
	SpliceOutResponse, SpontaneousSendResponse, UpdateChannelConfigResponse,
	VerifySignatureResponse,
};
use sc_rest_client::sc_protos::stable::{GetPriceResponse, ListSettlementPaymentsResponse, ListStableChannelsResponse, LogResponse};
use sc_rest_client::ldk_server_grpc::types::PageToken;

#[derive(Clone, PartialEq, Default)]
pub enum ConnectionStatus {
	#[default]
	Disconnected,
	Connected,
	Error(String),
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum ActiveTab {
	#[default]
	NodeInfo,
	Balances,
	Channels,
	Peers,
	Payments,
	ForwardedPayments,
	Lightning,
	Onchain,
	StableChannels,
	Tools,
	NetworkGraph,
	Logs,
	Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum DisplayUnit {
	#[default]
	Usd,
	Btc,
	Sats,
}

/// Stable-channel settlement classification recorded by the daemon, keyed by payment_id.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettlementKind {
	Stability,
	Sync,
}

impl SettlementKind {
	/// Parse the daemon's `kind` string; unknown strings are ignored.
	pub fn parse(s: &str) -> Option<Self> {
		match s {
			"stability" => Some(Self::Stability),
			"sync" => Some(Self::Sync),
			_ => None,
		}
	}
}

#[derive(Default, Clone)]
pub struct OpenChannelForm {
	pub node_pubkey: String,
	pub address: String,
	pub channel_amount_sats: String,
	pub push_to_counterparty_msat: String,
	pub announce_channel: bool,
	pub forwarding_fee_proportional_millionths: String,
	pub forwarding_fee_base_msat: String,
	pub cltv_expiry_delta: String,
}

#[derive(Default, Clone)]
pub struct Bolt11ReceiveForm {
	pub amount_msat: String,
	pub description: String,
	pub expiry_secs: String,
}

#[derive(Default, Clone)]
pub struct Bolt11SendForm {
	pub invoice: String,
	pub amount_msat: String,
}

#[derive(Default, Clone)]
pub struct Bolt12ReceiveForm {
	pub description: String,
	pub amount_msat: String,
	pub expiry_secs: String,
	pub quantity: String,
}

#[derive(Default, Clone)]
pub struct Bolt12SendForm {
	pub offer: String,
	pub amount_msat: String,
	pub quantity: String,
	pub payer_note: String,
}

#[derive(Default, Clone)]
pub struct OnchainSendForm {
	pub address: String,
	pub amount_sats: String,
	pub send_all: bool,
	pub fee_rate_sat_per_vb: String,
}

#[derive(Default, Clone)]
pub struct SpliceForm {
	pub user_channel_id: String,
	pub counterparty_node_id: String,
	pub splice_amount_sats: String,
	pub address: String,
}

#[derive(Default, Clone)]
pub struct UpdateChannelConfigForm {
	pub user_channel_id: String,
	pub counterparty_node_id: String,
	pub forwarding_fee_proportional_millionths: String,
	pub forwarding_fee_base_msat: String,
	pub cltv_expiry_delta: String,
}

#[derive(Default, Clone)]
pub struct CloseChannelForm {
	pub user_channel_id: String,
	pub counterparty_node_id: String,
	pub force_close_reason: String,
}

#[derive(Default, Clone)]
pub struct ConnectPeerForm {
	pub node_pubkey: String,
	pub address: String,
	pub persist: bool,
}

#[derive(Default, Clone)]
pub struct SpontaneousSendForm {
	pub amount_msat: String,
	pub node_id: String,
}

#[derive(Default, Clone)]
pub struct SignMessageForm {
	pub message: String,
}

#[derive(Default, Clone)]
pub struct VerifySignatureForm {
	pub message: String,
	pub signature: String,
	pub public_key: String,
}

#[derive(Default, Clone)]
pub struct GraphGetChannelForm {
	pub short_channel_id: String,
}

#[derive(Default, Clone)]
pub struct GraphGetNodeForm {
	pub node_id: String,
}

#[derive(Clone)]
pub struct LogForm {
	pub max_lines: String,
}

impl Default for LogForm {
	fn default() -> Self {
		Self { max_lines: "200".to_string() }
	}
}

/// Editable chain source configuration (used on native only)
#[allow(dead_code)]
#[derive(Default, Clone)]
pub struct ChainSourceForm {
	pub source_type: ChainSourceType,
	// Bitcoind fields
	pub btc_rpc_address: String,
	pub btc_rpc_user: String,
	pub btc_rpc_password: String,
	// Electrum/Esplora field
	pub server_url: String,
}

#[allow(dead_code)]
impl ChainSourceForm {
	pub fn from_config(config: &ChainSourceConfig) -> Self {
		match config {
			ChainSourceConfig::None => Self::default(),
			ChainSourceConfig::Bitcoind { rpc_address, rpc_user, rpc_password } => Self {
				source_type: ChainSourceType::Bitcoind,
				btc_rpc_address: rpc_address.clone(),
				btc_rpc_user: rpc_user.clone(),
				btc_rpc_password: rpc_password.clone(),
				server_url: String::new(),
			},
			ChainSourceConfig::Electrum { server_url } => Self {
				source_type: ChainSourceType::Electrum,
				server_url: server_url.clone(),
				..Default::default()
			},
			ChainSourceConfig::Esplora { server_url } => Self {
				source_type: ChainSourceType::Esplora,
				server_url: server_url.clone(),
				..Default::default()
			},
		}
	}

	pub fn to_config(&self) -> ChainSourceConfig {
		match self.source_type {
			ChainSourceType::None => ChainSourceConfig::None,
			ChainSourceType::Bitcoind => ChainSourceConfig::Bitcoind {
				rpc_address: self.btc_rpc_address.clone(),
				rpc_user: self.btc_rpc_user.clone(),
				rpc_password: self.btc_rpc_password.clone(),
			},
			ChainSourceType::Electrum => {
				ChainSourceConfig::Electrum { server_url: self.server_url.clone() }
			},
			ChainSourceType::Esplora => {
				ChainSourceConfig::Esplora { server_url: self.server_url.clone() }
			},
		}
	}
}

#[derive(Default, Clone)]
pub struct Forms {
	pub open_channel: OpenChannelForm,
	pub bolt11_receive: Bolt11ReceiveForm,
	pub bolt11_send: Bolt11SendForm,
	pub bolt12_receive: Bolt12ReceiveForm,
	pub bolt12_send: Bolt12SendForm,
	pub onchain_send: OnchainSendForm,
	pub splice_in: SpliceForm,
	pub splice_out: SpliceForm,
	pub update_channel_config: UpdateChannelConfigForm,
	pub close_channel: CloseChannelForm,
	pub connect_peer: ConnectPeerForm,
	pub spontaneous_send: SpontaneousSendForm,
	pub sign_message: SignMessageForm,
	pub verify_signature: VerifySignatureForm,
	pub graph_get_channel: GraphGetChannelForm,
	pub graph_get_node: GraphGetNodeForm,
	pub ldk_log: LogForm,
	#[allow(dead_code)]
	pub chain_source: ChainSourceForm,
}

pub struct StatusMessage {
	pub text: String,
	pub is_error: bool,
	#[allow(dead_code)]
	#[cfg(not(target_arch = "wasm32"))]
	pub timestamp: Instant,
	#[allow(dead_code)]
	#[cfg(target_arch = "wasm32")]
	pub timestamp: f64,
}

impl StatusMessage {
	pub fn success(text: impl Into<String>) -> Self {
		Self {
			text: text.into(),
			is_error: false,
			#[cfg(not(target_arch = "wasm32"))]
			timestamp: Instant::now(),
			#[cfg(target_arch = "wasm32")]
			timestamp: 0.0, // Could use js_sys::Date::now() if needed
		}
	}

	pub fn error(text: impl Into<String>) -> Self {
		Self {
			text: text.into(),
			is_error: true,
			#[cfg(not(target_arch = "wasm32"))]
			timestamp: Instant::now(),
			#[cfg(target_arch = "wasm32")]
			timestamp: 0.0,
		}
	}
}

pub struct AsyncTasks {
	pub node_info: Option<ChannelTaskHandle<GetNodeInfoResponse>>,
	pub balances: Option<ChannelTaskHandle<GetBalancesResponse>>,
	pub channels: Option<ChannelTaskHandle<ListChannelsResponse>>,
	pub payments: Option<ChannelTaskHandle<ListPaymentsResponse>>,
	pub peers: Option<ChannelTaskHandle<ListPeersResponse>>,
	pub forwarded_payments: Option<ChannelTaskHandle<ListForwardedPaymentsResponse>>,
	pub payment_details: Option<ChannelTaskHandle<GetPaymentDetailsResponse>>,
	pub onchain_receive: Option<ChannelTaskHandle<OnchainReceiveResponse>>,
	pub onchain_send: Option<ChannelTaskHandle<OnchainSendResponse>>,
	pub bolt11_receive: Option<ChannelTaskHandle<Bolt11ReceiveResponse>>,
	pub bolt11_send: Option<ChannelTaskHandle<Bolt11SendResponse>>,
	pub bolt12_receive: Option<ChannelTaskHandle<Bolt12ReceiveResponse>>,
	pub bolt12_send: Option<ChannelTaskHandle<Bolt12SendResponse>>,
	pub open_channel: Option<ChannelTaskHandle<OpenChannelResponse>>,
	pub close_channel: Option<ChannelTaskHandle<CloseChannelResponse>>,
	pub force_close_channel: Option<ChannelTaskHandle<ForceCloseChannelResponse>>,
	pub splice_in: Option<ChannelTaskHandle<SpliceInResponse>>,
	pub splice_out: Option<ChannelTaskHandle<SpliceOutResponse>>,
	pub update_channel_config: Option<ChannelTaskHandle<UpdateChannelConfigResponse>>,
	pub connect_peer: Option<ChannelTaskHandle<ConnectPeerResponse>>,
	pub disconnect_peer: Option<ChannelTaskHandle<DisconnectPeerResponse>>,
	pub spontaneous_send: Option<ChannelTaskHandle<SpontaneousSendResponse>>,
	pub sign_message: Option<ChannelTaskHandle<SignMessageResponse>>,
	pub verify_signature: Option<ChannelTaskHandle<VerifySignatureResponse>>,
	pub graph_list_channels: Option<ChannelTaskHandle<GraphListChannelsResponse>>,
	pub graph_get_channel: Option<ChannelTaskHandle<GraphGetChannelResponse>>,
	pub graph_list_nodes: Option<ChannelTaskHandle<GraphListNodesResponse>>,
	pub graph_get_node: Option<ChannelTaskHandle<GraphGetNodeResponse>>,
	pub export_pathfinding_scores: Option<ChannelTaskHandle<ExportPathfindingScoresResponse>>,
	pub get_price: Option<ChannelTaskHandle<GetPriceResponse>>,
	pub list_stable_channels: Option<ChannelTaskHandle<ListStableChannelsResponse>>,
	pub list_settlement_payments: Option<ChannelTaskHandle<ListSettlementPaymentsResponse>>,
	pub ldk_log: Option<ChannelTaskHandle<LogResponse>>,
}

impl Default for AsyncTasks {
	fn default() -> Self {
		Self {
			node_info: None,
			balances: None,
			channels: None,
			payments: None,
			peers: None,
			forwarded_payments: None,
			payment_details: None,
			onchain_receive: None,
			onchain_send: None,
			bolt11_receive: None,
			bolt11_send: None,
			bolt12_receive: None,
			bolt12_send: None,
			open_channel: None,
			close_channel: None,
			force_close_channel: None,
			splice_in: None,
			splice_out: None,
			update_channel_config: None,
			connect_peer: None,
			disconnect_peer: None,
			spontaneous_send: None,
			sign_message: None,
			verify_signature: None,
			graph_list_channels: None,
			graph_get_channel: None,
			graph_list_nodes: None,
			graph_get_node: None,
			export_pathfinding_scores: None,
			get_price: None,
			list_stable_channels: None,
			list_settlement_payments: None,
			ldk_log: None,
		}
	}
}

impl AsyncTasks {
	pub fn any_pending(&self) -> bool {
		self.node_info.is_some()
			|| self.balances.is_some()
			|| self.channels.is_some()
			|| self.payments.is_some()
			|| self.peers.is_some()
			|| self.forwarded_payments.is_some()
			|| self.payment_details.is_some()
			|| self.onchain_receive.is_some()
			|| self.onchain_send.is_some()
			|| self.bolt11_receive.is_some()
			|| self.bolt11_send.is_some()
			|| self.bolt12_receive.is_some()
			|| self.bolt12_send.is_some()
			|| self.open_channel.is_some()
			|| self.close_channel.is_some()
			|| self.force_close_channel.is_some()
			|| self.splice_in.is_some()
			|| self.splice_out.is_some()
			|| self.update_channel_config.is_some()
			|| self.connect_peer.is_some()
			|| self.disconnect_peer.is_some()
			|| self.spontaneous_send.is_some()
			|| self.sign_message.is_some()
			|| self.verify_signature.is_some()
			|| self.graph_list_channels.is_some()
			|| self.graph_get_channel.is_some()
			|| self.graph_list_nodes.is_some()
			|| self.graph_get_node.is_some()
			|| self.export_pathfinding_scores.is_some()
			|| self.get_price.is_some()
			|| self.list_stable_channels.is_some()
			|| self.list_settlement_payments.is_some()
			|| self.ldk_log.is_some()
	}
}

pub struct AppState {
	// Connection settings
	pub server_url: String,
	pub api_key: String,
	#[allow(dead_code)] // Used only on native
	pub tls_cert_path: String,
	pub connection_status: ConnectionStatus,
	pub client: Option<Arc<LspRestClient>>,

	// Config info (from loaded config file)
	#[allow(dead_code)] // Used only on native
	pub config_file_path: Option<String>,
	pub network: String,
	pub chain_source: ChainSourceConfig,

	// Navigation
	pub active_tab: ActiveTab,

	// Cached API responses
	pub node_info: Option<GetNodeInfoResponse>,
	pub balances: Option<GetBalancesResponse>,
	pub channels: Option<ListChannelsResponse>,
	pub payments: Option<ListPaymentsResponse>,
	pub payments_page_token: Option<PageToken>,
	// True while a "Load More" fetch is in flight, so its result appends instead of replacing.
	pub payments_appending: bool,
	pub peers: Option<ListPeersResponse>,
	pub forwarded_payments: Option<ListForwardedPaymentsResponse>,
	pub forwarded_payments_page_token: Option<PageToken>,
	pub payment_details: Option<GetPaymentDetailsResponse>,
	pub price: Option<GetPriceResponse>,
	pub stable_channels: Option<ListStableChannelsResponse>,
	// payment_id (hex) -> settlement classification, fetched alongside payments.
	pub settlement_kinds: Option<HashMap<String, SettlementKind>>,
	pub ldk_log: Option<LogResponse>,

	// Operation results
	pub onchain_address: Option<String>,
	pub generated_invoice: Option<String>,
	pub generated_offer: Option<String>,
	pub last_payment_id: Option<String>,
	pub last_txid: Option<String>,
	pub last_channel_id: Option<String>,

	// Async tasks
	pub tasks: AsyncTasks,

	// Form state
	pub forms: Forms,

	// UI state
	pub display_unit: DisplayUnit,
	pub last_price_fetch: f64,
	pub auto_connect_pending: bool,
	pub status_message: Option<StatusMessage>,
	pub show_open_channel_dialog: bool,
	pub show_close_channel_dialog: bool,
	pub show_splice_in_dialog: bool,
	pub show_splice_out_dialog: bool,
	pub show_update_config_dialog: bool,
	pub show_connect_peer_dialog: bool,
	pub show_load_config_dialog: bool,
	pub show_payment_details_dialog: bool,
	pub payment_details_id: String,
	pub config_paste_text: String,
	pub lightning_tab: LightningTab,
	pub onchain_tab: OnchainTab,
	pub graph_channels: Option<GraphListChannelsResponse>,
	pub graph_channel_detail: Option<GraphGetChannelResponse>,
	pub graph_nodes: Option<GraphListNodesResponse>,
	pub graph_node_detail: Option<GraphGetNodeResponse>,
	pub sign_result: Option<String>,
	pub verify_result: Option<bool>,
	pub export_scores_result: Option<ExportPathfindingScoresResponse>,
	pub show_disconnect_peer_dialog: bool,
	#[allow(dead_code)]
	pub disconnect_peer_pubkey: String,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum LightningTab {
	#[default]
	Bolt11Send,
	Bolt11Receive,
	Bolt12Send,
	Bolt12Receive,
	SpontaneousSend,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum OnchainTab {
	#[default]
	Send,
	Receive,
	History,
}

impl Default for AppState {
	fn default() -> Self {
		Self {
			server_url: "localhost:3002".into(),
			api_key: String::new(),
			tls_cert_path: String::new(),
			connection_status: ConnectionStatus::Disconnected,
			client: None,

			config_file_path: None,
			network: String::new(),
			chain_source: ChainSourceConfig::default(),

			active_tab: ActiveTab::NodeInfo,

			node_info: None,
			balances: None,
			channels: None,
			payments: None,
			payments_page_token: None,
			payments_appending: false,
			peers: None,
			forwarded_payments: None,
			forwarded_payments_page_token: None,
			payment_details: None,
			price: None,
			stable_channels: None,
			settlement_kinds: None,
			ldk_log: None,

			onchain_address: None,
			generated_invoice: None,
			generated_offer: None,
			last_payment_id: None,
			last_txid: None,
			last_channel_id: None,

			tasks: AsyncTasks::default(),

			forms: Forms::default(),

			display_unit: DisplayUnit::default(),
			last_price_fetch: 0.0,
			auto_connect_pending: false,
			status_message: None,
			show_open_channel_dialog: false,
			show_close_channel_dialog: false,
			show_splice_in_dialog: false,
			show_splice_out_dialog: false,
			show_update_config_dialog: false,
			show_connect_peer_dialog: false,
			show_load_config_dialog: false,
			show_payment_details_dialog: false,
			payment_details_id: String::new(),
			config_paste_text: String::new(),
			lightning_tab: LightningTab::default(),
			onchain_tab: OnchainTab::default(),
			graph_channels: None,
			graph_channel_detail: None,
			graph_nodes: None,
			graph_node_detail: None,
			sign_result: None,
			verify_result: None,
			export_scores_result: None,
			show_disconnect_peer_dialog: false,
			disconnect_peer_pubkey: String::new(),
		}
	}
}
