use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::ui::components::dashpay_subscreen_chooser_panel::add_dashpay_subscreen_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use egui::{Context, Ui};
use std::sync::Arc;

use super::contact_requests::ContactRequests;
use super::contacts_list::ContactsList;
use super::profile_screen::ProfileScreen;
use super::send_payment::PaymentHistory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashPaySubscreen {
    Contacts,
    Requests,
    Profile,
    Payments,
    ProfileSearch,
}

pub struct DashPayScreen {
    pub app_context: Arc<AppContext>,
    pub dashpay_subscreen: DashPaySubscreen,
    contacts_list: ContactsList,
    contact_requests: ContactRequests,
    profile_screen: ProfileScreen,
    payment_history: PaymentHistory,
}

impl DashPayScreen {
    pub fn new(app_context: &Arc<AppContext>, dashpay_subscreen: DashPaySubscreen) -> Self {
        Self {
            app_context: app_context.clone(),
            dashpay_subscreen,
            contacts_list: ContactsList::new(app_context.clone()),
            contact_requests: ContactRequests::new(app_context.clone()),
            profile_screen: ProfileScreen::new(app_context.clone()),
            payment_history: PaymentHistory::new(app_context.clone()),
        }
    }

    fn render_subscreen(&mut self, ui: &mut Ui) -> AppAction {
        match self.dashpay_subscreen {
            DashPaySubscreen::Contacts => self.contacts_list.render(ui),
            DashPaySubscreen::Requests => self.contact_requests.render(ui),
            DashPaySubscreen::Profile => self.profile_screen.render(ui),
            DashPaySubscreen::Payments => self.payment_history.render(ui),
            DashPaySubscreen::ProfileSearch => {
                // ProfileSearch is a separate screen, not embedded
                ui.label("Use the Search Profiles tab to search for public profiles");
                AppAction::None
            }
        }
    }
}

impl ScreenLike for DashPayScreen {
    fn refresh(&mut self) {
        match self.dashpay_subscreen {
            DashPaySubscreen::Contacts => {
                self.contacts_list.refresh();
            } // Ignore return value for now
            DashPaySubscreen::Requests => {
                self.contact_requests.refresh();
            } // Ignore return value for now
            DashPaySubscreen::Profile => self.profile_screen.refresh(),
            DashPaySubscreen::Payments => self.payment_history.refresh(),
            DashPaySubscreen::ProfileSearch => {
                // ProfileSearch is a separate screen, not embedded here
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh();
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        // Add top panel with action buttons based on current subscreen
        let right_buttons = match self.dashpay_subscreen {
            DashPaySubscreen::Contacts => vec![
                (
                    "Load Contacts",
                    DesiredAppAction::Custom("fetch_contacts".to_string()),
                ),
                (
                    "Send Contact Request",
                    DesiredAppAction::AddScreenType(Box::new(
                        crate::ui::ScreenType::DashPayAddContact,
                    )),
                ),
                (
                    "Generate QR Code",
                    DesiredAppAction::AddScreenType(Box::new(
                        crate::ui::ScreenType::DashPayQRGenerator,
                    )),
                ),
            ],
            DashPaySubscreen::Requests => vec![(
                "Load Contact Requests",
                DesiredAppAction::Custom("fetch_requests".to_string()),
            )],
            DashPaySubscreen::Profile => vec![(
                "Load Profile",
                DesiredAppAction::Custom("load_profile".to_string()),
            )],
            DashPaySubscreen::Payments => vec![(
                "Load Payments",
                DesiredAppAction::Custom("fetch_payment_history".to_string()),
            )],
            DashPaySubscreen::ProfileSearch => vec![],
        };

        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![("DashPay", AppAction::None)],
            right_buttons,
        );

        // Add left panel - map subscreen to appropriate RootScreenType
        let root_screen = match self.dashpay_subscreen {
            DashPaySubscreen::Contacts => RootScreenType::RootScreenDashPayContacts,
            DashPaySubscreen::Requests => RootScreenType::RootScreenDashPayRequests,
            DashPaySubscreen::Profile => RootScreenType::RootScreenDashPayProfile,
            DashPaySubscreen::Payments => RootScreenType::RootScreenDashPayPayments,
            DashPaySubscreen::ProfileSearch => RootScreenType::RootScreenDashPayProfileSearch,
        };
        action |= add_left_panel(ctx, &self.app_context, root_screen);

        // Add DashPay subscreen chooser panel on the left side
        action |=
            add_dashpay_subscreen_chooser_panel(ctx, &self.app_context, self.dashpay_subscreen);

        // Main content area with island styling
        action |= island_central_panel(ctx, |ui| self.render_subscreen(ui));

        // Handle custom actions from top panel buttons
        if let AppAction::Custom(command) = &action {
            match command.as_str() {
                "fetch_contacts" => {
                    action = self.contacts_list.trigger_fetch_contacts();
                }
                "fetch_requests" => {
                    action = self.contact_requests.trigger_fetch_requests();
                }
                "load_profile" => {
                    action = self.profile_screen.trigger_load_profile();
                }
                "fetch_payment_history" => {
                    action = self.payment_history.trigger_fetch_payment_history();
                }
                _ => {}
            }
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match self.dashpay_subscreen {
            DashPaySubscreen::Contacts => self.contacts_list.display_message(message, message_type),
            DashPaySubscreen::Requests => {
                self.contact_requests.display_message(message, message_type)
            }
            DashPaySubscreen::Profile => self.profile_screen.display_message(message, message_type),
            DashPaySubscreen::Payments => {
                self.payment_history.display_message(message, message_type)
            }
            DashPaySubscreen::ProfileSearch => {
                // ProfileSearch is a separate screen, not embedded here
            }
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match self.dashpay_subscreen {
            DashPaySubscreen::Profile => self.profile_screen.display_task_result(result),
            DashPaySubscreen::Requests => self.contact_requests.display_task_result(result),
            DashPaySubscreen::Contacts => self.contacts_list.display_task_result(result),
            DashPaySubscreen::Payments => self.payment_history.display_task_result(result),
            DashPaySubscreen::ProfileSearch => {
                // ProfileSearch is a separate screen, not embedded here
            }
        }
    }
}
