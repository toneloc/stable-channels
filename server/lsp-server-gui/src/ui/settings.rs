use dioxus::prelude::*;

use crate::actions;
use crate::config::{self, GuiConfig};
#[cfg(not(target_arch = "wasm32"))]
use crate::config::ChainSourceType;
#[cfg(target_arch = "wasm32")]
use crate::state::Dialog;
use crate::state::{AppCtx, ConnectionStatus, DisplayUnit, Theme};
use crate::ui::widgets::{Card, Check, Field, Icon, Modal, SegBtn, TextArea, TextInput};
use crate::ui::close_dialog;
#[cfg(target_arch = "wasm32")]
use crate::ui::open_dialog;

const HINT_API_KEY: &str =
	"Auto-generated at <storage_dir>/<network>/api_key. Get hex: xxd -p <path>/api_key | tr -d '\\n'";

#[component]
pub fn Settings() -> Element {
	let ctx = use_context::<AppCtx>();
	let prefs = *ctx.prefs.read();
	let price = ctx.price.read().as_ref().map(|p| p.price).filter(|p| *p > 0.0);
	let mut prefs_signal = ctx.prefs;
	let mut set_unit = move |unit: DisplayUnit| prefs_signal.write().display_unit = unit;
	let mut set_theme = move |theme: Theme| prefs_signal.write().theme = theme;
	rsx! {
		div { class: "grid-2",
			div { class: "stack", style: "gap: 18px;",
				ConnectionCard {}
				ChainSourceCard {}
			}
			div { class: "stack", style: "gap: 18px;",
				Card { title: "Display unit", sub: "How amounts are shown and entered across the app",
					div { class: "stack",
						div { class: "seg", style: "align-self: flex-start;",
							SegBtn { active: prefs.display_unit == DisplayUnit::Usd, onclick: move |_| set_unit(DisplayUnit::Usd), "USD" }
							SegBtn { active: prefs.display_unit == DisplayUnit::Btc, onclick: move |_| set_unit(DisplayUnit::Btc), "BTC" }
							SegBtn { active: prefs.display_unit == DisplayUnit::Sats, onclick: move |_| set_unit(DisplayUnit::Sats), "Sats" }
						}
						div { class: "row", style: "gap: 8px;",
							span { class: "coin sm", "₿" }
							match price {
								Some(p) => rsx! { span { class: "muted", "Live rate: " span { class: "num", style: "color: var(--text); font-weight: 600;", "{crate::format::format_usd(p)}" } " / BTC" } },
								None => rsx! { span { class: "muted", "Live rate: unavailable" } },
							}
						}
					}
				}
				Card { title: "Data refresh", sub: "Keep the visible page current without pressing Refresh",
					div { class: "toggle-row",
						Check {
							checked: prefs.auto_refresh,
							label: "Auto-refresh every 30 seconds",
							help: "Refreshes only what the current page shows. Payments stop refreshing once more than one page is loaded, so paging is never undone. Followed logs refresh every 10 seconds.",
							onchange: move |v| prefs_signal.write().auto_refresh = v,
						}
					}
				}
				Card { title: "Appearance", sub: "Follow the system or pick a theme",
					div { class: "chips",
						button { class: if prefs.theme == Theme::System { "chip on" } else { "chip" }, onclick: move |_| set_theme(Theme::System), Icon { name: "monitor", size: 16 } "System" }
						button { class: if prefs.theme == Theme::Light { "chip on" } else { "chip" }, onclick: move |_| set_theme(Theme::Light), Icon { name: "sun", size: 16 } "Light" }
						button { class: if prefs.theme == Theme::Dark { "chip on" } else { "chip" }, onclick: move |_| set_theme(Theme::Dark), Icon { name: "moon", size: 16 } "Dark" }
					}
				}
			}
		}
	}
}

