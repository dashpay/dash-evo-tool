use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dpp::identity::Identity;
use dpp::prelude::IdentityPublicKey;
use eframe::egui::{self, Context};
use egui::{RichText, TextEdit};
use std::sync::Arc;

pub struct KeyInfoScreen {
    pub identity: Identity,
    pub key: IdentityPublicKey,
    pub private_key_bytes: Option<Vec<u8>>,
    pub app_context: Arc<AppContext>,
}

impl ScreenLike for KeyInfoScreen {
    fn refresh(&mut self) {}

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Key Info", AppAction::None),
            ],
            None,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Key Information");

            egui::Grid::new("key_info_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .striped(true)
                .show(ui, |ui| {
                    // Key ID
                    ui.label(RichText::new("Key ID:").strong());
                    ui.label(format!("{}", self.key.id()));
                    ui.end_row();

                    // Purpose
                    ui.label(RichText::new("Purpose:").strong());
                    ui.label(format!("{:?}", self.key.purpose()));
                    ui.end_row();

                    // Security Level
                    ui.label(RichText::new("Security Level:").strong());
                    ui.label(format!("{:?}", self.key.security_level()));
                    ui.end_row();

                    // Type
                    ui.label(RichText::new("Type:").strong());
                    ui.label(format!("{:?}", self.key.key_type()));
                    ui.end_row();

                    // Read Only
                    ui.label(RichText::new("Read Only:").strong());
                    ui.label(format!("{}", self.key.read_only()));
                    ui.end_row();
                });

            ui.separator();

            // Display the private key if available
            if let Some(private_key) = &self.private_key_bytes {
                ui.label("Private Key:");
                let private_key_hex = hex::encode(private_key);
                ui.add(
                    TextEdit::multiline(&mut private_key_hex.as_str().to_owned())
                        .desired_width(f32::INFINITY),
                );
            } else {
                ui.label(RichText::new("No Private Key Available").color(egui::Color32::GRAY));
            }
        });

        action
    }
}

impl KeyInfoScreen {
    pub fn new(
        identity: Identity,
        key: IdentityPublicKey,
        private_key_bytes: Option<Vec<u8>>,
        app_context: &Arc<AppContext>,
    ) -> Self {
        Self {
            identity,
            key,
            private_key_bytes,
            app_context: app_context.clone(),
        }
    }
}
