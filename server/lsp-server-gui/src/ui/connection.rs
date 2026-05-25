use egui::Ui;

use crate::app::LspServerApp;
use crate::config;
#[cfg(not(target_arch = "wasm32"))]
use crate::config::ChainSourceType;
#[cfg(not(target_arch = "wasm32"))]
use crate::state::ChainSourceForm;
use crate::state::{AppState, ConnectionStatus, StatusMessage};

pub fn render_status(ui: &mut Ui, state: &AppState) {
	match &state.connection_status {
		ConnectionStatus::Disconnected => {
			ui.colored_label(egui::Color32::GRAY, "Disconnected");
		},
		ConnectionStatus::Connected => {
			ui.colored_label(egui::Color32::GREEN, "Connected");
		},
		ConnectionStatus::Error(e) => {
			ui.colored_label(egui::Color32::RED, format!("Error: {}", e));
		},
	}
}

pub fn render_settings(ui: &mut Ui, app: &mut LspServerApp) {
	ui.group(|ui| {
        ui.heading("Connection Settings");
        ui.add_space(5.0);

        egui::Grid::new("connection_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
            ui.label("Server URL:");
            ui.text_edit_singleline(&mut app.state.server_url);
            ui.end_row();

            ui.label("API Key:");
            ui.vertical(|ui| {
                ui.text_edit_singleline(&mut app.state.api_key);
                ui.label(
                    egui::RichText::new("Auto-generated at <storage_dir>/<network>/api_key. Get hex: xxd -p <path>/api_key | tr -d '\\n'")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            });
            ui.end_row();

            // TLS cert path is only needed on native (browser handles TLS)
            #[cfg(not(target_arch = "wasm32"))]
            {
                ui.label("TLS Cert Path:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut app.state.tls_cert_path);
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PEM files", &["pem"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                        {
                            app.state.tls_cert_path = path.display().to_string();
                        }
                    }
                });
                ui.end_row();
            }
        });

        ui.add_space(10.0);

        ui.horizontal(|ui| {
            let is_connected = matches!(app.state.connection_status, ConnectionStatus::Connected);
            if is_connected {
                if ui.button("Disconnect").clicked() {
                    app.disconnect();
                }
            } else if ui.button("Connect").clicked() {
                app.connect();
            }

            ui.separator();

            // Native: file dialog
            #[cfg(not(target_arch = "wasm32"))]
            if ui.button("Load Config").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("TOML files", &["toml"])
                    .add_filter("All files", &["*"])
                    .pick_file()
                {
                    match config::load_config(&path) {
                        Ok(gui_config) => {
                            app.state.server_url = gui_config.server_url;
                            app.state.api_key = gui_config.api_key;
                            app.state.tls_cert_path = gui_config.tls_cert_path;
                            app.state.network = gui_config.network;
                            app.state.config_file_path = Some(path.display().to_string());
                            app.state.forms.chain_source =
                                ChainSourceForm::from_config(&gui_config.chain_source);
                            app.state.chain_source = gui_config.chain_source;
                            app.state.status_message = Some(StatusMessage::success(format!(
                                "Config loaded from {}",
                                path.display()
                            )));
                        }
                        Err(e) => {
                            app.state.status_message =
                                Some(StatusMessage::error(format!("Failed to load config: {}", e)));
                        }
                    }
                }
            }

            // WASM: show paste dialog
            #[cfg(target_arch = "wasm32")]
            if ui.button("Load Config").clicked() {
                app.state.show_load_config_dialog = true;
            }
        });
    });

	// Chain Source Settings (collapsible) - only on native
	#[cfg(not(target_arch = "wasm32"))]
	{
		ui.add_space(10.0);

		egui::CollapsingHeader::new("Chain Source Settings").default_open(false).show(ui, |ui| {
			render_chain_source_editor(ui, &mut app.state.forms.chain_source);

			ui.add_space(10.0);

			ui.horizontal(|ui| {
				// Save Config button - saves to current file or prompts if none loaded
				let has_config = app.state.config_file_path.is_some();
				if ui.button("Save").clicked() {
					if let Some(path) = &app.state.config_file_path {
						let chain_source = app.state.forms.chain_source.to_config();
						match config::save_chain_source(path, &chain_source) {
							Ok(()) => {
								app.state.chain_source = chain_source;
								app.state.status_message = Some(StatusMessage::success(format!(
									"Config saved to {}",
									path
								)));
							},
							Err(e) => {
								app.state.status_message =
									Some(StatusMessage::error(format!("Failed to save: {}", e)));
							},
						}
					} else {
						app.state.status_message =
							Some(StatusMessage::error("No config file loaded. Use 'Save As...'"));
					}
				}

				// Save As button - always shows file dialog
				if ui.button("Save As...").clicked() {
					let mut dialog = rfd::FileDialog::new()
						.add_filter("TOML files", &["toml"])
						.set_file_name("sc-config.toml");

					// Set starting directory based on existing config or sensible default
					if let Some(existing_path) = &app.state.config_file_path {
						if let Some(parent) = std::path::Path::new(existing_path).parent() {
							dialog = dialog.set_directory(parent);
						}
					} else if let Ok(cwd) = std::env::current_dir() {
						let sc_daemon_dir = cwd.join("server/stable-channels-lsp");
						if sc_daemon_dir.exists() {
							dialog = dialog.set_directory(&sc_daemon_dir);
						} else {
							dialog = dialog.set_directory(&cwd);
						}
					}

					if let Some(path) = dialog.save_file() {
						let chain_source = app.state.forms.chain_source.to_config();
						match config::save_chain_source(&path, &chain_source) {
							Ok(()) => {
								app.state.config_file_path = Some(path.display().to_string());
								app.state.chain_source = chain_source;
								app.state.status_message = Some(StatusMessage::success(format!(
									"Config saved to {}",
									path.display()
								)));
							},
							Err(e) => {
								app.state.status_message =
									Some(StatusMessage::error(format!("Failed to save: {}", e)));
							},
						}
					}
				}

				// Show current config file path
				if has_config {
					if let Some(path) = &app.state.config_file_path {
						ui.label(
							egui::RichText::new(format!("({})", path))
								.small()
								.color(egui::Color32::GRAY),
						);
					}
				}
			});

			ui.add_space(5.0);
			ui.label(
				egui::RichText::new("Note: Chain source changes require server restart")
					.small()
					.italics()
					.color(egui::Color32::GRAY),
			);
		});
	}
}