/// Fill connection settings from a parsed daemon config (native also takes cert, path and chain source).
fn apply_config(ctx: AppCtx, gui_config: GuiConfig, path: Option<String>) {
	let mut conn = ctx.conn;
	let mut c = conn.write();
	c.server_url = gui_config.server_url;
	c.api_key = gui_config.api_key;
	c.network = gui_config.network;
	if let Some(path) = path {
		c.tls_cert_path = gui_config.tls_cert_path;
		c.config_file_path = Some(path);
		let mut forms = ctx.forms;
		forms.write().chain_source = crate::state::ChainSourceForm::from_config(&gui_config.chain_source);
		c.chain_source = gui_config.chain_source;
	}
}

#[component]
fn ConnectionCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut conn = ctx.conn;
	let c = conn.read();
	let server_url = c.server_url.clone();
	let api_key = c.api_key.clone();
	// TLS cert path is only needed on native (browser handles TLS)
	#[cfg(not(target_arch = "wasm32"))]
	let cert = cert_field(ctx, c.tls_cert_path.clone());
	#[cfg(target_arch = "wasm32")]
	let cert = rsx! {};
	let config_path = c.config_file_path.clone();
	let status = c.status.clone();
	drop(c);
	let connected = status == ConnectionStatus::Connected;
	let (tone, label) = match &status {
		ConnectionStatus::Disconnected => ("", "Disconnected".to_string()),
		ConnectionStatus::Connected => ("success", "Connected".to_string()),
		ConnectionStatus::Error(e) => ("danger", e.clone()),
	};
	let file_busy = ctx.busy(crate::state::Op::FileDialog);

	rsx! {
		Card { title: "Connection", sub: "Stable Channels LSP daemon (REST over TLS, HMAC-authenticated)",
			div { class: "stack", style: "gap: 16px;",
				div { class: "toggle-row",
					div { class: "row nowrap", style: "gap: 10px; min-width: 0;",
						span { class: "dot {tone}" }
						span { class: "strong break", "{label}" }
					}
					if let Some(path) = config_path {
						span { class: "small muted break", "Loaded from: {path}" }
					}
				}
				Field { label: "Server URL",
					TextInput { value: server_url, mono: true, placeholder: "localhost:3002", oninput: move |v| conn.write().server_url = v }
				}
				Field { label: "API Key", hint: HINT_API_KEY,
					TextInput { value: api_key, mono: true, placeholder: "64 hex characters", oninput: move |v| conn.write().api_key = v }
				}
				{cert}
				div { class: "row",
					if connected {
						button { class: "btn danger", onclick: move |_| actions::disconnect(ctx), Icon { name: "power", size: 16 } "Disconnect" }
					} else {
						button { class: "btn primary", onclick: move |_| actions::connect(ctx), Icon { name: "plug", size: 16 } "Connect" }
					}
					button { class: "btn", disabled: file_busy, onclick: move |_| load_config(ctx), Icon { name: "upload", size: 16 } "Load Config" }
				}
			}
		}
	}
}

#[cfg(not(target_arch = "wasm32"))]
fn cert_field(ctx: AppCtx, value: String) -> Element {
	let mut conn = ctx.conn;
	let browse = move |_| {
		with_file_dialog(ctx, async move {
			// Default cert is tls.crt; include common cert extensions so the
			// real file isn't greyed out on macOS (NSOpenPanel disables non-matching files).
			if let Some(path) = crate::platform::pick_file(&[("Certificate", &["crt", "cert", "pem", "der"])]).await {
				let mut conn = ctx.conn;
				conn.write().tls_cert_path = path.display().to_string();
			}
		});
	};
	rsx! {
		Field { label: "TLS Cert Path", hint: "The daemon's self-signed certificate, <storage_dir>/tls.crt",
			div { class: "row nowrap",
				div { class: "grow", TextInput { value, mono: true, placeholder: "/path/to/tls.crt", oninput: move |v| conn.write().tls_cert_path = v } }
				button { class: "btn", disabled: ctx.busy(crate::state::Op::FileDialog), onclick: browse, "Browse..." }
			}
		}
	}
}

