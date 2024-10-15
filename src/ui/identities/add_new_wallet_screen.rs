use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use eframe::egui::Context;

use crate::model::wallet::Wallet;
use crate::ui::components::entropy_grid::U256EntropyGrid;
use bip39::{Language, Mnemonic};
use egui::{
    Color32, ComboBox, Direction, FontId, Frame, Grid, Layout, Margin, RichText, Stroke, TextStyle,
    Ui, Vec2,
};
use std::sync::Arc;

pub struct AddNewWalletScreen {
    seed_phrase: Option<Mnemonic>,
    passphrase: String,
    entropy_grid: U256EntropyGrid,
    selected_language: Language,
    alias_input: String,
    pub app_context: Arc<AppContext>,
}

impl AddNewWalletScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            seed_phrase: None,
            passphrase: String::new(),
            entropy_grid: U256EntropyGrid::new(),
            selected_language: Language::English,
            alias_input: String::new(),
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

    fn save_wallet(&mut self) -> AppAction {
        if let Some(mnemonic) = &self.seed_phrase {
            let seed = mnemonic.to_seed(self.passphrase.as_str());
            let wallet = Wallet {
                seed,
                alias: None,
                is_main: true,
                password_hint: None,
            };

            self.app_context
                .db
                .insert_wallet(&wallet, &self.app_context.network)
                .ok();

            // Acquire a write lock and add the new wallet
            if let Ok(mut wallets) = self.app_context.wallets.write() {
                wallets.push(wallet);
            } else {
                eprintln!("Failed to acquire write lock on wallets");
            }

            AppAction::GoToMainScreen // Navigate back to the main screen after saving
        } else {
            AppAction::None // No action if no seed phrase exists
        }
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
}

impl ScreenLike for AddNewWalletScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Create Wallet", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.heading("Follow these steps to create your wallet!");
            ui.add_space(5.0);

            self.entropy_grid.ui(ui);

            ui.add_space(5.0);

            ui.heading("2. Select your desired seed phrase language and press \"Generate\"");

            self.render_seed_phrase_input(ui);

            ui.add_space(20.0); // Add space before the button

            // Centered "Save Wallet" button at the bottom
            ui.with_layout(Layout::centered_and_justified(Direction::TopDown), |ui| {
                let save_button = egui::Button::new(
                    RichText::new("Save Wallet").strong().size(30.0), // Bold, large text
                )
                .min_size(Vec2::new(300.0, 60.0)) // Large button size
                .rounding(10.0) // Rounded corners
                .stroke(Stroke::new(1.5, Color32::WHITE)); // White border

                if ui.add(save_button).clicked() {
                    action = self.save_wallet(); // Trigger the save action
                }
            });
        });

        action
    }
}
