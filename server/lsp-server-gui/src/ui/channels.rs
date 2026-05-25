use egui::{Context, ScrollArea, Ui};

use crate::app::LspServerApp;
use crate::state::ConnectionStatus;
use crate::ui::{format_msat, format_sats, truncate_id};

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Channels");
	ui.add_space(10.0);

	if !matches!(app.state.connection_status, ConnectionStatus::Connected) {
		ui.label("Connect to a server to view channels.");
		return;
	}

	ui.horizontal(|ui| {
		if app.state.tasks.channels.is_some() {
			ui.spinner();
			ui.label("Loading...");
		} else if ui.button("Refresh").clicked() {
			app.fetch_channels();
		}

		ui.separator();

		if ui.button("Connect Peer").clicked() {
			app.state.show_connect_peer_dialog = true;
		}

		if ui.button("Open Channel").clicked() {
			app.state.show_open_channel_dialog = true;
		}
	});

	ui.add_space(10.0);

	if let Some(channels_response) = &app.state.channels {
		let channels = &channels_response.channels;
		if channels.is_empty() {
			ui.label("No channels found.");
		} else {
			ui.label(format!("{} channel(s)", channels.len()));
			ui.add_space(5.0);

			ScrollArea::both().max_height(400.0).show(ui, |ui| {
				egui::Grid::new("channels_grid").striped(true).spacing([12.0, 6.0]).show(
					ui,
					|ui| {
						// Header
						ui.strong("Channel ID");
						ui.strong("Counterparty");
						ui.strong("Funding Tx");
						ui.strong("Capacity");
						ui.strong("Outbound");
						ui.strong("Inbound");
						ui.strong("Ready");
						ui.strong("Use");
						ui.strong("Actions");
						ui.end_row();

						for ch in channels {
							// Channel ID
							ui.horizontal(|ui| {
								ui.monospace(truncate_id(&ch.channel_id, 5, 4));
								if ui.small_button("Copy").clicked() {
									ui.output_mut(|o| o.copied_text = ch.channel_id.clone());
								}
							});

							// Counterparty
							ui.horizontal(|ui| {
								ui.monospace(truncate_id(&ch.counterparty_node_id, 5, 4));
								if ui.small_button("Copy").clicked() {
									ui.output_mut(|o| {
										o.copied_text = ch.counterparty_node_id.clone()
									});
								}
							});

							// Funding Txid
							ui.horizontal(|ui| {
								if let Some(ref funding_txo) = ch.funding_txo {
									ui.monospace(truncate_id(&funding_txo.txid, 5, 4));
									if ui.small_button("Copy").clicked() {
										ui.output_mut(|o| o.copied_text = funding_txo.txid.clone());
									}
								} else {
									ui.label("-");
								}
							});

							// Capacity
							ui.label(format!("{} sats", format_sats(ch.channel_value_sats)));

							// Outbound capacity
							ui.label(format_msat(ch.outbound_capacity_msat));

							// Inbound capacity
							ui.label(format_msat(ch.inbound_capacity_msat));

							// Ready
							ui.label(if ch.is_channel_ready { "Yes" } else { "No" });

							// Usable
							ui.label(if ch.is_usable { "Yes" } else { "No" });

							// Actions
							ui.horizontal(|ui| {
								if ui.small_button("Close").clicked() {
									app.state.forms.close_channel.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.close_channel.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_close_channel_dialog = true;
								}
								if ui.small_button("Splice+").clicked() {
									app.state.forms.splice_in.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.splice_in.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_splice_in_dialog = true;
								}
								if ui.small_button("Splice-").clicked() {
									app.state.forms.splice_out.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.splice_out.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_splice_out_dialog = true;
								}
								if ui.small_button("Config").clicked() {
									app.state.forms.update_channel_config.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.update_channel_config.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_update_config_dialog = true;
								}
							});

							ui.end_row();
						}
					},
				);
			});
		}
	} else {
		ui.label("No channel data available. Click Refresh to fetch.");
	}
}

