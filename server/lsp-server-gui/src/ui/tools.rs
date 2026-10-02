use dioxus::prelude::*;

use crate::actions;
use crate::state::{AppCtx, Op};
use crate::ui::widgets::{Bubble, Card, Field, Gate, Icon, Pill, Spinner, TextArea, TextInput};

#[component]
pub fn Tools() -> Element {
	let ctx = use_context::<AppCtx>();
	if !ctx.is_connected() {
		return rsx! { Gate {} };
	}
	rsx! {
		div { class: "tools",
			div { class: "tools-pair",
				SignCard {}
				VerifyCard {}
			}
			ExportCard {}
		}
	}
}

#[component]
fn SignCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let message = forms.read().sign_message.message.clone();
	let pending = ctx.busy(Op::SignMessage);
	let signature = ctx.results.read().sign_result.clone();
	rsx! {
		Card { title: "Sign Message", sub: "Sign with the node's key",
			icon: rsx! { Bubble { icon: "pen", tone: "blue" } },
			div { class: "tool-body",
				Field { label: "Message",
					TextArea { value: message, rows: 3, mono: false, oninput: move |v| forms.write().sign_message.message = v }
				}
				div { class: "tool-actions",
					button { class: "btn primary", disabled: pending, onclick: move |_| actions::sign_message(ctx),
						if pending { Spinner {} "Signing..." } else { "Sign" }
					}
				}
				if let Some(signature) = signature {
					div { class: "result",
						div { class: "row between",
							span { class: "field-label", "Signature" }
							button { class: "btn sm", onclick: {
								let signature = signature.clone();
								move |_| crate::actions::copy(ctx, &signature)
							}, Icon { name: "copy", size: 14 } "Copy Signature" }
						}
						div { class: "value", "{signature}" }
					}
				}
			}
		}
	}
}

#[component]
fn VerifyCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let mut forms = ctx.forms;
	let form = forms.read().verify_signature.clone();
	let pending = ctx.busy(Op::VerifySignature);
	let result = ctx.results.read().verify_result;
	rsx! {
		Card { title: "Verify Signature", sub: "Check a message against a node public key",
			icon: rsx! { Bubble { icon: "shield", tone: "green" } },
			div { class: "tool-body",
				Field { label: "Message",
					TextArea { value: form.message.clone(), rows: 2, mono: false, oninput: move |v| forms.write().verify_signature.message = v }
				}
				Field { label: "Signature (zbase32)",
					TextInput { value: form.signature.clone(), mono: true, oninput: move |v| forms.write().verify_signature.signature = v }
				}
				Field { label: "Public Key (hex)",
					TextInput { value: form.public_key.clone(), mono: true, oninput: move |v| forms.write().verify_signature.public_key = v }
				}
				div { class: "tool-actions",
					button { class: "btn primary", disabled: pending, onclick: move |_| actions::verify_signature(ctx),
						if pending { Spinner {} "Verifying..." } else { "Verify" }
					}
					match result {
						Some(true) => rsx! { Pill { tone: "success", Icon { name: "check", size: 12 } "VALID" } },
						Some(false) => rsx! { Pill { tone: "danger", Icon { name: "x", size: 12 } "INVALID" } },
						None => rsx! {},
					}
				}
			}
		}
	}
}

#[component]
fn ExportCard() -> Element {
	let ctx = use_context::<AppCtx>();
	let pending = ctx.busy(Op::ExportPathfindingScores);
	let size = ctx.results.read().export_scores_result.as_ref().map(|r| r.scores.len());
	rsx! {
		Card { title: "Export Pathfinding Scores", sub: "Export the pathfinding scores used by the router.",
			icon: rsx! { Bubble { icon: "graph", tone: "purple" } },
			actions: rsx! {
				if let Some(n) = size {
					Pill { tone: "success", "Exported {n} bytes" }
				}
				button { class: "btn", disabled: pending, onclick: move |_| actions::export_pathfinding_scores(ctx),
					if pending { Spinner {} "Exporting..." } else { Icon { name: "download", size: 16 } "Export Scores" }
				}
			},
		}
	}
}
