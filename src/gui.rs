use eframe::{egui, App, Frame};
use egui::{epaint::{self, Margin}, TextureHandle, TextureOptions};
use image::{GrayImage, Luma};
use std::{fs, path::PathBuf, str::FromStr, time::{Duration, Instant}};
use dirs_next as dirs;
use qrcode::{Color, QrCode};
use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network},
    lightning::{ln::msgs::SocketAddress, ln::types::ChannelId},
    Node, Event
};
use ldk_node::lightning_invoice::Bolt11InvoiceDescription;

use crate::config::Config;
use crate::state::{StateManager, StabilityAction};
use crate::types::{Bitcoin, StableChannel, USD};
use crate::make_node;

use crate::config::{ComponentType};

// Enum to track the application state
enum UIState {
    OnboardingScreen,
    WaitingForPayment,
    MainScreen,
    ClosingScreen
}

// Main application structure
pub struct StableChannelsApp {
    state: UIState,
    last_stability_check: Instant,
    invoice_result: String,
    state_manager: StateManager,
    qr_texture: Option<TextureHandle>,
    status_message: String,
    close_channel_address: String,
    config: Config,
}

impl StableChannelsApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = Config::get_or_create_for_component(ComponentType::Gui);
    
        // Ensure directories exist
        if let Err(e) = config.ensure_directories_exist() {
            eprintln!("Warning: Failed to create directories: {}", e);
        }
    
        // Parse LSP pubkey
        let lsp_pubkey_bytes = match hex::decode(&config.lsp.pubkey) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("Error decoding LSP pubkey: {:?}", e);
                vec![0; 33] // Fallback to empty pubkey
            }
        };
    
        let lsp_pubkey = match PublicKey::from_slice(&lsp_pubkey_bytes) {
            Ok(key) => Some(key),
            Err(e) => {
                eprintln!("Error parsing LSP pubkey: {:?}", e);
                None
            }
        };
    
        let is_service = false; 
        let user = make_node(&config, lsp_pubkey, is_service);
        
        let state_manager = StateManager::new(user);

        let channels = state_manager.node().list_channels();
        let state = if channels.is_empty() {
            UIState::OnboardingScreen
        } else {
            UIState::MainScreen
        }; 

        Self {
            state,
            last_stability_check: Instant::now() - Duration::from_secs(60),
            invoice_result: String::new(),
            state_manager,
            qr_texture: None,
            status_message: String::new(),
            close_channel_address: String::new(),
            config,
        }
    }

    fn check_stability(&mut self) {
        let action = self.state_manager.check_stability();
        
        match action {
            StabilityAction::DoNothing => {
                self.status_message = "Difference from par less than 0.1%. Stable.".to_string();
            },
            StabilityAction::Wait => {
                self.status_message = "Waiting for payment from counterparty...".to_string();
            },
            StabilityAction::Pay(amt) => {
                self.status_message = "Paying the difference...".to_string();
                
                match self.state_manager.execute_payment(amt) {
                    Ok(payment_id) => {
                        self.status_message = format!("Payment sent successfully with ID: {}", payment_id);
                        println!("Payment sent successfully with payment ID: {}", payment_id);
                    },
                    Err(e) => {
                        self.status_message = format!("Failed to send payment: {}", e);
                        println!("Failed to send payment: {}", e);
                    }
                }
            },
            StabilityAction::HighRisk(risk_level) => {
                self.status_message = format!("Risk level high: {}", risk_level);
            },

            StabilityAction::NotInitialized => {
                self.status_message = "Channel not properly initialized. Please create a channel first.".to_string();
                println!("Channel not properly initialized. Please create a channel first.");
            }
        }
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {    
        let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
            ldk_node::lightning_invoice::Description::new("Stable Channel JIT payment".to_string()).unwrap()
        );
        
        // Use the amount from the config
        let amount_msats = (self.config.stable_channel_defaults.expected_usd * 1_000_000.0) as u64;

        let result = self.state_manager.node().bolt11_payment().receive_via_jit_channel(
            amount_msats,
            &description,
            3600,
            Some(10_000_000),
        );
    
        match result {
            Ok(invoice) => {
                self.invoice_result = invoice.to_string();
                let code = QrCode::new(&self.invoice_result).unwrap_or_else(|_| QrCode::new("Error generating QR").unwrap());
                let bits = code.to_colors();
                let width = code.width();
                let scale_factor = 4;
                let mut imgbuf =
                    GrayImage::new((width * scale_factor) as u32, (width * scale_factor) as u32);
    
                for y in 0..width {
                    for x in 0..width {
                        let color = if bits[y * width + x] == Color::Dark {
                            0
                        } else {
                            255
                        };
                        for dy in 0..scale_factor {
                            for dx in 0..scale_factor {
                                imgbuf.put_pixel(
                                    (x * scale_factor + dx) as u32,
                                    (y * scale_factor + dy) as u32,
                                    Luma([color]),
                                );
                            }
                        }
                    }
                }
                let (w, h) = (imgbuf.width() as usize, imgbuf.height() as usize);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for pixel in imgbuf.pixels() {
                    let lum = pixel[0];
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(255);
                }
                let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                self.qr_texture =
                    Some(ctx.load_texture("qr_code", color_image, TextureOptions::LINEAR));
            }
            Err(e) => {
                self.invoice_result = format!("Error: {e:?}");
            }
        }
    }

    fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Stable Channels v0.2")
                        .size(28.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(50.0);
    
                // Step 1
                ui.heading(
                    egui::RichText::new("Step 1: Get a Lightning invoice ⚡")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(r#"Press the "Stabilize" button below."#)
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 2
                ui.heading(
                    egui::RichText::new("Step 2: Send yourself bitcoin 💸")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Over Lightning, from an app or an exchange.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 3
                ui.heading(
                    egui::RichText::new("Step 3: Stable channel created 🔧")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Self-custody. Your keys, your coins.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(50.0);
    
                // Create channel button
                let subtle_orange = egui::Color32::from_rgba_premultiplied(247, 147, 26, 200); 
                let create_channel_button = egui::Button::new(
                    egui::RichText::new("Stabilize")
                        .color(egui::Color32::WHITE)
                        .strong()
                        .size(18.0),
                )
                .min_size(egui::vec2(200.0, 55.0))
                .fill(subtle_orange)
                .rounding(8.0);
    
                if ui.add(create_channel_button).clicked() {
                    self.get_jit_invoice(ctx);
                    self.state = UIState::WaitingForPayment;
                }
            });
        });
    }

    fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);

            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Send yourself bitcoin to stabilize.")
                        .size(16.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(3.0);
                ui.label("This is a Bolt11 Lightning invoice.");
                ui.add_space(8.0);

                if let Some(ref qr) = self.qr_texture {
                    ui.image(qr);
                } else {
                    ui.label("Lightning QR Missing");
                }

                ui.add_space(8.0);

                ui.add(
                    egui::TextEdit::multiline(&mut self.invoice_result)
                        .frame(true)
                        .desired_width(400.0)
                        .desired_rows(3)
                        .hint_text("Invoice..."),
                );

                ui.add_space(8.0);

                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Copy Invoice")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0),
                ).clicked() {
                    ctx.output_mut(|o| {
                        o.copied_text = self.invoice_result.clone();
                    });
                }
                
                ui.add_space(5.0); 
                
                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Back")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0), 
                ).clicked() {
                    self.state = UIState::OnboardingScreen;
                }
                
                ui.add_space(8.0); 
            });
        });
    }

    fn show_main_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Frame::none()
                .inner_margin(epaint::Margin::symmetric(20.0, 0.0))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        // --- Existing Balance UI ---
                        let balances = self.state_manager.node().list_balances();
                        let sc = self.state_manager.get_stable_channel();
                        let lightning_balance_btc = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                        let lightning_balance_usd = USD::from_bitcoin(lightning_balance_btc, sc.latest_price);
              
                        ui.add_space(30.0);

                        ui.group(|ui| {
                            ui.add_space(20.0);
                            ui.heading("Your Stable Balance");
                            ui.add(egui::Label::new(
                                egui::RichText::new(lightning_balance_usd.to_string())
                                    .size(36.0)
                                    .strong(),
                            ));
                            ui.label(format!("Agreed Peg USD: {}", sc.expected_usd));
                            ui.label(format!("Bitcoin: {}", lightning_balance_btc.to_string()));
                            ui.add_space(20.0);
                        });

                        ui.add_space(20.0);

                        ui.group(|ui| {
                            ui.add_space(20.0);
                            ui.heading("Bitcoin Price");
                            ui.label(format!("${:.2}", sc.latest_price));
                            ui.add_space(20.0);

                            let last_updated = self.last_stability_check.elapsed().as_secs();
                            ui.add_space(5.0);
                            ui.label(
                                egui::RichText::new(format!("Last updated: {}s ago", last_updated))
                                    .size(12.0)
                                    .color(egui::Color32::GRAY),
                            );
                        });

                        ui.add_space(20.0);

                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                ui.collapsing("Close Channel", |ui| {
                                    ui.label("Withdrawal address (minus transaction fees):");
                                    ui.add_space(10.0);
                                    ui.text_edit_singleline(&mut self.close_channel_address);
                                    ui.add_space(10.0);

                                    if ui.add(
                                        egui::Button::new(
                                            egui::RichText::new("Close Channel")
                                                .color(egui::Color32::WHITE)
                                                .size(12.0),
                                        )
                                        .rounding(6.0),
                                    )
                                    .clicked()
                                    {
                                        self.close_all_channels_to_address();
                                    }
                                });

                                ui.add_space(20.0);

                                if !self.status_message.is_empty() {
                                    ui.label(self.status_message.clone());
                                }
                            });
                    });
                });
        });
    }

    fn show_closing_screen(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.heading(
                    egui::RichText::new(format!("Withdrawal processing")).size(28.0).strong(),
                );    
            });
    
            ui.add_space(20.0);
            ui.horizontal_centered(|ui| {
                ui.heading(                    
                    egui::RichText::new(format!("{}",self.close_channel_address)).size(28.0).strong(), 
                );
            });
        });
    }

    fn poll_for_events(&mut self) {
        while let Some(event) = self.state_manager.node().next_event() {
            match event {
                Event::ChannelReady { .. } => {
                    // Once we have a channel, initialize the stable channel
                    let channels = self.state_manager.node().list_channels();
                    if let Some(channel) = channels.first() {
                        // Use the first channel we find
                        if let Err(e) = self.state_manager.initialize_stable_channel(
                            &channel.channel_id.to_string(),
                            true, // default to stable receiver
                            self.config.stable_channel_defaults.expected_usd,
                            0.0, // no native bitcoin amount
                        ) {
                            eprintln!("Error initializing stable channel: {:?}", e);
                        }
                    }
                    self.check_stability();
                    self.state = UIState::MainScreen;
                }
                
                Event::PaymentReceived { .. } => {
                    self.state = UIState::MainScreen;
                    println!("Payment received");
                }

                Event::ChannelClosed { .. } => {
                    if self.state_manager.node().list_channels().is_empty() {
                        println!("All channels closed, returning to onboarding screen");
                        self.state = UIState::OnboardingScreen;
                    } else {
                        self.state = UIState::ClosingScreen;
                        println!("Channel closed, but other channels still exist");
                    }
                }
                _ => {}
            }
            self.state_manager.node().event_handled();
        }
    }

    fn close_all_channels_to_address(&mut self) {
        if self.close_channel_address.is_empty() {
            self.status_message = "Please enter a withdrawal address".to_string();
            return;
        }

        for channel in self.state_manager.node().list_channels().iter() {
            let user_channel_id = channel.user_channel_id.clone();
            let counterparty_node_id = channel.counterparty_node_id;
            match self.state_manager.node().close_channel(&user_channel_id, counterparty_node_id) {
                Ok(_) => self.status_message = "Closing channel...".to_string(),
                Err(e) => self.status_message = format!("Error closing channel: {}", e),
            }
        }

        // Withdraw everything to address
        match ldk_node::bitcoin::Address::from_str(&self.close_channel_address) {
            Ok(addr) => {
                let network = Network::Signet;
                
                match addr.require_network(network) {
                    Ok(addr_checked) => {
                        match self.state_manager.node().onchain_payment().send_all_to_address(&addr_checked, false, None) {
                            Ok(txid) => {
                                self.status_message = format!("Withdrawal transaction sent: {}", txid);
                                self.state = UIState::ClosingScreen;
                            },
                            Err(e) => self.status_message = format!("Error sending withdrawal: {}", e),
                        }
                    },
                    Err(_) => self.status_message = "Invalid address for this network".to_string(),
                }
            },
            Err(_) => self.status_message = "Invalid address format".to_string(),
        }
    }
}

impl App for StableChannelsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        let now = Instant::now();
        
        if now.duration_since(self.last_stability_check) >= Duration::from_secs(30) {
            self.check_stability();
            self.last_stability_check = now;
        }

        match self.state {
            UIState::OnboardingScreen => self.show_onboarding_screen(ctx),
            UIState::WaitingForPayment => self.show_waiting_for_payment_screen(ctx),
            UIState::MainScreen => self.show_main_screen(ctx),
            UIState::ClosingScreen => self.show_closing_screen(ctx),
        }

        self.poll_for_events();
    }
}

pub fn launch_app() {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([460.0, 700.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "Stable Channels",
        native_options,
        Box::new(|cc| {
            Ok(Box::new(StableChannelsApp::new(cc)))
        }),
    ).unwrap_or_else(|e| {
        eprintln!("Error running application: {:?}", e);
    });
}