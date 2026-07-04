use egui::{Color32, Context, ScrollArea, Ui};

use crate::app::LspServerApp;
use crate::ui::widgets;

// Liquidity bar colors: amber = outbound (local funds), blue = inbound (remote funds).
const OUTBOUND_COLOR: Color32 = Color32::from_rgb(0xF7, 0x93, 0x1A);
const INBOUND_COLOR: Color32 = Color32::from_rgb(0x3D, 0x84, 0xC7);

pub fn render(ui: &mut Ui, app: &mut LspServerApp) {
	ui.heading("Channels");
	ui.add_space(10.0);

	if app.render_disconnected_gate(ui) {
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

	// Pre-extract per-row data into locals so the &app.state.channels borrow is
	// released before we call app.fmt_* / borrow &mut app.state.status_message below.
	let rows: Option<Vec<ChannelRow>> = app.state.channels.as_ref().map(|resp| {
		resp.channels
			.iter()
			.map(|ch| ChannelRow {
				channel_id: ch.channel_id.clone(),
				counterparty_node_id: ch.counterparty_node_id.clone(),
				user_channel_id: ch.user_channel_id.clone(),
				funding_txid: ch.funding_txo.as_ref().map(|f| f.txid.clone()),
				channel_value_sats: ch.channel_value_sats,
				outbound_capacity_msat: ch.outbound_capacity_msat,
				inbound_capacity_msat: ch.inbound_capacity_msat,
				is_channel_ready: ch.is_channel_ready,
				is_usable: ch.is_usable,
			})
			.collect()
	});

	if let Some(rows) = rows {
		if rows.is_empty() {
			ui.label("No channels found.");
		} else {
			// Filter controls live in egui temp memory (not persisted).
			let filter_id = ui.id().with("chan_filter");
			let status_id = ui.id().with("chan_status");
			let sort_id = ui.id().with("chan_sort");
			let mut filter =
				ui.memory_mut(|m| m.data.get_temp::<String>(filter_id).unwrap_or_default());
			// status filter: -1 = all, 0 = ready, 1 = usable, 2 = pending
			let mut status_filter =
				ui.memory_mut(|m| m.data.get_temp::<i32>(status_id).unwrap_or(-1));
			// sort key: (column, descending) where 0 = Capacity, 1 = Outbound, 2 = Inbound
			let mut sort =
				ui.memory_mut(|m| m.data.get_temp::<(u8, bool)>(sort_id).unwrap_or((0, true)));

			ui.horizontal(|ui| {
				ui.label("Filter:");
				ui.add(
					egui::TextEdit::singleline(&mut filter)
						.hint_text("channel id, user channel id, or counterparty"),
				);
				egui::ComboBox::from_id_salt(status_id)
					.selected_text(channel_status_label(status_filter))
					.show_ui(ui, |ui| {
						ui.selectable_value(&mut status_filter, -1, "All");
						ui.selectable_value(&mut status_filter, 0, "Ready");
						ui.selectable_value(&mut status_filter, 1, "Usable");
						ui.selectable_value(&mut status_filter, 2, "Pending");
					});
			});

			let needle = filter.trim().to_lowercase();
			let mut view: Vec<usize> = (0..rows.len())
				.filter(|&i| {
					let r = &rows[i];
					let matches_text = needle.is_empty()
						|| r.channel_id.to_lowercase().contains(&needle)
						|| r.user_channel_id.to_lowercase().contains(&needle)
						|| r.counterparty_node_id.to_lowercase().contains(&needle);
					let matches_status = match status_filter {
						0 => r.is_channel_ready,
						1 => r.is_usable,
						2 => !r.is_channel_ready,
						_ => true,
					};
					matches_text && matches_status
				})
				.collect();

			// Sort the view (display ordering only; the underlying list is untouched).
			view.sort_by(|&a, &b| {
				let (ra, rb) = (&rows[a], &rows[b]);
				let ord = match sort.0 {
					1 => ra.outbound_capacity_msat.cmp(&rb.outbound_capacity_msat),
					2 => ra.inbound_capacity_msat.cmp(&rb.inbound_capacity_msat),
					_ => ra.channel_value_sats.cmp(&rb.channel_value_sats),
				};
				if sort.1 {
					ord.reverse()
				} else {
					ord
				}
			});

			ui.label(format!("{} of {} channel(s)", view.len(), rows.len()));
			ui.add_space(5.0);

			ScrollArea::both().max_height(400.0).show(ui, |ui| {
				egui::Grid::new("channels_grid").striped(true).spacing([12.0, 6.0]).show(
					ui,
					|ui| {
						// Header (values now carry their own unit).
						ui.strong("Channel ID");
						ui.strong("User Channel ID");
						ui.strong("Counterparty");
						ui.strong("Funding Tx");
						if ui.button(sort_header("Capacity", &sort, 0)).clicked() {
							sort = (0, if sort.0 == 0 { !sort.1 } else { true });
						}
						if ui.button(sort_header("Outbound", &sort, 1)).clicked() {
							sort = (1, if sort.0 == 1 { !sort.1 } else { true });
						}
						if ui.button(sort_header("Inbound", &sort, 2)).clicked() {
							sort = (2, if sort.0 == 2 { !sort.1 } else { true });
						}
						ui.strong("Liquidity");
						ui.strong("Ready");
						ui.strong("Use");
						ui.strong("Actions");
						ui.end_row();

						for &i in &view {
							let ch = &rows[i];
							// Channel ID
							widgets::id_with_copy(
								ui,
								&ch.channel_id,
								&mut app.state.status_message,
							);

							// User Channel ID
							widgets::id_with_copy(
								ui,
								&ch.user_channel_id,
								&mut app.state.status_message,
							);

							// Counterparty
							widgets::id_with_copy(
								ui,
								&ch.counterparty_node_id,
								&mut app.state.status_message,
							);

							// Funding Txid
							if let Some(txid) = &ch.funding_txid {
								widgets::id_with_copy(ui, txid, &mut app.state.status_message);
							} else {
								ui.label("-");
							}

							// Capacity (unit-aware)
							ui.label(app.fmt_sats(ch.channel_value_sats));

							// Outbound capacity (unit-aware)
							ui.label(app.fmt_msat(ch.outbound_capacity_msat));

							// Inbound capacity (unit-aware)
							ui.label(app.fmt_msat(ch.inbound_capacity_msat));

							// Liquidity split bar (outbound vs inbound)
							let total = ch.outbound_capacity_msat + ch.inbound_capacity_msat;
							let frac = if total == 0 {
								0.0
							} else {
								ch.outbound_capacity_msat as f32 / total as f32
							};
							let hover = format!(
								"out {} / in {}",
								app.fmt_msat(ch.outbound_capacity_msat),
								app.fmt_msat(ch.inbound_capacity_msat)
							);
							liquidity_bar(ui, frac).on_hover_text(hover);

							// Ready
							if ch.is_channel_ready {
								widgets::status_pill(ui, "Ready", Color32::GREEN);
							} else {
								widgets::status_pill(ui, "No", Color32::GRAY);
							}

							// Usable
							if ch.is_usable {
								widgets::status_pill(ui, "Yes", Color32::GREEN);
							} else {
								widgets::status_pill(ui, "No", Color32::GRAY);
							}

							// Actions (collapsed into a single menu)
							ui.menu_button("⋮", |ui| {
								if ui.button("Close").clicked() {
									app.state.forms.close_channel.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.close_channel.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_close_channel_dialog = true;
									ui.close_menu();
								}
								if ui.button("Splice+").clicked() {
									app.state.forms.splice_in.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.splice_in.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_splice_in_dialog = true;
									ui.close_menu();
								}
								if ui.button("Splice-").clicked() {
									app.state.forms.splice_out.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.splice_out.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_splice_out_dialog = true;
									ui.close_menu();
								}
								if ui.button("Config").clicked() {
									app.state.forms.update_channel_config.user_channel_id =
										ch.user_channel_id.clone();
									app.state.forms.update_channel_config.counterparty_node_id =
										ch.counterparty_node_id.clone();
									app.state.show_update_config_dialog = true;
									ui.close_menu();
								}
							});

							ui.end_row();
						}
					},
				);
			});

			ui.memory_mut(|m| {
				m.data.insert_temp(filter_id, filter);
				m.data.insert_temp(status_id, status_filter);
				m.data.insert_temp(sort_id, sort);
			});
		}
	} else {
		ui.label("No channel data available. Click Refresh to fetch.");
	}
}

// Two-color liquidity bar: amber outbound (left) over a blue inbound track.
fn liquidity_bar(ui: &mut Ui, frac: f32) -> egui::Response {
	let frac = frac.clamp(0.0, 1.0);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(80.0, 14.0), egui::Sense::hover());
	if ui.is_rect_visible(rect) {
		let painter = ui.painter();
		let full = egui::Rounding::same(3.0);
		painter.rect_filled(rect, full, INBOUND_COLOR);
		let out_w = rect.width() * frac;
		if out_w > 0.0 {
			let out_rect = egui::Rect::from_min_size(rect.min, egui::vec2(out_w, rect.height()));
			let rounding = if frac >= 0.999 {
				full
			} else {
				egui::Rounding { nw: 3.0, sw: 3.0, ne: 0.0, se: 0.0 }
			};
			painter.rect_filled(out_rect, rounding, OUTBOUND_COLOR);
		}
	}
	response
}

