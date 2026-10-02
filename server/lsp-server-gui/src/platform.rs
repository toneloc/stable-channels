//! Platform services that differ between the desktop and web builds.

use crate::state::Prefs;

/// Sleep without blocking the UI thread.
pub async fn sleep_ms(ms: u64) {
	#[cfg(not(target_arch = "wasm32"))]
	tokio::time::sleep(std::time::Duration::from_millis(ms)).await;

	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::{closure::Closure, JsCast, JsValue};

		let promise = js_sys::Promise::new(&mut |resolve, _reject| {
			let resolve_from_timer = resolve.clone();
			let callback = Closure::once_into_js(move || {
				let _ = resolve_from_timer.call0(&JsValue::UNDEFINED);
			});
			let scheduled = web_sys::window()
				.and_then(|win| {
					win.set_timeout_with_callback_and_timeout_and_arguments_0(
						callback.as_ref().unchecked_ref(),
						ms.min(i32::MAX as u64) as i32,
					)
					.ok()
				})
				.is_some();
			if !scheduled {
				let _ = resolve.call0(&JsValue::UNDEFINED);
			}
		});
		let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
	}
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
	// Kept alive so X11 clipboard ownership survives after the copy call returns.
	static CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> = const { std::cell::RefCell::new(None) };
}

/// Copy text to the system clipboard; returns false if the platform refused.
#[cfg(not(target_arch = "wasm32"))]
pub fn copy_text(text: &str) -> bool {
	CLIPBOARD.with(|slot| {
		let mut slot = slot.borrow_mut();
		if slot.is_none() {
			*slot = arboard::Clipboard::new().ok();
		}
		match slot.as_mut() {
			Some(clipboard) => clipboard.set_text(text.to_string()).is_ok(),
			None => false,
		}
	})
}

/// Copy text via the browser; resolves to false when both clipboard paths fail.
#[cfg(target_arch = "wasm32")]
pub async fn copy_text(text: &str) -> bool {
	// navigator.clipboard needs a secure context; Umbrel serves plain HTTP, so fall back to execCommand.
	let literal = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
	let script = format!(
		r#"
		const text = {literal};
		const fallback = () => {{
			const previousFocus = document.activeElement;
			const area = document.createElement("textarea");
			area.value = text;
			area.setAttribute("readonly", "");
			area.style.position = "fixed";
			area.style.left = "-10000px";
			area.style.top = "0";
			document.body.appendChild(area);
			area.select();
			try {{ return document.execCommand("copy"); }} catch (_) {{ return false; }} finally {{
				area.remove();
				if (previousFocus && typeof previousFocus.focus === "function") previousFocus.focus();
			}}
		}};
		if (navigator.clipboard && window.isSecureContext) {{
			return navigator.clipboard.writeText(text).then(() => true, fallback);
		}}
		return fallback();
		"#
	);
	dioxus::document::eval(&script).await.ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn prefs_path() -> Option<std::path::PathBuf> {
	dirs::config_dir().map(|dir| dir.join("lsp-server-gui").join("prefs.json"))
}

#[cfg(target_arch = "wasm32")]
const PREFS_KEY: &str = "lsp-server-gui.prefs";

/// Load persisted preferences, falling back to defaults.
pub fn load_prefs() -> Prefs {
	#[cfg(not(target_arch = "wasm32"))]
	let raw = prefs_path().and_then(|path| std::fs::read_to_string(path).ok());

	#[cfg(target_arch = "wasm32")]
	let raw = web_sys::window()
		.and_then(|win| win.local_storage().ok().flatten())
		.and_then(|storage| storage.get_item(PREFS_KEY).ok().flatten());

	raw.and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

/// Persist preferences; failures are ignored (preferences are a convenience).
pub fn save_prefs(prefs: &Prefs) {
	let Ok(raw) = serde_json::to_string(prefs) else { return };

	#[cfg(not(target_arch = "wasm32"))]
	if let Some(path) = prefs_path() {
		if let Some(parent) = path.parent() {
			let _ = std::fs::create_dir_all(parent);
		}
		let _ = std::fs::write(path, raw);
	}

	#[cfg(target_arch = "wasm32")]
	if let Some(storage) = web_sys::window().and_then(|win| win.local_storage().ok().flatten()) {
		let _ = storage.set_item(PREFS_KEY, &raw);
	}
}

/// Native open-file dialog.
#[cfg(not(target_arch = "wasm32"))]
pub async fn pick_file(filters: &[(&str, &[&str])]) -> Option<std::path::PathBuf> {
	let mut dialog = rfd::AsyncFileDialog::new();
	for (name, extensions) in filters {
		dialog = dialog.add_filter(*name, extensions);
	}
	dialog.pick_file().await.map(|handle| handle.path().to_path_buf())
}

/// Native save-file dialog.
#[cfg(not(target_arch = "wasm32"))]
pub async fn save_file(
	filters: &[(&str, &[&str])], file_name: &str, directory: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
	let mut dialog = rfd::AsyncFileDialog::new().set_file_name(file_name);
	for (name, extensions) in filters {
		dialog = dialog.add_filter(*name, extensions);
	}
	if let Some(directory) = directory {
		dialog = dialog.set_directory(directory);
	}
	dialog.save_file().await.map(|handle| handle.path().to_path_buf())
}

/// Offer `content` to the browser as a file download.
#[cfg(target_arch = "wasm32")]
pub fn download_text(file_name: &str, mime: &str, content: &str) -> Result<(), String> {
	use wasm_bindgen::JsCast;

	let err = |e: wasm_bindgen::JsValue| format!("{:?}", e);
	let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(content));
	let options = web_sys::BlobPropertyBag::new();
	options.set_type(mime);
	let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options).map_err(err)?;
	let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
	let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
	let anchor = document
		.create_element("a")
		.map_err(err)?
		.dyn_into::<web_sys::HtmlAnchorElement>()
		.map_err(|_| "anchor cast failed".to_string())?;
	anchor.set_href(&url);
	anchor.set_download(file_name);
	anchor.click();
	let _ = web_sys::Url::revoke_object_url(&url);
	Ok(())
}

/// Fetch the same-origin API key the serving container publishes at /setup/key.txt.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_setup_key() -> Option<String> {
	use wasm_bindgen::JsCast;

	let win = web_sys::window()?;
	let resp = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str("/setup/key.txt")).await.ok()?;
	let resp = resp.dyn_into::<web_sys::Response>().ok()?;
	if !resp.ok() {
		return None;
	}
	let text = resp.text().ok()?;
	let text = wasm_bindgen_futures::JsFuture::from(text).await.ok()?;
	let key = text.as_string()?.trim().to_string();
	(key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())).then_some(key)
}

/// Show a crash notice in the page when the app panics after mounting.
#[cfg(target_arch = "wasm32")]
pub fn install_panic_hook() {
	let previous = std::panic::take_hook();
	std::panic::set_hook(Box::new(move |info| {
		previous(info);
		// The std hook writes to a stderr that goes nowhere in the browser, so echo the message to the console.
		web_sys::console::error_1(&info.to_string().into());
		if let Some(body) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.body()) {
			body.set_inner_html(
				"<div id=\"loading_text\"><p>The app has crashed. See the developer console for details.</p></div>",
			);
		}
	}));
}
