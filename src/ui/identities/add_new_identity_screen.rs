use crate::app::AppAction;
use crate::context::AppContext;
use crate::platform::identity::{IdentityRegistrationInfo, IdentityTask};
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use dash_sdk::dpp::identity::{KeyType, TimestampMillis};
use eframe::egui::Context;

use crate::ui::components::entropy_grid::U256EntropyGrid;
use bip39::{Language, Mnemonic};
use egui::{
    Color32, ComboBox, Direction, FontId, Frame, Grid, Layout, Margin, RichText, ScrollArea,
    Stroke, TextStyle, Ui, Vec2,
};
use serde::Deserialize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize)]
struct KeyInfo {
    address: String,
    #[serde(rename = "private_key")]
    private_key: String,
}

pub enum AddIdentityStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct AddNewIdentityScreen {
    seed_phrase: Option<Mnemonic>,
    entropy_grid: U256EntropyGrid,
    selected_language: Language,
    identity_id_input: String,
    alias_input: String,
    master_private_key_input: String,
    master_private_key_type: KeyType,
    keys_input: Vec<(String, KeyType)>,
    add_identity_status: AddIdentityStatus,
    pub app_context: Arc<AppContext>,
}

impl AddNewIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            seed_phrase: None,
            entropy_grid: U256EntropyGrid::new(),
            selected_language: Language::English,
            identity_id_input: String::new(),
            alias_input: String::new(),
            master_private_key_input: String::new(),
            master_private_key_type: KeyType::ECDSA_HASH160,
            keys_input: vec![(String::new(), KeyType::ECDSA_HASH160)],
            add_identity_status: AddIdentityStatus::NotStarted,
            app_context: app_context.clone(),
        }
    }

    /// Generate a new seed phrase based on the selected language
    fn generate_seed_phrase(&mut self) {
        let mnemonic = Mnemonic::from_entropy_in(
            self.selected_language,
            &self.entropy_grid.random_number_with_user_input(),
        )
        .expect("Failed to generate mnemonic");
        self.seed_phrase = Some(mnemonic);
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        let mut keys_to_remove = vec![];

        for (i, (key, key_type)) in self.keys_input.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Key {}:", i + 1));
                ui.text_edit_singleline(key);

                ComboBox::from_label("Key Type")
                    .selected_text(format!("{:?}", key_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(key_type, KeyType::ECDSA_SECP256K1, "ECDSA_SECP256K1");
                        ui.selectable_value(key_type, KeyType::BLS12_381, "BLS12_381");
                        ui.selectable_value(key_type, KeyType::ECDSA_HASH160, "ECDSA_HASH160");
                        ui.selectable_value(
                            key_type,
                            KeyType::BIP13_SCRIPT_HASH,
                            "BIP13_SCRIPT_HASH",
                        );
                        ui.selectable_value(
                            key_type,
                            KeyType::EDDSA_25519_HASH160,
                            "EDDSA_25519_HASH160",
                        );
                    });

                if ui.button("-").clicked() {
                    keys_to_remove.push(i);
                }
            });
        }

        for i in keys_to_remove.iter().rev() {
            self.keys_input.remove(*i);
        }

        if ui.button("+ Add Key").clicked() {
            self.keys_input
                .push((String::new(), KeyType::ECDSA_HASH160));
        }
    }

    fn register_identity_clicked(&mut self) -> AppAction {
        let identity_input = IdentityRegistrationInfo {
            identity_id_input: self.identity_id_input.trim().to_string(),
            alias_input: self.alias_input.clone(),
            master_private_key_input: self.master_private_key_input.clone(),
            master_private_key_type: self.master_private_key_type,
            keys_input: self.keys_input.clone(),
        };

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
            identity_input,
        )))
    }

    fn render_seed_phrase_input(&mut self, ui: &mut Ui) {
        ui.add_space(15.0); // Add spacing from the top
        ui.vertical(|ui| {
            // Allocate a full-width container to center align the elements
            let available_width = ui.available_width();

            ui.allocate_ui_with_layout(
                Vec2::new(available_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.horizontal(|ui| {
                        // Add spacing to align the combo box to the left of the center
                        let half_width = available_width / 2.0 - 400.0; // Adjust half-width with padding
                        ui.add_space(half_width);

                        let mut style = ui.style_mut();

                        // Customize text size for the ComboBox
                        style.text_styles.insert(
                            TextStyle::Button,          // Apply style to buttons (used in ComboBox entries)
                            FontId::proportional(24.0), // Set larger font size
                        );

                        ComboBox::from_label("")
                            .selected_text(format!("{:?}", self.selected_language))
                            .width(200.0)
                            .height(40.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.selected_language,
                                    Language::English,
                                    "English",
                                );
                                ui.selectable_value(
                                    &mut self.selected_language,
                                    Language::Spanish,
                                    "Spanish",
                                );
                                ui.selectable_value(
                                    &mut self.selected_language,
                                    Language::French,
                                    "French",
                                );
                                ui.selectable_value(
                                    &mut self.selected_language,
                                    Language::Italian,
                                    "Italian",
                                );
                                ui.selectable_value(
                                    &mut self.selected_language,
                                    Language::Portuguese,
                                    "Portuguese",
                                );
                            });

                        // Add a spacer between the combo box and the generate button
                        ui.add_space(20.0); // Adjust the space between elements

                        let generate_button =
                            egui::Button::new(RichText::new("Generate").strong().size(24.0))
                                .min_size(Vec2::new(150.0, 30.0))
                                .rounding(5.0)
                                .stroke(Stroke::new(1.0, Color32::WHITE));

                        if ui.add(generate_button).clicked() {
                            self.generate_seed_phrase();
                        }
                    });
                },
            );

            ui.add_space(10.0);

            // Create a container with a fixed width (72% of the available width)
            let frame_width = available_width * 0.72;
            ui.allocate_ui_with_layout(
                Vec2::new(frame_width, 300.0), // Set width and height of the container
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    Frame::none()
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0, Color32::BLACK))
                        .rounding(5.0)
                        .inner_margin(Margin::same(10.0))
                        .show(ui, |ui| {
                            let columns = 6;
                            let rows = 24 / columns;

                            // Calculate the size of each grid cell
                            let column_width = frame_width / columns as f32;
                            let row_height = 300.0 / rows as f32;

                            Grid::new("seed_phrase_grid")
                                .num_columns(columns)
                                .spacing((0.0, 0.0))
                                .min_col_width(column_width)
                                .min_row_height(row_height)
                                .show(ui, |ui| {
                                    if let Some(mnemonic) = &self.seed_phrase {
                                        for (i, word) in mnemonic.words().enumerate() {
                                            let word_text = RichText::new(word)
                                                .size(row_height * 0.5)
                                                .monospace();

                                            ui.with_layout(
                                                Layout::centered_and_justified(
                                                    Direction::LeftToRight,
                                                ),
                                                |ui| {
                                                    ui.label(word_text);
                                                },
                                            );

                                            if (i + 1) % columns == 0 {
                                                ui.end_row();
                                            }
                                        }
                                    } else {
                                        let word_text =
                                            RichText::new("Seed Phrase").size(40.0).monospace();

                                        ui.with_layout(
                                            Layout::centered_and_justified(Direction::LeftToRight),
                                            |ui| {
                                                ui.label(word_text);
                                            },
                                        );
                                    }
                                });
                        });
                },
            );
        });
    }

    fn render_master_key_input(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Master Private Key:");
            ui.text_edit_singleline(&mut self.master_private_key_input);

            ComboBox::from_label("Master Key Type")
                .selected_text(format!("{:?}", self.master_private_key_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::ECDSA_SECP256K1,
                        "ECDSA_SECP256K1",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::BLS12_381,
                        "BLS12_381",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::ECDSA_HASH160,
                        "ECDSA_HASH160",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::BIP13_SCRIPT_HASH,
                        "BIP13_SCRIPT_HASH",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::EDDSA_25519_HASH160,
                        "EDDSA_25519_HASH160",
                    );
                });
        });
    }
}

