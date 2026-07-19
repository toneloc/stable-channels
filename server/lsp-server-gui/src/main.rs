mod app;
mod config;
mod state;
mod task;
mod ui;

// Native entry point
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
	use eframe::egui;

	let _ = rustls::crypto::ring::default_provider().install_default();

	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.init();

	let options = eframe::NativeOptions {
		viewport: egui::ViewportBuilder::default()
			.with_inner_size([1200.0, 800.0])
			.with_min_inner_size([1000.0, 640.0]),
		..Default::default()
	};

	eframe::run_native(
		"LSP Server GUI",
		options,
		Box::new(|cc| Ok(Box::new(app::LspServerApp::new(cc)))),
	)
}

// WASM entry point
#[cfg(target_arch = "wasm32")]
fn main() {
	use wasm_bindgen::JsCast;

	// Redirect tracing to console.log
	eframe::WebLogger::init(log::LevelFilter::Debug).ok();

	let web_options = eframe::WebOptions::default();

	wasm_bindgen_futures::spawn_local(async {
		// Get the canvas element
		let document = web_sys::window().expect("No window").document().expect("No document");

		let canvas = document
			.get_element_by_id("lsp_server_gui_canvas")
			.expect("Failed to find canvas element")
			.dyn_into::<web_sys::HtmlCanvasElement>()
			.expect("Element is not a canvas");

		let start_result = eframe::WebRunner::new()
			.start(canvas, web_options, Box::new(|cc| Ok(Box::new(app::LspServerApp::new(cc)))))
			.await;

		// Remove the loading text and spinner
		if let Some(loading_text) = document.get_element_by_id("loading_text") {
			match start_result {
				Ok(_) => {
					loading_text.remove();
				},
				Err(e) => {
					loading_text.set_inner_html(&format!(
                        "<p>The app has crashed. See the developer console for details.</p><p>Error: {:?}</p>",
                        e
                    ));
				},
			}
		}
	});
}
