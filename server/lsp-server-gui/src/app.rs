use std::sync::Arc;
use std::time::Duration;

use eframe::{egui, App, Frame};

#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Runtime;

use sc_rest_client::client::LspRestClient;
use sc_rest_client::ldk_server_grpc::api::{
	Bolt11ReceiveRequest, Bolt11SendRequest, Bolt12ReceiveRequest, Bolt12SendRequest,
	CloseChannelRequest, ConnectPeerRequest, DisconnectPeerRequest, ExportPathfindingScoresRequest,
	ForceCloseChannelRequest, GetBalancesRequest, GetNodeInfoRequest, GetPaymentDetailsRequest,
	GraphGetChannelRequest, GraphGetNodeRequest, GraphListChannelsRequest, GraphListNodesRequest,
	ListChannelsRequest, ListForwardedPaymentsRequest, ListPaymentsRequest, ListPeersRequest,
	OnchainReceiveRequest, OnchainSendRequest, OpenChannelRequest, SignMessageRequest,
	SpliceInRequest, SpliceOutRequest, SpontaneousSendRequest, UpdateChannelConfigRequest,
	VerifySignatureRequest,
};
use sc_rest_client::sc_protos::stable::{GetPriceRequest, ListSettlementPaymentsRequest, ListStableChannelsRequest, LogRequest};
use sc_rest_client::ldk_server_grpc::types::{
	bolt11_invoice_description, Bolt11InvoiceDescription, ChannelConfig,
};

#[cfg(not(target_arch = "wasm32"))]
use crate::config;
#[cfg(not(target_arch = "wasm32"))]
use crate::state::ChainSourceForm;
use crate::state::{ActiveTab, AppState, ConnectionStatus, StatusMessage};
use crate::task;
use crate::ui;

fn install_theme(ctx: &egui::Context) {
    const AMBER: egui::Color32 = egui::Color32::from_rgb(0xF7, 0x93, 0x1A); // bitcoin orange
    let mut visuals = egui::Visuals::dark();
    visuals.selection.bg_fill = egui::Color32::from_rgb(0x4A, 0x39, 0x14);
    visuals.selection.stroke = egui::Stroke::new(1.0, AMBER);
    visuals.hyperlink_color = AMBER;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, AMBER);
    visuals.widgets.hovered.expansion = 0.0;
    visuals.widgets.active.expansion = 0.0;
    let rounding = egui::Rounding::same(6.0);
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.active.rounding = rounding;
    visuals.window_rounding = egui::Rounding::same(8.0);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    ctx.set_style(style);
    ctx.set_pixels_per_point(1.1);
}

pub struct LspServerApp {
	pub state: AppState,
	#[cfg(not(target_arch = "wasm32"))]
	pub rt: Runtime,
}

impl LspServerApp {
	pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
		install_theme(&cc.egui_ctx);
		#[cfg(not(target_arch = "wasm32"))]
		let mut state = {
			let mut state = AppState::default();
			// Try to load config from file and populate connection settings
			if let Some(gui_config) = config::find_and_load_config() {
				state.server_url = gui_config.server_url;
				state.api_key = gui_config.api_key;
				state.tls_cert_path = gui_config.tls_cert_path;
				state.network = gui_config.network;
				state.forms.chain_source = ChainSourceForm::from_config(&gui_config.chain_source);
				state.chain_source = gui_config.chain_source;
				state.auto_connect_pending = true; // Auto-connect on first update
			}
			state
		};

		#[cfg(target_arch = "wasm32")]
		let mut state = AppState::default();

		if let Some(storage) = cc.storage {
			if let Some(u) = eframe::get_value::<crate::state::DisplayUnit>(storage, "display_unit") {
				state.display_unit = u;
			}
		}

		// First launch with no connection to auto-attempt: land on Settings so connecting is obvious.
		if !state.auto_connect_pending {
			state.active_tab = ActiveTab::Settings;
		}

