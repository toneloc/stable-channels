//! Cross-platform async task handling for native and WASM targets.

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
use std::future::Future;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

/// A task handle that can be polled for completion (native implementation using channels)
#[cfg(not(target_arch = "wasm32"))]
pub struct ChannelTaskHandle<T> {
	rx: std::sync::mpsc::Receiver<Result<T, String>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> ChannelTaskHandle<T> {
	/// Check if the task has completed and return the result if so.
	pub fn try_take(&mut self) -> Option<Result<T, String>> {
		match self.rx.try_recv() {
			Ok(result) => Some(result),
			Err(_) => None,
		}
	}
}

/// Spawn a task on the provided tokio runtime and return a channel-based handle
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_with_runtime<T, F>(rt: &tokio::runtime::Runtime, future: F) -> ChannelTaskHandle<T>
where
	T: Send + 'static,
	F: Future<Output = Result<T, String>> + Send + 'static,
{
	let (tx, rx) = std::sync::mpsc::channel();

	rt.spawn(async move {
		let res = future.await;
		let _ = tx.send(res);
	});

	ChannelTaskHandle { rx }
}

/// WASM implementation using shared state
#[cfg(target_arch = "wasm32")]
pub struct ChannelTaskHandle<T> {
	result: Rc<RefCell<Option<Result<T, String>>>>,
}

#[cfg(target_arch = "wasm32")]
impl<T> ChannelTaskHandle<T> {
	pub fn try_take(&mut self) -> Option<Result<T, String>> {
		self.result.borrow_mut().take()
	}
}

/// Spawn a task for WASM targets
#[cfg(target_arch = "wasm32")]
pub fn spawn_local<T, F>(future: F) -> ChannelTaskHandle<T>
where
	T: 'static,
	F: Future<Output = Result<T, String>> + 'static,
{
	let result: Rc<RefCell<Option<Result<T, String>>>> = Rc::new(RefCell::new(None));
	let result_clone = result.clone();

	wasm_bindgen_futures::spawn_local(async move {
		let res = future.await;
		*result_clone.borrow_mut() = Some(res);
	});

	ChannelTaskHandle { result }
}
