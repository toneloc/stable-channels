use eframe::egui;

use crate::app::LspServerApp;
use crate::state::ConnectionStatus;
use crate::ui::truncate_id;

pub fn render(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.heading("Network Graph");
	ui.add_space(10.0);

	if !matches!(app.state.connection_status, ConnectionStatus::Connected) {
		ui.label("Connect to a server to explore the network graph.");
		return;
	}

	egui::ScrollArea::vertical().show(ui, |ui| {
		render_channels_section(ui, app);
		ui.add_space(20.0);
		render_nodes_section(ui, app);
	});
}

fn render_channels_section(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Graph Channels");
		ui.add_space(5.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.graph_list_channels.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Loading...");
			} else if ui.button("List Channels").clicked() {
				app.fetch_graph_channels();
			}
		});

		if let Some(resp) = &app.state.graph_channels {
			ui.add_space(5.0);
			ui.label(format!("{} channels in network graph", resp.short_channel_ids.len()));

			if !resp.short_channel_ids.is_empty() {
				ui.add_space(5.0);
				let max_display = 100.min(resp.short_channel_ids.len());
				egui::Grid::new("graph_channels_list").striped(true).show(ui, |ui| {
					ui.label(egui::RichText::new("Short Channel ID").strong());
					ui.end_row();

					for scid in resp.short_channel_ids.iter().take(max_display) {
						ui.label(format!("{}", scid));
						ui.end_row();
					}
				});

				if resp.short_channel_ids.len() > max_display {
					ui.label(format!(
						"... and {} more",
						resp.short_channel_ids.len() - max_display
					));
				}
			}
		}

		ui.add_space(10.0);
		ui.separator();
		ui.heading("Lookup Channel");
		ui.add_space(5.0);

		let form = &mut app.state.forms.graph_get_channel;
		ui.horizontal(|ui| {
			ui.label("Short Channel ID:");
			ui.text_edit_singleline(&mut form.short_channel_id);
		});

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.graph_get_channel.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Loading...");
			} else if ui.button("Lookup").clicked() {
				app.fetch_graph_channel();
			}
		});

		if let Some(resp) = &app.state.graph_channel_detail {
			if let Some(ch) = &resp.channel {
				ui.add_space(5.0);
				egui::Grid::new("graph_channel_detail").num_columns(2).spacing([10.0, 5.0]).show(
					ui,
					|ui| {
						ui.label("Node One:");
						ui.horizontal(|ui| {
							ui.monospace(truncate_id(&ch.node_one, 8, 4));
							if ui.small_button("Copy").clicked() {
								ui.output_mut(|o| o.copied_text = ch.node_one.clone());
							}
						});
						ui.end_row();

						ui.label("Node Two:");
						ui.horizontal(|ui| {
							ui.monospace(truncate_id(&ch.node_two, 8, 4));
							if ui.small_button("Copy").clicked() {
								ui.output_mut(|o| o.copied_text = ch.node_two.clone());
							}
						});
						ui.end_row();

						if let Some(cap) = ch.capacity_sats {
							ui.label("Capacity:");
							ui.label(format!("{} sats", crate::ui::format_sats(cap)));
							ui.end_row();
						}

						if let Some(update) = &ch.one_to_two {
							ui.label("1->2 Enabled:");
							ui.label(if update.enabled { "Yes" } else { "No" });
							ui.end_row();
							ui.label("1->2 CLTV Delta:");
							ui.label(format!("{}", update.cltv_expiry_delta));
							ui.end_row();
							ui.label("1->2 HTLC Min:");
							ui.label(format!("{} msat", update.htlc_minimum_msat));
							ui.end_row();
							ui.label("1->2 HTLC Max:");
							ui.label(format!("{} msat", update.htlc_maximum_msat));
							ui.end_row();
						}

						if let Some(update) = &ch.two_to_one {
							ui.label("2->1 Enabled:");
							ui.label(if update.enabled { "Yes" } else { "No" });
							ui.end_row();
							ui.label("2->1 CLTV Delta:");
							ui.label(format!("{}", update.cltv_expiry_delta));
							ui.end_row();
							ui.label("2->1 HTLC Min:");
							ui.label(format!("{} msat", update.htlc_minimum_msat));
							ui.end_row();
							ui.label("2->1 HTLC Max:");
							ui.label(format!("{} msat", update.htlc_maximum_msat));
							ui.end_row();
						}
					},
				);
			}
		}
	});
}

fn render_nodes_section(ui: &mut egui::Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
		ui.heading("Graph Nodes");
		ui.add_space(5.0);

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.graph_list_nodes.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Loading...");
			} else if ui.button("List Nodes").clicked() {
				app.fetch_graph_nodes();
			}
		});

		if let Some(resp) = &app.state.graph_nodes {
			ui.add_space(5.0);
			ui.label(format!("{} nodes in network graph", resp.node_ids.len()));

			if !resp.node_ids.is_empty() {
				ui.add_space(5.0);
				let max_display = 100.min(resp.node_ids.len());
				egui::Grid::new("graph_nodes_list").striped(true).show(ui, |ui| {
					ui.label(egui::RichText::new("Node ID").strong());
					ui.end_row();

					for node_id in resp.node_ids.iter().take(max_display) {
						ui.horizontal(|ui| {
							ui.monospace(truncate_id(node_id, 8, 4));
							if ui.small_button("Copy").clicked() {
								ui.output_mut(|o| o.copied_text = node_id.clone());
							}
						});
						ui.end_row();
					}
				});

				if resp.node_ids.len() > max_display {
					ui.label(format!("... and {} more", resp.node_ids.len() - max_display));
				}
			}
		}

		ui.add_space(10.0);
		ui.separator();
		ui.heading("Lookup Node");
		ui.add_space(5.0);

		let form = &mut app.state.forms.graph_get_node;
		ui.horizontal(|ui| {
			ui.label("Node ID:");
			ui.text_edit_singleline(&mut form.node_id);
		});

		ui.horizontal(|ui| {
			let is_pending = app.state.tasks.graph_get_node.is_some();
			if is_pending {
				ui.spinner();
				ui.label("Loading...");
			} else if ui.button("Lookup").clicked() {
				app.fetch_graph_node();
			}
		});

		if let Some(resp) = &app.state.graph_node_detail {
			if let Some(node) = &resp.node {
				ui.add_space(5.0);
				egui::Grid::new("graph_node_detail").num_columns(2).spacing([10.0, 5.0]).show(
					ui,
					|ui| {
						ui.label("Channels:");
						ui.label(format!("{}", node.channels.len()));
						ui.end_row();

						if let Some(ann) = &node.announcement_info {
							ui.label("Alias:");
							ui.label(&ann.alias);
							ui.end_row();

							ui.label("Color:");
							ui.label(format!("#{}", ann.rgb));
							ui.end_row();

							ui.label("Last Update:");
							ui.label(format!("{}", ann.last_update));
							ui.end_row();

							if !ann.addresses.is_empty() {
								ui.label("Addresses:");
								ui.vertical(|ui| {
									for addr in &ann.addresses {
										ui.label(addr);
									}
								});
								ui.end_row();
							}
						}
					},
				);
			}
		}
	});
}
