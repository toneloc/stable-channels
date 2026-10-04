mod actions;
mod config;
mod dashboard;
mod format;
mod health;
mod history;
mod ledger;
mod platform;
mod state;
mod ui;

// Native entry point
#[cfg(not(target_arch = "wasm32"))]
fn main() {
	use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

	let _ = rustls::crypto::ring::default_provider().install_default();

	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.init();

	let window = WindowBuilder::new()
		.with_title("LSP Server GUI")
		.with_inner_size(LogicalSize::new(1200.0, 800.0))
		.with_min_inner_size(LogicalSize::new(1000.0, 640.0));
	let config = Config::new().with_window(window).with_background_color((0, 0, 0, 255));
	// macOS needs the default Edit menu for copy/paste shortcuts; elsewhere match the old menu-less window.
	#[cfg(not(target_os = "macos"))]
	let config = config.with_menu(None);

	dioxus::LaunchBuilder::desktop().with_cfg(config).launch(ui::App);
}

// WASM entry point
#[cfg(target_arch = "wasm32")]
fn main() {
	platform::install_panic_hook();
	dioxus::LaunchBuilder::web().launch(ui::App);
}