fn channel_status_label(s: i32) -> &'static str {
	match s {
		0 => "Ready",
		1 => "Usable",
		2 => "Pending",
		_ => "All",
	}
}

// Header label with a sort-direction arrow when this column is the active sort key.
fn sort_header(label: &str, sort: &(u8, bool), col: u8) -> String {
	if sort.0 == col {
		format!("{} {}", label, if sort.1 { "⬇" } else { "⬆" })
	} else {
		label.to_string()
	}
}

// Per-row snapshot extracted from state.channels so the state borrow is released
// before formatting/status-bar writes in the grid body.
struct ChannelRow {
	channel_id: String,
	counterparty_node_id: String,
	user_channel_id: String,
	funding_txid: Option<String>,
	channel_value_sats: u64,
	outbound_capacity_msat: u64,
	inbound_capacity_msat: u64,
	is_channel_ready: bool,
	is_usable: bool,
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
		let unit_label = crate::ui::unit_label(app.state.display_unit);
		let form = &mut app.state.forms.open_channel;

		egui::Grid::new("open_channel_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
			ui.label("Node Pubkey:");
			ui.text_edit_singleline(&mut form.node_pubkey);
			ui.end_row();

			ui.label("Address:");
			ui.text_edit_singleline(&mut form.address);
			ui.end_row();

			ui.label(format!("Channel Amount ({}):", unit_label));
			ui.text_edit_singleline(&mut form.channel_amount_sats);
			ui.end_row();

			ui.label(format!("Push Amount ({}):", unit_label));
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

		let is_close_pending = app.state.tasks.close_channel.is_some();
		let is_force_close_pending = app.state.tasks.force_close_channel.is_some();

		// Cooperative close: a normal, non-gated button.
		ui.horizontal(|ui| {
			if is_close_pending || is_force_close_pending {
				ui.spinner();
			} else if ui.button("Close (Cooperative)").clicked() {
				app.close_channel();
			}
			if ui.button("Cancel").clicked() {
				app.state.show_close_channel_dialog = false;
				app.state.forms.close_channel = Default::default();
			}
		});

		ui.separator();

		// Destructive force close: gated behind an irreversibility checkbox.
		ui.label(egui::RichText::new("Force Close").strong());
		let id = ui.id().with("force_close_confirm");
		let mut confirmed = ui.memory_mut(|m| m.data.get_temp::<bool>(id).unwrap_or(false));
		ui.checkbox(&mut confirmed, "I understand this is irreversible");
		ui.memory_mut(|m| m.data.insert_temp(id, confirmed));

		if is_force_close_pending {
			ui.spinner();
		} else {
			let btn = egui::Button::new(
				egui::RichText::new("Force Close").color(egui::Color32::WHITE),
			)
			.fill(egui::Color32::DARK_RED);
			if ui.add_enabled(confirmed, btn).clicked() {
				app.force_close_channel();
			}
		}
	});
}