pub fn render_dialogs(ctx: &Context, app: &mut LspServerApp) {
	render_connect_peer_dialog(ctx, app);
	render_open_channel_dialog(ctx, app);
	render_close_channel_dialog(ctx, app);
	render_splice_in_dialog(ctx, app);
	render_splice_out_dialog(ctx, app);
	render_update_config_dialog(ctx, app);
}

fn render_connect_peer_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_connect_peer_dialog {
		return;
	}

	egui::Window::new("Connect Peer").collapsible(false).resizable(false).show(ctx, |ui| {
		let form = &mut app.state.forms.connect_peer;

		ui.label("Connect to a Lightning Network peer");
		ui.add_space(5.0);

		egui::Grid::new("connect_peer_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Node Pubkey:");
			ui.text_edit_singleline(&mut form.node_pubkey);
			ui.end_row();

			ui.label("Address:");
			ui.text_edit_singleline(&mut form.address);
			ui.end_row();

			ui.label("Persist Connection:");
			ui.checkbox(&mut form.persist, "");
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.connect_peer.is_some();
			if is_pending {
				ui.spinner();
			} else if ui.button("Connect").clicked() {
				app.connect_peer();
			}
			if ui.button("Cancel").clicked() {
				app.state.show_connect_peer_dialog = false;
				app.state.forms.connect_peer = Default::default();
			}
		});
	});
}

fn render_open_channel_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_open_channel_dialog {
		return;
	}

	egui::Window::new("Open Channel").collapsible(false).resizable(false).show(ctx, |ui| {
		let form = &mut app.state.forms.open_channel;

		egui::Grid::new("open_channel_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Node Pubkey:");
			ui.text_edit_singleline(&mut form.node_pubkey);
			ui.end_row();

			ui.label("Address:");
			ui.text_edit_singleline(&mut form.address);
			ui.end_row();

			ui.label("Channel Amount (sats):");
			ui.text_edit_singleline(&mut form.channel_amount_sats);
			ui.end_row();

			ui.label("Push Amount (msat):");
			ui.text_edit_singleline(&mut form.push_to_counterparty_msat);
			ui.end_row();

			ui.label("Announce Channel:");
			ui.checkbox(&mut form.announce_channel, "");
			ui.end_row();
		});

		ui.collapsing("Advanced Options", |ui| {
			egui::Grid::new("open_channel_advanced_grid").num_columns(2).spacing([10.0, 5.0]).show(
				ui,
				|ui| {
					ui.label("Fee Proportional (millionths):");
					ui.text_edit_singleline(&mut form.forwarding_fee_proportional_millionths);
					ui.end_row();

					ui.label("Fee Base (msat):");
					ui.text_edit_singleline(&mut form.forwarding_fee_base_msat);
					ui.end_row();

					ui.label("CLTV Expiry Delta:");
					ui.text_edit_singleline(&mut form.cltv_expiry_delta);
					ui.end_row();
				},
			);
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.open_channel.is_some();
			if is_pending {
				ui.spinner();
			} else {
				if ui.button("Open Channel").clicked() {
					app.open_channel();
				}
			}
			if ui.button("Cancel").clicked() {
				app.state.show_open_channel_dialog = false;
				app.state.forms.open_channel = Default::default();
			}
		});
	});
}

fn render_close_channel_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_close_channel_dialog {
		return;
	}

	egui::Window::new("Close Channel").collapsible(false).resizable(false).show(ctx, |ui| {
		let form = &mut app.state.forms.close_channel;

		egui::Grid::new("close_channel_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Channel ID:");
			ui.text_edit_singleline(&mut form.user_channel_id);
			ui.end_row();

			ui.label("Counterparty:");
			ui.text_edit_singleline(&mut form.counterparty_node_id);
			ui.end_row();

			ui.label("Force Close Reason:");
			ui.text_edit_singleline(&mut form.force_close_reason);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_close_pending = app.state.tasks.close_channel.is_some();
			let is_force_close_pending = app.state.tasks.force_close_channel.is_some();

			if is_close_pending || is_force_close_pending {
				ui.spinner();
			} else {
				if ui.button("Close (Cooperative)").clicked() {
					app.close_channel();
				}
				if ui.button("Force Close").clicked() {
					app.force_close_channel();
				}
			}
			if ui.button("Cancel").clicked() {
				app.state.show_close_channel_dialog = false;
				app.state.forms.close_channel = Default::default();
			}
		});
	});
}

