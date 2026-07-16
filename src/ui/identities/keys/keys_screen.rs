use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::ScreenLike;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use eframe::egui::{self};
use std::sync::Arc;

pub struct KeysScreen {
    pub identity: Identity,
    pub app_context: Arc<AppContext>,
}

impl ScreenLike for KeysScreen {
    fn refresh(&mut self) {}

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    action = AppAction::PopScreen;
                }
                ui.heading("Identity Keys");
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Key ID");
                    ui.label("Purpose");
                    ui.label("Security Level");
                    ui.label("Type");
                    ui.label("Read Only");
                });
                for key in self.identity.public_keys().values() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}", key.id()));
                        ui.label(format!("{:?}", key.purpose()));
                        ui.label(format!("{:?}", key.security_level()));
                        ui.label(format!("{:?}", key.key_type()));
                        ui.label(format!("{}", key.read_only()));
                    });
                }
            });
        });
        action
    }
}

impl KeysScreen {
    pub fn new(identity: Identity, app_context: &Arc<AppContext>) -> Self {
        Self {
            identity,
            app_context: app_context.clone(),
        }
    }
}