#[cfg(not(target_arch = "wasm32"))]
fn render_chain_source_editor(ui: &mut Ui, form: &mut ChainSourceForm) {
	ui.horizontal(|ui| {
		ui.label("Type:");
		egui::ComboBox::from_id_salt("chain_source_type")
			.selected_text(form.source_type.label())
			.show_ui(ui, |ui| {
				for source_type in ChainSourceType::ALL {
					ui.selectable_value(&mut form.source_type, source_type, source_type.label());
				}
			});
	});

	ui.add_space(5.0);

	match form.source_type {
		ChainSourceType::None => {
			ui.label("No chain source selected");
		},
		ChainSourceType::Bitcoind => {
			egui::Grid::new("bitcoind_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
				ui.label("RPC Address:");
				ui.text_edit_singleline(&mut form.btc_rpc_address);
				ui.end_row();

				ui.label("RPC User:");
				ui.text_edit_singleline(&mut form.btc_rpc_user);
				ui.end_row();

				ui.label("RPC Password:");
				ui.add(egui::TextEdit::singleline(&mut form.btc_rpc_password).password(true));
				ui.end_row();
			});
		},
		ChainSourceType::Electrum => {
			egui::Grid::new("electrum_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
				ui.label("Server URL:");
				ui.text_edit_singleline(&mut form.server_url);
				ui.end_row();
			});
			ui.label(
				egui::RichText::new("e.g., ssl://electrum.blockstream.info:50002")
					.small()
					.color(egui::Color32::GRAY),
			);
		},
		ChainSourceType::Esplora => {
			egui::Grid::new("esplora_grid").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
				ui.label("Server URL:");
				ui.text_edit_singleline(&mut form.server_url);
				ui.end_row();
			});
			ui.label(
				egui::RichText::new("e.g., https://mempool.space/api")
					.small()
					.color(egui::Color32::GRAY),
			);
		},
	}
}

/// Render the Load Config dialog (for WASM - paste config content)
pub fn render_load_config_dialog(ctx: &egui::Context, app: &mut LspServerApp) {
	if !app.state.show_load_config_dialog {
		return;
	}

	egui::Window::new("Load Config").collapsible(false).resizable(true).default_width(500.0).show(
		ctx,
		|ui| {
			ui.label("Paste your sc-config.toml content below:");
			ui.add_space(5.0);

			egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
				ui.add(
					egui::TextEdit::multiline(&mut app.state.config_paste_text)
						.desired_width(f32::INFINITY)
						.desired_rows(15)
						.font(egui::TextStyle::Monospace),
				);
			});

			ui.add_space(10.0);

			ui.horizontal(|ui| {
				if ui.button("Load").clicked() {
					match config::parse_config_from_str(&app.state.config_paste_text) {
						Ok(gui_config) => {
							app.state.server_url = gui_config.server_url;
							app.state.api_key = gui_config.api_key;
							app.state.network = gui_config.network;
							app.state.status_message =
								Some(StatusMessage::success("Config loaded successfully"));
							app.state.show_load_config_dialog = false;
							app.state.config_paste_text.clear();
						},
						Err(e) => {
							app.state.status_message = Some(StatusMessage::error(format!(
								"Failed to parse config: {}",
								e
							)));
						},
					}
				}

				if ui.button("Cancel").clicked() {
					app.state.show_load_config_dialog = false;
					app.state.config_paste_text.clear();
				}
			});
		},
	);
}