impl ScreenLike for AddNewIdentityScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Create Identity", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.heading("Follow these steps to create your identity!");
            ui.add_space(5.0);

            self.entropy_grid.ui(ui);

            ui.add_space(5.0);

            ui.heading("2. Select your desired seed phrase language and press \"Generate\"");

            self.render_seed_phrase_input(ui);
            self.render_master_key_input(ui);

            ui.horizontal(|ui| {
                ui.label("Identity ID (Hex or Base58):");
                ui.text_edit_singleline(&mut self.identity_id_input);
            });

            ui.horizontal(|ui| {
                ui.label("Alias:");
                ui.text_edit_singleline(&mut self.alias_input);
            });

            self.render_keys_input(ui);

            if ui.button("Create Identity").clicked() {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.add_identity_status = AddIdentityStatus::WaitingForResult(now);
                action = self.register_identity_clicked();
            }

            match &self.add_identity_status {
                AddIdentityStatus::NotStarted => {}
                AddIdentityStatus::WaitingForResult(start_time) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    let elapsed = now - start_time;

                    let display = if elapsed < 60 {
                        format!("{} second{}", elapsed, if elapsed == 1 { "" } else { "s" })
                    } else {
                        let minutes = elapsed / 60;
                        let seconds = elapsed % 60;
                        format!(
                            "{} minute{} and {} second{}",
                            minutes,
                            if minutes == 1 { "" } else { "s" },
                            seconds,
                            if seconds == 1 { "" } else { "s" }
                        )
                    };

                    ui.label(format!("Loading... Time taken so far: {}", display));
                }
                AddIdentityStatus::ErrorMessage(msg) => {
                    ui.label(format!("Error: {}", msg));
                }
                AddIdentityStatus::Complete => {
                    action = AppAction::PopScreenAndRefresh;
                }
            }
        });

        action
    }
}