fn render_splice_in_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_splice_in_dialog {
		return;
	}

	egui::Window::new("Splice In").collapsible(false).resizable(false).show(ctx, |ui| {
		let form = &mut app.state.forms.splice_in;

		ui.label("Add funds to an existing channel");
		ui.add_space(5.0);

		egui::Grid::new("splice_in_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Channel ID:");
			ui.text_edit_singleline(&mut form.user_channel_id);
			ui.end_row();

			ui.label("Counterparty:");
			ui.text_edit_singleline(&mut form.counterparty_node_id);
			ui.end_row();

			ui.label("Amount (sats):");
			ui.text_edit_singleline(&mut form.splice_amount_sats);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.splice_in.is_some();
			if is_pending {
				ui.spinner();
			} else if ui.button("Splice In").clicked() {
				app.splice_in();
			}
			if ui.button("Cancel").clicked() {
				app.state.show_splice_in_dialog = false;
				app.state.forms.splice_in = Default::default();
			}
		});
	});
}

fn render_splice_out_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_splice_out_dialog {
		return;
	}

	egui::Window::new("Splice Out").collapsible(false).resizable(false).show(ctx, |ui| {
		let form = &mut app.state.forms.splice_out;

		ui.label("Remove funds from an existing channel");
		ui.add_space(5.0);

		egui::Grid::new("splice_out_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Channel ID:");
			ui.text_edit_singleline(&mut form.user_channel_id);
			ui.end_row();

			ui.label("Counterparty:");
			ui.text_edit_singleline(&mut form.counterparty_node_id);
			ui.end_row();

			ui.label("Amount (sats):");
			ui.text_edit_singleline(&mut form.splice_amount_sats);
			ui.end_row();

			ui.label("Address (optional):");
			ui.text_edit_singleline(&mut form.address);
			ui.end_row();
		});

		ui.add_space(10.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.splice_out.is_some();
			if is_pending {
				ui.spinner();
			} else if ui.button("Splice Out").clicked() {
				app.splice_out();
			}
			if ui.button("Cancel").clicked() {
				app.state.show_splice_out_dialog = false;
				app.state.forms.splice_out = Default::default();
			}
		});
	});
}

fn render_update_config_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_update_config_dialog {
		return;
	}

	egui::Window::new("Update Channel Config").collapsible(false).resizable(false).show(
		ctx,
		|ui| {
			let form = &mut app.state.forms.update_channel_config;

			egui::Grid::new("update_config_grid").num_columns(2).spacing([10.0, 5.0]).show(
				ui,
				|ui| {
					ui.label("Channel ID:");
					ui.text_edit_singleline(&mut form.user_channel_id);
					ui.end_row();

					ui.label("Counterparty:");
					ui.text_edit_singleline(&mut form.counterparty_node_id);
					ui.end_row();

					ui.label("Fee Proportional (millionths):");
					ui.text_edit_singleline(&mut form.forwarding_fee_proportional_millionths);
					ui.end_row();

					ui.label("Fee Base (msat):");
					ui.text_edit_singleline(&mut form.forwarding_fee_base_msat);
					ui.end_row();

					ui.label("CLTV Expiry Delta:");
					ui.text_edit_singleline(&mut form.cltv_expiry_delta);
					ui.end_row();
				},
			);

			ui.add_space(10.0);

			ui.horizontal(|ui| {
				let is_pending = app.state.tasks.update_channel_config.is_some();
				if is_pending {
					ui.spinner();
				} else if ui.button("Update Config").clicked() {
					app.update_channel_config();
				}
				if ui.button("Cancel").clicked() {
					app.state.show_update_config_dialog = false;
					app.state.forms.update_channel_config = Default::default();
				}
			});
		},
	);
}