fn render_splice_in_dialog(ctx: &Context, app: &mut LspServerApp) {
	if !app.state.show_splice_in_dialog {
		return;
	}

	egui::Window::new("Splice In").collapsible(false).resizable(false).show(ctx, |ui| {
		let unit_label = crate::ui::unit_label(app.state.display_unit);
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

			ui.label(format!("Amount ({}):", unit_label));
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
		let unit_label = crate::ui::unit_label(app.state.display_unit);
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

			ui.label(format!("Amount ({}):", unit_label));
			ui.text_edit_singleline(&mut form.splice_amount_sats);
			ui.end_row();

			ui.label("Address (optional):");
			ui.text_edit_singleline(&mut form.address);
			ui.end_row();
		});

		ui.add_space(10.0);

		// Destructive splice out: gated behind an irreversibility checkbox.
		let id = ui.id().with("splice_out_confirm");
		let mut confirmed = ui.memory_mut(|m| m.data.get_temp::<bool>(id).unwrap_or(false));
		ui.checkbox(&mut confirmed, "I understand this is irreversible");
		ui.memory_mut(|m| m.data.insert_temp(id, confirmed));

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.splice_out.is_some();
			if is_pending {
				ui.spinner();
			} else {
				let btn = egui::Button::new(
					egui::RichText::new("Splice Out").color(egui::Color32::WHITE),
				)
				.fill(egui::Color32::DARK_RED);
				if ui.add_enabled(confirmed, btn).clicked() {
					app.splice_out();
				}
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