/// Run a native file dialog without blocking the window; one dialog at a time.
#[cfg(not(target_arch = "wasm32"))]
fn with_file_dialog(ctx: AppCtx, fut: impl std::future::Future<Output = ()> + 'static) {
	use crate::state::Op;
	let mut pending = ctx.pending;
	if pending.peek().has(Op::FileDialog) {
		return;
	}
	pending.write().insert(Op::FileDialog);
	dioxus::core::spawn_forever(async move {
		fut.await;
		let mut pending = ctx.pending;
		pending.write().remove(Op::FileDialog);
	});
}

#[cfg(not(target_arch = "wasm32"))]
fn load_config(ctx: AppCtx) {
	with_file_dialog(ctx, async move {
		let Some(path) = crate::platform::pick_file(&[("TOML files", &["toml"]), ("All files", &["*"])]).await else {
			return;
		};
		match config::load_config(&path) {
			Ok(gui_config) => {
				apply_config(ctx, gui_config, Some(path.display().to_string()));
				ctx.success(format!("Config loaded from {}", path.display()));
			},
			Err(e) => ctx.error(format!("Failed to load config: {}", e)),
		}
	});
}

#[cfg(target_arch = "wasm32")]
fn load_config(ctx: AppCtx) {
	open_dialog(ctx, Dialog::LoadConfig);
}

/// Paste-a-config dialog (web builds cannot read the daemon's files).
#[component]
pub fn LoadConfigDialog() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut view = ctx.view;
	let text = view.read().config_paste_text.clone();
	let cancel = move |_| {
		view.write().config_paste_text.clear();
		close_dialog(ctx);
	};
	let load = move |_| {
		let text = view.peek().config_paste_text.clone();
		match config::parse_config_from_str(&text) {
			Ok(gui_config) => {
				apply_config(ctx, gui_config, None);
				ctx.success("Config loaded successfully");
				view.write().config_paste_text.clear();
				close_dialog(ctx);
			},
			Err(e) => ctx.error(format!("Failed to parse config: {}", e)),
		}
	};
	rsx! {
		Modal {
			title: "Load Config",
			sub: "Paste your sc-config.toml content below",
			wide: true,
			onclose: move |_| {
				view.write().config_paste_text.clear();
				close_dialog(ctx);
			},
			footer: rsx! {
				button { class: "btn ghost", onclick: cancel, "Cancel" }
				button { class: "btn primary", onclick: load, "Load" }
			},
			TextArea { value: text, rows: 15, placeholder: "[node]\nnetwork = \"signet\"\nrest_service_address = \"...\"", oninput: move |v| view.write().config_paste_text = v }
		}
	}
}

