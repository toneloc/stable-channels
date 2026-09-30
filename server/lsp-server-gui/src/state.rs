use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dioxus::prelude::*;
use sc_rest_client::client::LspRestClient;
use sc_rest_client::ldk_server_grpc::api::{
	ExportPathfindingScoresResponse, GetBalancesResponse, GetNodeInfoResponse,
	GetPaymentDetailsResponse, GraphGetChannelResponse, GraphGetNodeResponse,
	GraphListChannelsResponse, GraphListNodesResponse, ListChannelsResponse,
	ListForwardedPaymentsResponse, ListPaymentsResponse, ListPeersResponse,
};
use sc_rest_client::sc_protos::revenue::{GetRevenueResponse, RevenueItem};
use sc_rest_client::sc_protos::stable::{
	GetPriceResponse, ListChannelLedgerEventsResponse, ListStableChannelsResponse, LogResponse,
};

use crate::config::{ChainSourceConfig, ChainSourceType};

#[derive(Clone, PartialEq, Default)]
pub enum ConnectionStatus {
	#[default]
	Disconnected,
	Connected,
	Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ActiveTab {
	#[default]
	Overview,
	NodeInfo,
	Balances,
	Revenue,
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

impl ActiveTab {
	/// Sidebar groups in display order.
	pub const GROUPS: [(&'static str, &'static [ActiveTab]); 5] = [
		("Overview", &[ActiveTab::Overview, ActiveTab::NodeInfo, ActiveTab::Balances, ActiveTab::Revenue]),
		(
			"Lightning",
			&[
				ActiveTab::Channels,
				ActiveTab::StableChannels,
				ActiveTab::Peers,
				ActiveTab::Lightning,
				ActiveTab::Payments,
				ActiveTab::ForwardedPayments,
			],
		),
		("On-chain", &[ActiveTab::Onchain]),
		("Network", &[ActiveTab::NetworkGraph]),
		("System", &[ActiveTab::Tools, ActiveTab::Logs, ActiveTab::Settings]),
	];

	pub fn label(self) -> &'static str {
		match self {
			ActiveTab::Overview => "Overview",
			ActiveTab::NodeInfo => "Node Info",
			ActiveTab::Balances => "Balances",
			ActiveTab::Revenue => "Revenue",
			ActiveTab::Channels => "Channels",
			ActiveTab::Peers => "Peers",
			ActiveTab::Payments => "Payments",
			ActiveTab::ForwardedPayments => "Forwarded",
			ActiveTab::Lightning => "Lightning",
			ActiveTab::Onchain => "On-chain",
			ActiveTab::StableChannels => "Stable",
			ActiveTab::Tools => "Tools",
			ActiveTab::NetworkGraph => "Graph",
			ActiveTab::Logs => "Logs",
			ActiveTab::Settings => "Settings",
		}
	}

	pub fn title(self) -> &'static str {
		match self {
			ActiveTab::NodeInfo => "Node Information",
			ActiveTab::Lightning => "Lightning Payments",
			ActiveTab::Onchain => "On-chain",
			ActiveTab::StableChannels => "Stable Channels",
			ActiveTab::ForwardedPayments => "Forwarded Payments",
			ActiveTab::NetworkGraph => "Network Graph",
			other => other.label(),
		}
	}

	pub fn subtitle(self) -> &'static str {
		match self {
			ActiveTab::Overview => "What needs attention right now",
			ActiveTab::NodeInfo => "Identity, chain source and sync status of the LSP node",
			ActiveTab::Balances => "On-chain and Lightning funds held by the node",
			ActiveTab::Revenue => "What the LSP earned, spent and settled for the peg",
			ActiveTab::Channels => "Open, splice, configure and close Lightning channels",
			ActiveTab::Peers => "Lightning peers this node is connected to",
			ActiveTab::Payments => "Every payment sent or received by the node",
			ActiveTab::ForwardedPayments => "Payments routed through this node and the fees earned",
			ActiveTab::Lightning => "Send and receive over Lightning",
			ActiveTab::Onchain => "Send, receive and review on-chain funds",
			ActiveTab::StableChannels => "USD-stabilized channels and their targets",
			ActiveTab::Tools => "Message signing and router utilities",
			ActiveTab::NetworkGraph => "Public channels and nodes known from gossip",
			ActiveTab::Logs => "Audit trail, channel ledger and LDK server logs",
			ActiveTab::Settings => "Connection, display unit and appearance",
		}
	}

	pub fn icon(self) -> &'static str {
		match self {
			ActiveTab::Overview => "activity",
			ActiveTab::NodeInfo => "home",
			ActiveTab::Balances => "wallet",
			ActiveTab::Revenue => "coins",
			ActiveTab::Channels => "link",
			ActiveTab::Peers => "users",
			ActiveTab::Payments => "receipt",
			ActiveTab::ForwardedPayments => "forward",
			ActiveTab::Lightning => "zap",
			ActiveTab::Onchain => "cube",
			ActiveTab::StableChannels => "shield",
			ActiveTab::Tools => "wrench",
			ActiveTab::NetworkGraph => "graph",
			ActiveTab::Logs => "file",
			ActiveTab::Settings => "sliders",
		}
	}
}