		Self {
			state,
			#[cfg(not(target_arch = "wasm32"))]
			rt: Runtime::new().expect("Failed to create tokio runtime"),
		}
	}

	pub fn connect(&mut self) {
		let url = self.state.server_url.trim().to_string();
		let api_key = self.state.api_key.clone();

		#[cfg(not(target_arch = "wasm32"))]
		{
			let cert_path = self.state.tls_cert_path.trim().to_string();

			if url.is_empty() || api_key.is_empty() || cert_path.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Please fill in all connection fields"));
				return;
			}

			let cert_data = match std::fs::read(&cert_path) {
				Ok(data) => data,
				Err(e) => {
					self.state.status_message =
						Some(StatusMessage::error(format!("Failed to read TLS cert: {}", e)));
					return;
				},
			};

			let _rt_guard = self.rt.enter();
			match LspRestClient::new(url.clone(), api_key, &cert_data) {
				Ok(client) => {
					self.state.client = Some(Arc::new(client));
					self.state.connection_status = ConnectionStatus::Connected;
					self.state.status_message = Some(StatusMessage::success("Connected"));
					self.fetch_node_info();
					self.fetch_balances();
					self.fetch_channels();
					self.fetch_price();
				},
				Err(e) => {
					self.state.connection_status = ConnectionStatus::Error(e.clone());
					self.state.status_message = Some(StatusMessage::error(e));
				},
			}
		}

		#[cfg(target_arch = "wasm32")]
		{
			if url.is_empty() || api_key.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Please fill in server URL and API key"));
				return;
			}

			// On WASM, the browser handles TLS - no certificate needed
			match LspRestClient::new(url.clone(), api_key, &[]) {
				Ok(client) => {
					self.state.client = Some(Arc::new(client));
					self.state.connection_status = ConnectionStatus::Connected;
					self.state.status_message = Some(StatusMessage::success("Connected"));
					self.fetch_node_info();
					self.fetch_balances();
					self.fetch_channels();
					self.fetch_price();
				},
				Err(e) => {
					self.state.connection_status = ConnectionStatus::Error(e.clone());
					self.state.status_message = Some(StatusMessage::error(e));
				},
			}
		}
	}

	pub fn disconnect(&mut self) {
		self.state.client = None;
		self.state.connection_status = ConnectionStatus::Disconnected;
		self.state.node_info = None;
		self.state.balances = None;
		self.state.channels = None;
		self.state.payments = None;
		self.state.status_message = Some(StatusMessage::error("Disconnected"));
	}

	/// Spawns an async task using the appropriate runtime for the platform
	#[cfg(not(target_arch = "wasm32"))]
	fn spawn_task<T, F>(&self, future: F) -> task::ChannelTaskHandle<T>
	where
		T: Send + 'static,
		F: std::future::Future<Output = Result<T, String>> + Send + 'static,
	{
		task::spawn_with_runtime(&self.rt, future)
	}

	#[cfg(target_arch = "wasm32")]
	fn spawn_task<T, F>(&self, future: F) -> task::ChannelTaskHandle<T>
	where
		T: 'static,
		F: std::future::Future<Output = Result<T, String>> + 'static,
	{
		task::spawn_local(future)
	}

	pub fn fetch_node_info(&mut self) {
		if self.state.tasks.node_info.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.node_info = Some(self.spawn_task(async move {
				client.get_node_info(GetNodeInfoRequest {}).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_balances(&mut self) {
		if self.state.tasks.balances.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.balances = Some(self.spawn_task(async move {
				client.get_balances(GetBalancesRequest {}).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_channels(&mut self) {
		if self.state.tasks.channels.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.channels = Some(self.spawn_task(async move {
				client.list_channels(ListChannelsRequest {}).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_payments(&mut self) {
		if self.state.tasks.payments.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			let page_token = self.state.payments_page_token.clone();
			self.state.tasks.payments = Some(self.spawn_task(async move {
				client
					.list_payments(ListPaymentsRequest { page_token })
					.await
					.map_err(|e| e.to_string())
			}));
		}
		self.fetch_settlement_payments();
	}

	pub fn fetch_peers(&mut self) {
		if self.state.tasks.peers.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.peers = Some(self.spawn_task(async move {
				client.list_peers(ListPeersRequest {}).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_forwarded_payments(&mut self) {
		if self.state.tasks.forwarded_payments.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			let page_token = self.state.forwarded_payments_page_token.clone();
			self.state.tasks.forwarded_payments = Some(self.spawn_task(async move {
				client
					.list_forwarded_payments(ListForwardedPaymentsRequest { page_token })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_ldk_log(&mut self) {
		if self.state.tasks.ldk_log.is_some() {
			return;
		}
		let max_lines = self.state.forms.ldk_log.max_lines.parse::<u32>().unwrap_or(200);
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.ldk_log = Some(self.spawn_task(async move {
				client.ldk_log(LogRequest { max_lines, filter: String::new(), full: false }).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_audit_log(&mut self) {
		if self.state.tasks.audit_log.is_some() {
			return;
		}
		let max_lines = self.state.forms.audit_log.max_lines.parse::<u32>().unwrap_or(200);
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.audit_log = Some(self.spawn_task(async move {
				client.audit_log(LogRequest { max_lines, filter: String::new(), full: false }).await.map_err(|e| e.to_string())
			}));
		}
	}

	/// Complete history for the filtered id, oldest-first (server `full` mode, ignores max_lines).
	pub fn fetch_channel_history(&mut self) {
		if self.state.tasks.channel_history.is_some() {
			return;
		}
		let filter = self.state.forms.channel_history.filter.clone();
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.channel_history = Some(self.spawn_task(async move {
				client.audit_log(LogRequest { max_lines: 0, filter, full: true }).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_payment_details(&mut self, payment_id: String) {
		if self.state.tasks.payment_details.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.payment_details = Some(self.spawn_task(async move {
				client
					.get_payment_details(GetPaymentDetailsRequest { payment_id })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn generate_onchain_address(&mut self) {
		if self.state.tasks.onchain_receive.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.onchain_receive = Some(self.spawn_task(async move {
				client.onchain_receive(OnchainReceiveRequest {}).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn send_onchain(&mut self) {
		if self.state.tasks.onchain_send.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.onchain_send;
			let address = form.address.trim().to_string();
			let amount_sats = self.parse_amount_sats(&form.amount_sats);
			let send_all = if form.send_all { Some(true) } else { None };
			let fee_rate = form.fee_rate_sat_per_vb.trim().parse::<u64>().ok();

			if address.is_empty() {
				self.state.status_message = Some(StatusMessage::error("Address is required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.onchain_send = Some(self.spawn_task(async move {
				client
					.onchain_send(OnchainSendRequest {
						address,
						amount_sats,
						send_all,
						fee_rate_sat_per_vb: fee_rate,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn generate_bolt11_invoice(&mut self) {
		if self.state.tasks.bolt11_receive.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.bolt11_receive;
			let amount_msat = self.parse_amount_msat(&form.amount_msat);
			let description = form.description.trim().to_string();
			let expiry_secs = form.expiry_secs.trim().parse::<u32>().unwrap_or(86400);

			let invoice_description = if !description.is_empty() {
				Some(Bolt11InvoiceDescription {
					kind: Some(bolt11_invoice_description::Kind::Direct(description)),
				})
			} else {
				None
			};

			let client = client.clone();
			self.state.tasks.bolt11_receive = Some(self.spawn_task(async move {
				client
					.bolt11_receive(Bolt11ReceiveRequest {
						amount_msat,
						description: invoice_description,
						expiry_secs,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn send_bolt11(&mut self) {
		if self.state.tasks.bolt11_send.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.bolt11_send;
			let invoice = form.invoice.trim().to_string();
			let amount_msat = self.parse_amount_msat(&form.amount_msat);

			if invoice.is_empty() {
				self.state.status_message = Some(StatusMessage::error("Invoice is required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.bolt11_send = Some(self.spawn_task(async move {
				client
					.bolt11_send(Bolt11SendRequest { invoice, amount_msat, route_parameters: None })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn generate_bolt12_offer(&mut self) {
		if self.state.tasks.bolt12_receive.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.bolt12_receive;
			let description = form.description.trim().to_string();
			let amount_msat = self.parse_amount_msat(&form.amount_msat);
			let expiry_secs = form.expiry_secs.trim().parse::<u32>().ok();
			let quantity = form.quantity.trim().parse::<u64>().ok();

			if description.is_empty() {
				self.state.status_message = Some(StatusMessage::error("Description is required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.bolt12_receive = Some(self.spawn_task(async move {
				client
					.bolt12_receive(Bolt12ReceiveRequest {
						description,
						amount_msat,
						expiry_secs,
						quantity,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn send_bolt12(&mut self) {
		if self.state.tasks.bolt12_send.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.bolt12_send;
			let offer = form.offer.trim().to_string();
			let amount_msat = self.parse_amount_msat(&form.amount_msat);
			let quantity = form.quantity.trim().parse::<u64>().ok();
			let payer_note = if form.payer_note.trim().is_empty() {
				None
			} else {
				Some(form.payer_note.trim().to_string())
			};

			if offer.is_empty() {
				self.state.status_message = Some(StatusMessage::error("Offer is required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.bolt12_send = Some(self.spawn_task(async move {
				client
					.bolt12_send(Bolt12SendRequest {
						offer,
						amount_msat,
						quantity,
						payer_note,
						route_parameters: None,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn open_channel(&mut self) {
		if self.state.tasks.open_channel.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.open_channel;
			let node_pubkey = form.node_pubkey.trim().to_string();
			let address = form.address.trim().to_string();
			let channel_amount_sats = match self.parse_amount_sats(&form.channel_amount_sats) {
				Some(v) => v,
				None => {
					self.state.status_message =
						Some(StatusMessage::error("Invalid channel amount"));
					return;
				},
			};
			let push_to_counterparty_msat =
				self.parse_amount_msat(&form.push_to_counterparty_msat);
			let announce_channel = form.announce_channel;

			let channel_config = build_channel_config(
				form.forwarding_fee_proportional_millionths.trim(),
				form.forwarding_fee_base_msat.trim(),
				form.cltv_expiry_delta.trim(),
			);

			if node_pubkey.is_empty() || address.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Node pubkey and address are required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.open_channel = Some(self.spawn_task(async move {
				client
					.open_channel(OpenChannelRequest {
						node_pubkey,
						address,
						channel_amount_sats,
						push_to_counterparty_msat,
						channel_config,
						announce_channel,
						disable_counterparty_reserve: false,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn close_channel(&mut self) {
		if self.state.tasks.close_channel.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.close_channel;
			let user_channel_id = form.user_channel_id.trim().to_string();
			let counterparty_node_id = form.counterparty_node_id.trim().to_string();

			if user_channel_id.is_empty() || counterparty_node_id.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Channel ID and counterparty node ID are required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.close_channel = Some(self.spawn_task(async move {
				client
					.close_channel(CloseChannelRequest { user_channel_id, counterparty_node_id })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn force_close_channel(&mut self) {
		if self.state.tasks.force_close_channel.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.close_channel;
			let user_channel_id = form.user_channel_id.trim().to_string();
			let counterparty_node_id = form.counterparty_node_id.trim().to_string();
			let force_close_reason = if form.force_close_reason.trim().is_empty() {
				None
			} else {
				Some(form.force_close_reason.trim().to_string())
			};

			if user_channel_id.is_empty() || counterparty_node_id.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Channel ID and counterparty node ID are required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.force_close_channel = Some(self.spawn_task(async move {
				client
					.force_close_channel(ForceCloseChannelRequest {
						user_channel_id,
						counterparty_node_id,
						force_close_reason,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn splice_in(&mut self) {
		if self.state.tasks.splice_in.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.splice_in;
			let user_channel_id = form.user_channel_id.trim().to_string();
			let counterparty_node_id = form.counterparty_node_id.trim().to_string();
			let splice_amount_sats = match self.parse_amount_sats(&form.splice_amount_sats) {
				Some(v) => v,
				None => {
					self.state.status_message = Some(StatusMessage::error("Invalid splice amount"));
					return;
				},
			};

			if user_channel_id.is_empty() || counterparty_node_id.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Channel ID and counterparty node ID are required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.splice_in = Some(self.spawn_task(async move {
				client
					.splice_in(SpliceInRequest {
						user_channel_id,
						counterparty_node_id,
						splice_amount_sats,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn splice_out(&mut self) {
		if self.state.tasks.splice_out.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.splice_out;
			let user_channel_id = form.user_channel_id.trim().to_string();
			let counterparty_node_id = form.counterparty_node_id.trim().to_string();
			let splice_amount_sats = match self.parse_amount_sats(&form.splice_amount_sats) {
				Some(v) => v,
				None => {
					self.state.status_message = Some(StatusMessage::error("Invalid splice amount"));
					return;
				},
			};
			let address = if form.address.trim().is_empty() {
				None
			} else {
				Some(form.address.trim().to_string())
			};

			if user_channel_id.is_empty() || counterparty_node_id.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Channel ID and counterparty node ID are required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.splice_out = Some(self.spawn_task(async move {
				client
					.splice_out(SpliceOutRequest {
						user_channel_id,
						counterparty_node_id,
						address,
						splice_amount_sats,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn update_channel_config(&mut self) {
		if self.state.tasks.update_channel_config.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.update_channel_config;
			let user_channel_id = form.user_channel_id.trim().to_string();
			let counterparty_node_id = form.counterparty_node_id.trim().to_string();

			let channel_config = ChannelConfig {
				forwarding_fee_proportional_millionths: form
					.forwarding_fee_proportional_millionths
					.trim()
					.parse()
					.ok(),
				forwarding_fee_base_msat: form.forwarding_fee_base_msat.trim().parse().ok(),
				cltv_expiry_delta: form.cltv_expiry_delta.trim().parse().ok(),
				force_close_avoidance_max_fee_satoshis: None,
				accept_underpaying_htlcs: None,
				max_dust_htlc_exposure: None,
			};

			if user_channel_id.is_empty() || counterparty_node_id.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Channel ID and counterparty node ID are required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.update_channel_config = Some(self.spawn_task(async move {
				client
					.update_channel_config(UpdateChannelConfigRequest {
						user_channel_id,
						counterparty_node_id,
						channel_config: Some(channel_config),
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn connect_peer(&mut self) {
		if self.state.tasks.connect_peer.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.connect_peer;
			let node_pubkey = form.node_pubkey.trim().to_string();
			let address = form.address.trim().to_string();
			let persist = form.persist;

			if node_pubkey.is_empty() || address.is_empty() {
				self.state.status_message =
					Some(StatusMessage::error("Node pubkey and address are required"));
				return;
			}

			let client = client.clone();
			self.state.tasks.connect_peer = Some(self.spawn_task(async move {
				client
					.connect_peer(ConnectPeerRequest { node_pubkey, address, persist })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_price(&mut self) {
		if self.state.tasks.get_price.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.get_price = Some(self.spawn_task(async move {
				client.get_price(GetPriceRequest {}).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_stable_channels(&mut self) {
		if self.state.tasks.list_stable_channels.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.list_stable_channels = Some(self.spawn_task(async move {
				client
					.list_stable_channels(ListStableChannelsRequest {})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_settlement_payments(&mut self) {
		if self.state.tasks.list_settlement_payments.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.list_settlement_payments = Some(self.spawn_task(async move {
				client
					.list_settlement_payments(ListSettlementPaymentsRequest {})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn disconnect_peer(&mut self, node_pubkey: String) {
		if self.state.tasks.disconnect_peer.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.disconnect_peer = Some(self.spawn_task(async move {
				client
					.disconnect_peer(DisconnectPeerRequest { node_pubkey })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn spontaneous_send(&mut self) {
		if self.state.tasks.spontaneous_send.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.spontaneous_send;
			let amount_msat = match self.parse_amount_msat(&form.amount_msat) {
				Some(v) => v,
				None => {
					self.state.status_message = Some(StatusMessage::error("Invalid amount"));
					return;
				},
			};
			let node_id = form.node_id.trim().to_string();
			if node_id.is_empty() {
				self.state.status_message = Some(StatusMessage::error("Node ID is required"));
				return;
			}
			let client = client.clone();
			self.state.tasks.spontaneous_send = Some(self.spawn_task(async move {
				client
					.spontaneous_send(SpontaneousSendRequest {
						amount_msat,
						node_id,
						route_parameters: None,
						custom_tlvs: vec![],
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn sign_message(&mut self) {
		if self.state.tasks.sign_message.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let message = self.state.forms.sign_message.message.trim().to_string();
			if message.is_empty() {
				self.state.status_message = Some(StatusMessage::error("Message is required"));
				return;
			}
			let client = client.clone();
			let message_bytes = bytes::Bytes::from(message.into_bytes());
			self.state.tasks.sign_message = Some(self.spawn_task(async move {
				client
					.sign_message(SignMessageRequest { message: message_bytes })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn verify_signature(&mut self) {
		if self.state.tasks.verify_signature.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let form = &self.state.forms.verify_signature;
			let message = form.message.trim().to_string();
			let signature = form.signature.trim().to_string();
			let public_key = form.public_key.trim().to_string();
			if message.is_empty() || signature.is_empty() || public_key.is_empty() {
				self.state.status_message = Some(StatusMessage::error("All fields are required"));
				return;
			}
			let client = client.clone();
			let message_bytes = bytes::Bytes::from(message.into_bytes());
			self.state.tasks.verify_signature = Some(self.spawn_task(async move {
				client
					.verify_signature(VerifySignatureRequest {
						message: message_bytes,
						signature,
						public_key,
					})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_graph_channels(&mut self) {
		if self.state.tasks.graph_list_channels.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.graph_list_channels = Some(self.spawn_task(async move {
				client
					.graph_list_channels(GraphListChannelsRequest {})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_graph_channel(&mut self) {
		if self.state.tasks.graph_get_channel.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let scid =
				match self.state.forms.graph_get_channel.short_channel_id.trim().parse::<u64>() {
					Ok(v) => v,
					Err(_) => {
						self.state.status_message =
							Some(StatusMessage::error("Invalid short channel ID"));
						return;
					},
				};
			let client = client.clone();
			self.state.tasks.graph_get_channel = Some(self.spawn_task(async move {
				client
					.graph_get_channel(GraphGetChannelRequest { short_channel_id: scid })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_graph_nodes(&mut self) {
		if self.state.tasks.graph_list_nodes.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.graph_list_nodes = Some(self.spawn_task(async move {
				client.graph_list_nodes(GraphListNodesRequest {}).await.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn fetch_graph_node(&mut self) {
		if self.state.tasks.graph_get_node.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let node_id = self.state.forms.graph_get_node.node_id.trim().to_string();
			if node_id.is_empty() {
				self.state.status_message = Some(StatusMessage::error("Node ID is required"));
				return;
			}
			let client = client.clone();
			self.state.tasks.graph_get_node = Some(self.spawn_task(async move {
				client
					.graph_get_node(GraphGetNodeRequest { node_id })
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	pub fn export_pathfinding_scores(&mut self) {
		if self.state.tasks.export_pathfinding_scores.is_some() {
			return;
		}
		if let Some(client) = &self.state.client {
			let client = client.clone();
			self.state.tasks.export_pathfinding_scores = Some(self.spawn_task(async move {
				client
					.export_pathfinding_scores(ExportPathfindingScoresRequest {})
					.await
					.map_err(|e| e.to_string())
			}));
		}
	}

	/// Shared connection gate: when not connected, render the "not connected" state (Open
	/// Settings / Retry) and return true so the caller short-circuits. Returns false when connected.
	pub fn render_disconnected_gate(&mut self, ui: &mut egui::Ui) -> bool {
		if matches!(self.state.connection_status, ConnectionStatus::Connected) {
			return false;
		}
		match ui::widgets::not_connected(ui, &self.state.connection_status, &self.state.server_url) {
			ui::widgets::NotConnectedAction::OpenSettings => self.state.active_tab = ActiveTab::Settings,
			ui::widgets::NotConnectedAction::Retry => self.connect(),
			ui::widgets::NotConnectedAction::None => {},
		}
		true
	}

	pub fn fmt_sats(&self, sats: u64) -> String {
		ui::format_amount_sats(sats, self.state.display_unit, self.state.price.as_ref().map(|p| p.price))
	}
	pub fn fmt_msat(&self, msat: u64) -> String {
		ui::format_amount_msat(msat, self.state.display_unit, self.state.price.as_ref().map(|p| p.price))
	}

	fn price_value(&self) -> Option<f64> {
		self.state.price.as_ref().map(|p| p.price)
	}
	/// Parse an amount the user typed in the active display unit into sats.
	pub fn parse_amount_sats(&self, input: &str) -> Option<u64> {
		ui::parse_amount_to_sats(input, self.state.display_unit, self.price_value())
	}
	/// Same, scaled to msats for the Lightning APIs.
	pub fn parse_amount_msat(&self, input: &str) -> Option<u64> {
		ui::parse_amount_to_msat(input, self.state.display_unit, self.price_value())
	}
	/// Preview line under an amount field: the sats that will actually be sent
	/// (USD/BTC modes), or the ≈USD value (sats mode); None if unparseable.
	pub fn amount_entry_preview(&self, input: &str) -> Option<String> {
		let sats = self.parse_amount_sats(input)?;
		match self.state.display_unit {
			crate::state::DisplayUnit::Sats => match self.price_value() {
				Some(p) if p > 0.0 => {
					Some(format!("≈ {}", ui::format_amount_sats(sats, crate::state::DisplayUnit::Usd, self.price_value())))
				},
				_ => None,
			},
			_ => Some(format!("= {} sats", ui::format_sats(sats))),
		}
	}

	fn poll_tasks(&mut self, _ctx: &egui::Context) {
		macro_rules! poll_task {
			($task:expr => |$val:ident| $handler:expr) => {
				if let Some(t) = &mut $task {
					if let Some(res) = t.try_take() {
						$task = None;
						match res {
							Ok($val) => $handler,
							Err(e) => {
								self.state.status_message = Some(StatusMessage::error(e));
							},
						}
					}
					// Note: repaint is handled by update() with request_repaint_after()
				}
			};
		}

		poll_task!(self.state.tasks.node_info => |v| {
			self.state.node_info = Some(v);
		});

		poll_task!(self.state.tasks.balances => |v| {
			self.state.balances = Some(v);
		});

		poll_task!(self.state.tasks.channels => |v| {
			self.state.channels = Some(v);
		});

		poll_task!(self.state.tasks.payments => |v| {
			self.state.payments_page_token = v.next_page_token.clone();
			if self.state.payments_appending {
				// "Load More": append this page to the existing list (an empty page is a no-op).
				if let Some(existing) = self.state.payments.as_mut() {
					existing.payments.extend(v.payments);
					existing.next_page_token = v.next_page_token;
				} else {
					self.state.payments = Some(v);
				}
			} else {
				self.state.payments = Some(v);
			}
			self.state.payments_appending = false;
		});

		poll_task!(self.state.tasks.peers => |v| {
			self.state.peers = Some(v);
		});

		poll_task!(self.state.tasks.forwarded_payments => |v| {
			self.state.forwarded_payments_page_token = v.next_page_token.clone();
			self.state.forwarded_payments = Some(v);
		});

		poll_task!(self.state.tasks.ldk_log => |v| {
			self.state.ldk_log = Some(v);
		});

		poll_task!(self.state.tasks.audit_log => |v| {
			self.state.audit_log = Some(v);
		});

		poll_task!(self.state.tasks.channel_history => |v| {
			self.state.channel_history = Some(v);
		});

		poll_task!(self.state.tasks.payment_details => |v| {
			self.state.payment_details = Some(v);
		});

		poll_task!(self.state.tasks.onchain_receive => |v| {
			self.state.onchain_address = Some(v.address);
			self.state.status_message = Some(StatusMessage::success("Address generated"));
		});

		poll_task!(self.state.tasks.onchain_send => |v| {
			self.state.last_txid = Some(v.txid.clone());
			self.state.status_message = Some(StatusMessage::success(format!("Sent! TXID: {}", v.txid)));
			self.state.forms.onchain_send = Default::default();
		});

		poll_task!(self.state.tasks.bolt11_receive => |v| {
			self.state.generated_invoice = Some(v.invoice);
			self.state.status_message = Some(StatusMessage::success("Invoice generated"));
		});

		poll_task!(self.state.tasks.bolt11_send => |v| {
			self.state.last_payment_id = Some(v.payment_id.clone());
			self.state.status_message = Some(StatusMessage::success(format!("Payment sent! ID: {}", v.payment_id)));
			self.state.forms.bolt11_send = Default::default();
		});

		poll_task!(self.state.tasks.bolt12_receive => |v| {
			self.state.generated_offer = Some(v.offer);
			self.state.status_message = Some(StatusMessage::success("Offer generated"));
		});

		poll_task!(self.state.tasks.bolt12_send => |v| {
			self.state.last_payment_id = Some(v.payment_id.clone());
			self.state.status_message = Some(StatusMessage::success(format!("Payment sent! ID: {}", v.payment_id)));
			self.state.forms.bolt12_send = Default::default();
		});

		poll_task!(self.state.tasks.open_channel => |v| {
			self.state.last_channel_id = Some(v.user_channel_id.clone());
			self.state.status_message = Some(StatusMessage::success(format!("Channel opened! ID: {}", v.user_channel_id)));
			self.state.forms.open_channel = Default::default();
			self.state.show_open_channel_dialog = false;
			self.fetch_channels();
		});

		poll_task!(self.state.tasks.close_channel => |_v| {
			self.state.status_message = Some(StatusMessage::success("Channel close initiated"));
			self.state.forms.close_channel = Default::default();
			self.state.show_close_channel_dialog = false;
			self.fetch_channels();
		});

		poll_task!(self.state.tasks.force_close_channel => |_v| {
			self.state.status_message = Some(StatusMessage::success("Force close initiated"));
			self.state.forms.close_channel = Default::default();
			self.state.show_close_channel_dialog = false;
			self.fetch_channels();
		});

		poll_task!(self.state.tasks.splice_in => |_v| {
			self.state.status_message = Some(StatusMessage::success("Splice-in initiated"));
			self.state.forms.splice_in = Default::default();
			self.state.show_splice_in_dialog = false;
			self.fetch_channels();
		});

		poll_task!(self.state.tasks.splice_out => |v| {
			self.state.status_message = Some(StatusMessage::success(format!("Splice-out initiated to {}", v.address)));
			self.state.forms.splice_out = Default::default();
			self.state.show_splice_out_dialog = false;
			self.fetch_channels();
		});

		poll_task!(self.state.tasks.update_channel_config => |_v| {
			self.state.status_message = Some(StatusMessage::success("Channel config updated"));
			self.state.forms.update_channel_config = Default::default();
			self.state.show_update_config_dialog = false;
			self.fetch_channels();
		});

		poll_task!(self.state.tasks.connect_peer => |_v| {
			self.state.status_message = Some(StatusMessage::success("Peer connected successfully"));
			self.state.forms.connect_peer = Default::default();
			self.state.show_connect_peer_dialog = false;
		});

		// The 5s price poll doubles as a connectivity heartbeat: a successful GetPrice means the
		// LSP is reachable, an error means it is not — so the status badge reflects reality, not
		// just whether the client object was built. Recovers automatically when the LSP returns.
		if let Some(t) = &mut self.state.tasks.get_price {
			if let Some(res) = t.try_take() {
				self.state.tasks.get_price = None;
				match res {
					Ok(v) => {
						self.state.price = Some(v);
						self.state.connection_status = ConnectionStatus::Connected;
					},
					Err(e) => {
						self.state.connection_status =
							ConnectionStatus::Error(format!("LSP unreachable: {}", e));
					},
				}
			}
		}

		poll_task!(self.state.tasks.list_stable_channels => |v| {
			self.state.stable_channels = Some(v);
		});

		poll_task!(self.state.tasks.list_settlement_payments => |v| {
			self.state.settlement_kinds = Some(
				v.settlements
					.into_iter()
					.filter_map(|p| {
						crate::state::SettlementKind::parse(&p.kind).map(|k| (p.payment_id, k))
					})
					.collect(),
			);
		});

		poll_task!(self.state.tasks.disconnect_peer => |_v| {
			self.state.status_message = Some(StatusMessage::success("Peer disconnected"));
			self.state.show_disconnect_peer_dialog = false;
			self.fetch_peers();
		});

		poll_task!(self.state.tasks.spontaneous_send => |v| {
			self.state.last_payment_id = Some(v.payment_id.clone());
			self.state.status_message = Some(StatusMessage::success(format!("Keysend sent! ID: {}", v.payment_id)));
			self.state.forms.spontaneous_send = Default::default();
		});

		poll_task!(self.state.tasks.sign_message => |v| {
			self.state.sign_result = Some(v.signature.clone());
			self.state.status_message = Some(StatusMessage::success("Message signed"));
		});

		poll_task!(self.state.tasks.verify_signature => |v| {
			self.state.verify_result = Some(v.valid);
			if v.valid {
				self.state.status_message = Some(StatusMessage::success("Signature is valid"));
			} else {
				self.state.status_message = Some(StatusMessage::error("Signature is INVALID"));
			}
		});

		poll_task!(self.state.tasks.graph_list_channels => |v| {
			self.state.graph_channels = Some(v);
		});

		poll_task!(self.state.tasks.graph_get_channel => |v| {
			self.state.graph_channel_detail = Some(v);
		});

		poll_task!(self.state.tasks.graph_list_nodes => |v| {
			self.state.graph_nodes = Some(v);
		});

		poll_task!(self.state.tasks.graph_get_node => |v| {
			self.state.graph_node_detail = Some(v);
		});

		poll_task!(self.state.tasks.export_pathfinding_scores => |v| {
			let size = v.scores.len();
			self.state.export_scores_result = Some(v);
			self.state.status_message = Some(StatusMessage::success(format!("Exported pathfinding scores ({} bytes)", size)));
		});
	}
}

fn build_channel_config(fee_prop: &str, fee_base: &str, cltv: &str) -> Option<ChannelConfig> {
	let fee_prop = fee_prop.parse::<u32>().ok();
	let fee_base = fee_base.parse::<u32>().ok();
	let cltv = cltv.parse::<u32>().ok();

	if fee_prop.is_none() && fee_base.is_none() && cltv.is_none() {
		return None;
	}

	Some(ChannelConfig {
		forwarding_fee_proportional_millionths: fee_prop,
		forwarding_fee_base_msat: fee_base,
		cltv_expiry_delta: cltv,
		force_close_avoidance_max_fee_satoshis: None,
		accept_underpaying_htlcs: None,
		max_dust_htlc_exposure: None,
	})
}

impl App for LspServerApp {
	fn save(&mut self, storage: &mut dyn eframe::Storage) {
		eframe::set_value(storage, "display_unit", &self.state.display_unit);
	}

	fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
		self.poll_tasks(ctx);

		// Auto-connect if config was loaded at startup
		if self.state.auto_connect_pending {
			self.state.auto_connect_pending = false;
			self.connect();
		}

		if self.state.client.is_some() && self.state.tasks.get_price.is_none() {
			let now = ctx.input(|i| i.time);
			if now - self.state.last_price_fetch > 5.0 {
				self.state.last_price_fetch = now;
				self.fetch_price();
			}
		}

		// Keep repainting while connected so the periodic price refresh fires even when idle.
		if self.state.client.is_some() {
			ctx.request_repaint_after(Duration::from_secs(1));
		}

		egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
			ui.horizontal(|ui| {
				ui.heading("LSP Server");
				ui.separator();
				ui::connection::render_status(ui, &self.state);
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					let mut unit = self.state.display_unit;
					ui.selectable_value(&mut unit, crate::state::DisplayUnit::Sats, "Sats");
					ui.selectable_value(&mut unit, crate::state::DisplayUnit::Btc, "BTC");
					ui.selectable_value(&mut unit, crate::state::DisplayUnit::Usd, "USD");
					self.state.display_unit = unit;
					ui.separator();
					match &self.state.price {
						Some(p) if p.price > 0.0 => { ui.weak(format!("${:.0}/BTC", p.price)); }
						_ => { ui.weak("price --"); }
					}
				});
			});
		});

		egui::SidePanel::left("nav_panel").resizable(true).default_width(160.0).show(ctx, |ui| {
			egui::ScrollArea::vertical().show(ui, |ui| {
			ui.add_space(10.0);
			ui.heading("Navigation");
			ui.separator();

			let groups: [(&str, &[(ActiveTab, &str)]); 5] = [
				("Overview", &[(ActiveTab::NodeInfo, "🏠 Node Info"), (ActiveTab::Balances, "💰 Balances")]),
				("Lightning", &[(ActiveTab::Channels, "🔗 Channels"), (ActiveTab::StableChannels, "📈 Stable"), (ActiveTab::Peers, "👥 Peers"), (ActiveTab::Lightning, "⚡ Lightning"), (ActiveTab::Payments, "🧾 Payments"), (ActiveTab::ForwardedPayments, "↪ Forwarded")]),
				("On-chain", &[(ActiveTab::Onchain, "⛓ On-chain")]),
				("Network", &[(ActiveTab::NetworkGraph, "🌐 Graph")]),
				("System", &[(ActiveTab::Tools, "🛠 Tools"), (ActiveTab::Logs, "📜 Logs"), (ActiveTab::Settings, "⚙ Settings")]),
			];
			for (section, items) in groups {
				ui.add_space(6.0);
				ui.label(egui::RichText::new(section).small().weak());
				for (tab, label) in items {
					if ui.selectable_label(self.state.active_tab == *tab, *label).clicked() {
						self.state.active_tab = *tab;
					}
				}
			}

			ui.add_space(20.0);
			ui.separator();
			ui.label(egui::RichText::new("Documentation").small().strong());
			ui.add_space(5.0);

			ui.hyperlink_to("LDK Server", "https://github.com/lightningdevkit/ldk-server");
			ui.hyperlink_to("LDK Node", "https://docs.rs/ldk-node/latest/ldk_node/");
			ui.hyperlink_to("Rust Lightning", "https://docs.rs/lightning/latest/lightning/");
			ui.hyperlink_to("BDK", "https://docs.rs/bdk_wallet/latest/bdk_wallet/");
			});
		});

		egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
			ui.horizontal(|ui| {
				if let Some(msg) = &self.state.status_message {
					if msg.is_error {
						ui.colored_label(egui::Color32::RED, &msg.text);
					} else {
						ui.colored_label(egui::Color32::GREEN, &msg.text);
					}
				} else {
					ui.label("Ready");
				}
			});
		});

		egui::CentralPanel::default().show(ctx, |ui| {
			egui::ScrollArea::vertical().show(ui, |ui| match self.state.active_tab {
				ActiveTab::NodeInfo => ui::node_info::render(ui, self),
				ActiveTab::Balances => ui::balances::render(ui, self),
				ActiveTab::Channels => ui::channels::render(ui, self),
				ActiveTab::Peers => ui::peers::render(ui, self),
				ActiveTab::Payments => ui::payments::render(ui, self),
				ActiveTab::ForwardedPayments => ui::forwarded_payments::render(ui, self),
				ActiveTab::Lightning => ui::lightning::render(ui, self),
				ActiveTab::Onchain => ui::onchain::render(ui, self),
				ActiveTab::StableChannels => ui::stable_channels::render(ui, self),
				ActiveTab::Tools => ui::tools::render(ui, self),
				ActiveTab::NetworkGraph => ui::network_graph::render(ui, self),
				ActiveTab::Logs => ui::logs::render(ui, self),
				ActiveTab::Settings => ui::settings::render(ui, self),
			});
		});

		ui::channels::render_dialogs(ctx, self);
		ui::connection::render_load_config_dialog(ctx, self);
		ui::payments::render_dialogs(ctx, self);

		// A fetch spawned during this frame's render must schedule a near-term
		// repaint, else its background result isn't polled until the next event.
		if self.state.tasks.any_pending() {
			ctx.request_repaint_after(Duration::from_millis(50));
		}
	}
}