#[cfg(not(target_arch = "wasm32"))]
#[component]
fn ChainSourceCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().chain_source.clone();
	let config_path = ctx.conn.read().config_file_path.clone();
	let file_busy = ctx.busy(crate::state::Op::FileDialog);

	let save = move |_| {
		let path = ctx.conn.peek().config_file_path.clone();
		if let Some(path) = path {
			let chain_source = ctx.forms.peek().chain_source.to_config();
			match config::save_chain_source(&path, &chain_source) {
				Ok(()) => {
					let mut conn = ctx.conn;
					conn.write().chain_source = chain_source;
					ctx.success(format!("Config saved to {}", path));
				},
				Err(e) => ctx.error(format!("Failed to save: {}", e)),
			}
		} else {
			ctx.error("No config file loaded. Use 'Save As...'");
		}
	};
	let save_as = move |_| {
		with_file_dialog(ctx, async move {
			// Start in the loaded config's directory, else the daemon crate dir, else the cwd.
			let existing = ctx.conn.peek().config_file_path.clone();
			let directory = match existing {
				Some(existing) => std::path::Path::new(&existing).parent().map(|p| p.to_path_buf()),
				None => std::env::current_dir().ok().map(|cwd| {
					let sc_daemon_dir = cwd.join("server/stable-channels-lsp");
					if sc_daemon_dir.exists() {
						sc_daemon_dir
					} else {
						cwd
					}
				}),
			};
			let Some(path) =
				crate::platform::save_file(&[("TOML files", &["toml"])], "sc-config.toml", directory).await
			else {
				return;
			};
			let chain_source = ctx.forms.peek().chain_source.to_config();
			match config::save_chain_source(&path, &chain_source) {
				Ok(()) => {
					let mut conn = ctx.conn;
					let mut c = conn.write();
					c.config_file_path = Some(path.display().to_string());
					c.chain_source = chain_source;
					drop(c);
					ctx.success(format!("Config saved to {}", path.display()));
				},
				Err(e) => ctx.error(format!("Failed to save: {}", e)),
			}
		});
	};
	let type_value = match form.source_type {
		ChainSourceType::None => "none",
		ChainSourceType::Bitcoind => "bitcoind",
		ChainSourceType::Electrum => "electrum",
		ChainSourceType::Esplora => "esplora",
	};

	rsx! {
		Card { title: "Chain Source", sub: "Stored in the daemon config file",
			details { class: "disclosure",
				summary { Icon { name: "chevron-right", size: 14 } "Chain Source Settings" }
				div { class: "body stack", style: "gap: 14px;",
					Field { label: "Type",
						select {
							class: "select",
							value: type_value,
							onchange: move |e| {
								forms.write().chain_source.source_type = match e.value().as_str() {
									"bitcoind" => ChainSourceType::Bitcoind,
									"electrum" => ChainSourceType::Electrum,
									"esplora" => ChainSourceType::Esplora,
									_ => ChainSourceType::None,
								};
							},
							if form.source_type == ChainSourceType::None {
								option { value: "none", selected: true, disabled: true, "None" }
							}
							for source_type in ChainSourceType::ALL {
								option {
									value: match source_type {
										ChainSourceType::Bitcoind => "bitcoind",
										ChainSourceType::Electrum => "electrum",
										ChainSourceType::Esplora => "esplora",
										ChainSourceType::None => "none",
									},
									selected: source_type == form.source_type,
									"{source_type.label()}"
								}
							}
						}
					}
					match form.source_type {
						ChainSourceType::None => rsx! { span { class: "muted", "No chain source selected" } },
						ChainSourceType::Bitcoind => rsx! {
							div { class: "form-grid",
								Field { label: "RPC Address", TextInput { value: form.btc_rpc_address.clone(), mono: true, oninput: move |v| forms.write().chain_source.btc_rpc_address = v } }
								Field { label: "RPC User", TextInput { value: form.btc_rpc_user.clone(), mono: true, oninput: move |v| forms.write().chain_source.btc_rpc_user = v } }
								Field { label: "RPC Password", TextInput { value: form.btc_rpc_password.clone(), password: true, oninput: move |v| forms.write().chain_source.btc_rpc_password = v } }
							}
						},
						ChainSourceType::Electrum => rsx! {
							Field { label: "Server URL", hint: "e.g., ssl://electrum.blockstream.info:50002",
								TextInput { value: form.server_url.clone(), mono: true, oninput: move |v| forms.write().chain_source.server_url = v }
							}
						},
						ChainSourceType::Esplora => rsx! {
							Field { label: "Server URL", hint: "e.g., https://mempool.space/api",
								TextInput { value: form.server_url.clone(), mono: true, oninput: move |v| forms.write().chain_source.server_url = v }
							}
						},
					}
					div { class: "row",
						button { class: "btn primary", onclick: save, "Save" }
						button { class: "btn", disabled: file_busy, onclick: save_as, "Save As..." }
						if let Some(path) = config_path {
							span { class: "small faint break", "({path})" }
						}
					}
					div { class: "notice", Icon { name: "info", size: 16 } span { em { "Note: Chain source changes require server restart" } } }
				}
			}
		}
	}
}

#[cfg(target_arch = "wasm32")]
#[component]
fn ChainSourceCard() -> Element {
	rsx! {}
}