/// Time window of the Revenue tab.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum RevenueWindow {
	Today,
	#[default]
	Week,
	Month,
	All,
}

impl RevenueWindow {
	pub const ALL: [RevenueWindow; 4] = [RevenueWindow::Today, RevenueWindow::Week, RevenueWindow::Month, RevenueWindow::All];

	pub fn label(self) -> &'static str {
		match self {
			RevenueWindow::Today => "Today",
			RevenueWindow::Week => "7 days",
			RevenueWindow::Month => "30 days",
			RevenueWindow::All => "All",
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum DisplayUnit {
	#[default]
	Usd,
	Btc,
	Sats,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Theme {
	#[default]
	System,
	Light,
	Dark,
}

/// Persisted per-user preferences.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Prefs {
	#[serde(default)]
	pub display_unit: DisplayUnit,
	#[serde(default)]
	pub theme: Theme,
	/// Refresh the visible page in the background.
	#[serde(default = "default_true")]
	pub auto_refresh: bool,
}

fn default_true() -> bool {
	true
}

impl Default for Prefs {
	fn default() -> Self {
		Self { display_unit: DisplayUnit::default(), theme: Theme::default(), auto_refresh: true }
	}
}

/// Stable-channel settlement classification recorded by the daemon, keyed by payment_id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettlementKind {
	Stability,
	Trade,
	Sync,
}

impl SettlementKind {
	/// Parse the daemon's `kind` string; unknown strings are ignored.
	pub fn parse(s: &str) -> Option<Self> {
		match s {
			"stability" => Some(Self::Stability),
			"trade" => Some(Self::Trade),
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
pub struct EditStableChannelForm {
	pub channel_id: String,
	pub expected_usd: String,
	pub note: String,
}

#[derive(Clone)]
pub struct ChannelLedgerForm {
	pub identifier: String,
	pub category: String,
	pub status: String,
	pub completeness: String,
    pub newest_first: bool,
    pub show_technical: bool,
}

impl Default for ChannelLedgerForm {
    fn default() -> Self {
        Self {
            identifier: String::new(),
            category: String::new(),
            status: String::new(),
            completeness: String::new(),
            newest_first: true,
            show_technical: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelLedgerRequestKey {
    pub identifier: String,
    pub category: String,
    pub status: String,
    pub completeness: String,
    pub show_technical: bool,
}

impl From<&ChannelLedgerForm> for ChannelLedgerRequestKey {
    fn from(form: &ChannelLedgerForm) -> Self {
        Self {
            identifier: form.identifier.trim().to_owned(),
            category: form.category.clone(),
            status: form.status.clone(),
            completeness: form.completeness.clone(),
            show_technical: form.show_technical,
        }
    }
}


/// The rejected trade fee the refund dialog is about to send back.
#[derive(Default, Clone)]
pub struct RefundTradeFeeForm {
	pub trade_payment_id: String,
	pub amount_msat: u64,
	pub node_id: String,
}

#[derive(Default, Clone)]
pub struct Forms {
	pub open_channel: OpenChannelForm,
	pub bolt11_receive: Bolt11ReceiveForm,
	pub edit_stable_channel: EditStableChannelForm,
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
	pub audit_log: LogForm,
	pub channel_ledger: ChannelLedgerForm,
	pub chain_source: ChainSourceForm,
	pub refund_trade_fee: RefundTradeFeeForm,
}

#[derive(Clone, PartialEq, Debug)]
pub struct StatusMessage {
	pub text: String,
	pub is_error: bool,
	/// Distinguishes consecutive identical messages so the toast timer restarts.
	pub id: u64,
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

#[derive(Clone, Copy, PartialEq, Default)]
pub enum LogsTab {
	#[default]
	Audit,
	ChannelLedger,
	Ldk,
}

/// Connection settings and the live client.
#[derive(Clone, Default)]
pub struct Connection {
	pub server_url: String,
	pub api_key: String,
	pub tls_cert_path: String,
	pub status: ConnectionStatus,
	pub client: Option<Arc<LspRestClient>>,
	pub config_file_path: Option<String>,
	pub network: String,
	pub chain_source: ChainSourceConfig,
}

impl Connection {
	pub fn new() -> Self {
		Self { server_url: "localhost:3002".into(), ..Default::default() }
	}
}

#[derive(Clone, Copy, PartialEq, Default)]
pub struct Nav {
	pub active_tab: ActiveTab,
	pub lightning_tab: LightningTab,
	pub onchain_tab: OnchainTab,
	pub logs_tab: LogsTab,
}

/// Cached daemon responses.
#[derive(Clone, Default)]
pub struct Data {
	pub node_info: Option<GetNodeInfoResponse>,
	pub balances: Option<GetBalancesResponse>,
	pub channels: Option<ListChannelsResponse>,
	pub payments: Option<ListPaymentsResponse>,
	pub payments_page_token: Option<String>,
	pub peers: Option<ListPeersResponse>,
	pub forwarded_payments: Option<ListForwardedPaymentsResponse>,
	pub forwarded_payments_page_token: Option<String>,
	pub payment_details: Option<GetPaymentDetailsResponse>,
	pub stable_channels: Option<ListStableChannelsResponse>,
	/// payment_id (hex) -> settlement classification, fetched alongside payments.
	pub settlement_kinds: Option<HashMap<String, SettlementKind>>,
	pub ldk_log: Option<LogResponse>,
	pub audit_log: Option<LogResponse>,
	pub channel_ledger: Option<ListChannelLedgerEventsResponse>,
	pub channel_ledger_cursor: Option<String>,
	pub graph_channels: Option<GraphListChannelsResponse>,
	pub graph_channel_detail: Option<GraphGetChannelResponse>,
	pub graph_nodes: Option<GraphListNodesResponse>,
	pub graph_node_detail: Option<GraphGetNodeResponse>,
	/// Payments pages currently loaded (auto-refresh only replaces a single first page).
	pub payments_pages: usize,
	/// Pages fetched so far by an in-flight "Load all".
	pub payments_load_all_progress: Option<usize>,
	/// node_id -> gossip alias (None when the node has no public announcement).
	pub aliases: HashMap<String, Option<String>>,
	pub alias_queue: Vec<String>,
	/// Unix seconds of each operation's last successful response.
	pub fetched_at: HashMap<Op, u64>,
	pub revenue: Option<GetRevenueResponse>,
	/// Revenue activity rows loaded so far (first page plus "Load more").
	pub revenue_items: Vec<RevenueItem>,
	pub revenue_cursor: Option<String>,
	pub revenue_pages: usize,
	/// Revenue of the last 7 days for the Overview tile, independent of the Revenue tab's window.
	pub revenue_week: Option<GetRevenueResponse>,
	pub revenue_week_error: Option<String>,
	/// Newest channel-state events across every channel (the Overview feed), pages so far.
	pub activity: Option<ListChannelLedgerEventsResponse>,
	pub activity_cursor: Option<String>,
	pub activity_pages: usize,
	/// Why the last feed request failed; cleared on success.
	pub activity_error: Option<String>,
	/// The daemon refused the feed (or the route is missing): stop asking until the next connect.
	pub activity_unsupported: bool,
	pub revenue_week_unsupported: bool,
}

impl Data {
	/// Gossip alias for a node, when known.
	pub fn alias(&self, node_id: &str) -> Option<String> {
		self.aliases.get(node_id).cloned().flatten()
	}
}

/// Outputs of user-initiated operations.
#[derive(Clone, Default)]
pub struct Results {
	pub onchain_address: Option<String>,
	pub generated_invoice: Option<String>,
	pub generated_offer: Option<String>,
	pub last_payment_id: Option<String>,
	pub last_txid: Option<String>,
	pub sign_result: Option<String>,
	pub verify_result: Option<bool>,
	pub export_scores_result: Option<ExportPathfindingScoresResponse>,
}

/// Column a table is sorted by, with direction (true = descending).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort<C> {
	pub column: C,
	pub descending: bool,
}

impl<C: PartialEq + Copy> Sort<C> {
	/// Clicking the active column flips direction; a new column starts descending.
	pub fn toggled(self, column: C) -> Self {
		if self.column == column {
			Self { column, descending: !self.descending }
		} else {
			Self { column, descending: true }
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChannelSortColumn {
	Capacity,
	Outbound,
	Inbound,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChannelStatusFilter {
	All,
	Ready,
	Usable,
	Pending,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaymentSortColumn {
	Amount,
	Timestamp,
}

/// Log viewer controls (one per log).
#[derive(Clone, PartialEq)]
pub struct LogViewState {
	pub filter: String,
	pub wrap: bool,
	pub follow: bool,
}

impl Default for LogViewState {
	fn default() -> Self {
		Self { filter: String::new(), wrap: false, follow: true }
	}
}

/// Session-only UI state that must survive tab switches (filters, sorts, confirmations).
#[derive(Clone, PartialEq)]
pub struct ViewState {
	pub channel_filter: String,
	pub channel_status: ChannelStatusFilter,
	pub channel_sort: Sort<ChannelSortColumn>,
	pub payment_filter: String,
	/// -1 = all, else matches payment.status
	pub payment_status: i32,
	/// -1 = all, else matches payment.direction
	pub payment_direction: i32,
	pub payment_sort: Sort<PaymentSortColumn>,
	pub scid_filter: String,
	pub node_filter: String,
	pub audit_view: LogViewState,
	pub ldk_view: LogViewState,
	pub force_close_confirm: bool,
	pub splice_out_confirm: bool,
	pub send_all_confirm: bool,
	pub config_paste_text: String,
	/// Payment type filter ("" = all).
	pub payment_type: String,
	pub revenue_window: RevenueWindow,
	/// Revenue activity category filter (empty = all).
	pub revenue_categories: Vec<String>,
	/// Stable tab also lists channels without a USD target (routing peers, bitcoin-only wallets).
	pub stable_show_unpositioned: bool,
	/// Overview activity widened from 24 h to 7 days by "Show more".
	pub activity_extended: bool,
}

impl Default for ViewState {
	fn default() -> Self {
		Self {
			channel_filter: String::new(),
			channel_status: ChannelStatusFilter::All,
			channel_sort: Sort { column: ChannelSortColumn::Capacity, descending: true },
			payment_filter: String::new(),
			payment_status: -1,
			payment_direction: -1,
			payment_sort: Sort { column: PaymentSortColumn::Timestamp, descending: true },
			scid_filter: String::new(),
			node_filter: String::new(),
			audit_view: LogViewState::default(),
			ldk_view: LogViewState::default(),
			force_close_confirm: false,
			splice_out_confirm: false,
			send_all_confirm: false,
			config_paste_text: String::new(),
			payment_type: String::new(),
			revenue_window: RevenueWindow::default(),
			revenue_categories: Vec::new(),
			stable_show_unpositioned: false,
			activity_extended: false,
		}
	}
}

/// Every daemon operation; at most one of each runs at a time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Op {
	NodeInfo,
	Balances,
	Channels,
	Payments,
	Peers,
	ForwardedPayments,
	PaymentDetails,
	OnchainReceive,
	OnchainSend,
	Bolt11Receive,
	Bolt11Send,
	Bolt12Receive,
	Bolt12Send,
	OpenChannel,
	CloseChannel,
	ForceCloseChannel,
	SpliceIn,
	SpliceOut,
	UpdateChannelConfig,
	ConnectPeer,
	DisconnectPeer,
	SpontaneousSend,
	SignMessage,
	VerifySignature,
	GraphListChannels,
	GraphGetChannel,
	GraphListNodes,
	GraphGetNode,
	ExportPathfindingScores,
	GetPrice,
	ListStableChannels,
	EditStableChannel,
	ListSettlementPayments,
	GetRevenue,
	RefundTradeFee,
	LdkLog,
	AuditLog,
	ChannelLedger,
	ChannelLedgerExport,
	ActivityFeed,
	RevenueWeek,
	/// Native file dialogs (config load/save, certificate browse).
	FileDialog,
	/// Background gossip alias lookups.
	Aliases,
	/// Fetching every remaining payments page.
	PaymentsAll,
	/// Exporting a table (native save dialog).
	Export,
}

#[derive(Clone, Default, PartialEq)]
pub struct Pending(HashSet<Op>);

impl Pending {
	pub fn has(&self, op: Op) -> bool {
		self.0.contains(&op)
	}
	pub fn insert(&mut self, op: Op) {
		self.0.insert(op);
	}
	pub fn remove(&mut self, op: Op) {
		self.0.remove(&op);
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialog {
	OpenChannel,
	CloseChannel,
	SpliceIn,
	SpliceOut,
	UpdateChannelConfig,
	ConnectPeer,
	/// Web-only paste-a-config dialog.
	#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
	LoadConfig,
	/// Review step shown before any payment leaves the node.
	ConfirmSend(SendKind),
	/// Confirm step before a rejected trade's fee is sent back.
	RefundTradeFee,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SendKind {
	Bolt11,
	Bolt12,
	Keysend,
	Onchain,
}

/// Side panel with the full record behind a table row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Drawer {
	Payment(String),
	/// Keyed by user_channel_id, which stays stable across splices.
	Channel(String),
}

/// Hover tooltip anchored at the pointer position (client coordinates).
#[derive(Clone, PartialEq)]
pub struct Tooltip {
	pub text: String,
	pub x: f64,
	pub y: f64,
}

/// App-wide reactive state, provided once at the root and read with `use_context`.
#[derive(Clone, Copy)]
pub struct AppCtx {
	pub conn: Signal<Connection>,
	pub prefs: Signal<Prefs>,
	pub nav: Signal<Nav>,
	pub status: Signal<Option<StatusMessage>>,
	pub price: Signal<Option<GetPriceResponse>>,
	pub now: Signal<u64>,
	pub data: Signal<Data>,
	pub results: Signal<Results>,
	pub forms: Signal<Forms>,
	pub view: Signal<ViewState>,
	pub pending: Signal<Pending>,
	pub dialog: Signal<Option<Dialog>>,
	pub drawer: Signal<Option<Drawer>>,
	pub tooltip: Signal<Option<Tooltip>>,
	/// Bumped whenever ledger identifier/filters change; stale ledger results are dropped.
	pub ledger_gen: Signal<u64>,
	pub status_seq: Signal<u64>,
	/// True when a daemon config was found at startup (native only).
	pub auto_connect: Signal<bool>,
}

impl AppCtx {
	/// Create every signal; must run inside a component (e.g. in `use_context_provider`).
	pub fn new(conn: Connection, forms: Forms, nav: Nav, prefs: Prefs, auto_connect: bool) -> Self {
		Self {
			conn: Signal::new(conn),
			prefs: Signal::new(prefs),
			nav: Signal::new(nav),
			status: Signal::new(None),
			price: Signal::new(None),
			now: Signal::new(crate::format::now_secs()),
			data: Signal::new(Data::default()),
			results: Signal::new(Results::default()),
			forms: Signal::new(forms),
			view: Signal::new(ViewState::default()),
			pending: Signal::new(Pending::default()),
			dialog: Signal::new(None),
			drawer: Signal::new(None),
			tooltip: Signal::new(None),
			ledger_gen: Signal::new(0),
			status_seq: Signal::new(0),
			auto_connect: Signal::new(auto_connect),
		}
	}

	pub fn busy(&self, op: Op) -> bool {
		self.pending.read().has(op)
	}

	/// Connection check that does not subscribe the caller to changes.
	pub fn is_connected_peek(&self) -> bool {
		matches!(self.conn.peek().status, ConnectionStatus::Connected)
	}

	pub fn is_connected(&self) -> bool {
		matches!(self.conn.read().status, ConnectionStatus::Connected)
	}

	pub fn client(&self) -> Option<Arc<LspRestClient>> {
		self.conn.peek().client.clone()
	}

	pub fn unit(&self) -> DisplayUnit {
		self.prefs.read().display_unit
	}

	pub fn price_value(&self) -> Option<f64> {
		self.price.read().as_ref().map(|p| p.price)
	}

	pub fn fmt_sats(&self, sats: u64) -> String {
		crate::format::format_amount_sats(sats, self.unit(), self.price_value())
	}

	pub fn fmt_msat(&self, msat: u64) -> String {
		crate::format::format_amount_msat(msat, self.unit(), self.price_value())
	}

	/// Parse an amount typed in the active display unit into sats.
	pub fn parse_amount_sats(&self, input: &str) -> Option<u64> {
		crate::format::parse_amount_to_sats(input, self.prefs.peek().display_unit, self.price_peek())
	}

	/// Same, scaled to msats for the Lightning APIs.
	pub fn parse_amount_msat(&self, input: &str) -> Option<u64> {
		crate::format::parse_amount_to_msat(input, self.prefs.peek().display_unit, self.price_peek())
	}

	fn price_peek(&self) -> Option<f64> {
		self.price.peek().as_ref().map(|p| p.price)
	}

	pub fn success(mut self, text: impl Into<String>) {
		self.push_status(text.into(), false);
	}

	pub fn error(mut self, text: impl Into<String>) {
		self.push_status(text.into(), true);
	}

	fn push_status(&mut self, text: String, is_error: bool) {
		let id = *self.status_seq.peek() + 1;
		self.status_seq.set(id);
		self.status.set(Some(StatusMessage { text, is_error, id }));
	}
}
