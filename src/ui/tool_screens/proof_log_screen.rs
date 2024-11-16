use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::proof_log_item::ProofLogItem;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::drive::grovedb::operations::proof::GroveDBProof;
use eframe::egui::{self, Context, Grid, ScrollArea, TextEdit, Ui};
use eframe::epaint::Vec2;
use std::ops::Range;
use std::sync::Arc;

/// Screen to visualize proofs from the proof log.
pub struct ProofLogScreen {
    pub(crate) app_context: Arc<AppContext>,
    proof_items: Vec<ProofLogItem>,
    selected_proof_index: Option<usize>,
    show_errors_only: bool,
    sort_column: ProofLogColumn,
    sort_ascending: bool,
    pagination_range: Range<u64>,
    items_per_page: u64,
    display_mode: DisplayMode,
}

#[derive(Clone, Copy)]
enum ProofLogColumn {
    RequestType,
    Height,
    Time,
    Error,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DisplayMode {
    Hex,
    Json,
}

impl ProofLogScreen {
    /// Creates a new ProofViewerScreen instance.
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            proof_items: Vec::new(),
            selected_proof_index: None,
            show_errors_only: false,
            sort_column: ProofLogColumn::Time,
            sort_ascending: false,
            pagination_range: 0..100,
            items_per_page: 100,
            display_mode: DisplayMode::Hex,
        }
    }

    /// Fetches proof log items from the database based on current settings.
    fn fetch_proof_items(&mut self) {
        let db = &self.app_context.db;
        if let Ok(mut items) =
            db.get_proof_log_items(self.show_errors_only, self.pagination_range.clone())
        {
            // Sort items based on current sorting settings
            items.sort_by(|a, b| {
                let ordering = match self.sort_column {
                    ProofLogColumn::RequestType => a.request_type.cmp(&b.request_type),
                    ProofLogColumn::Height => a.height.cmp(&b.height),
                    ProofLogColumn::Time => a.time_ms.cmp(&b.time_ms),
                    ProofLogColumn::Error => a.error.cmp(&b.error),
                };
                if self.sort_ascending {
                    ordering
                } else {
                    ordering.reverse()
                }
            });
            self.proof_items = items;
        }
    }

    /// Renders the left side of the screen with the list of proofs.
    fn render_proof_list(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut self.show_errors_only, "Show Errors Only")
                .changed()
            {
                self.fetch_proof_items();
            }
            ui.label("Items per page:");
            ui.add(egui::DragValue::new(&mut self.items_per_page).clamp_range(10..=1000));
            if ui.button("Refresh").clicked() {
                self.fetch_proof_items();
            }
        });

        // Table headers with sorting
        Grid::new("proof_log_table_headers")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                if ui
                    .button("Request Type")
                    .on_hover_text("Click to sort by Request Type")
                    .clicked()
                {
                    self.sort_column = ProofLogColumn::RequestType;
                    self.sort_ascending = !self.sort_ascending;
                    self.fetch_proof_items();
                }
                if ui
                    .button("Height")
                    .on_hover_text("Click to sort by Height")
                    .clicked()
                {
                    self.sort_column = ProofLogColumn::Height;
                    self.sort_ascending = !self.sort_ascending;
                    self.fetch_proof_items();
                }
                if ui
                    .button("Time")
                    .on_hover_text("Click to sort by Time")
                    .clicked()
                {
                    self.sort_column = ProofLogColumn::Time;
                    self.sort_ascending = !self.sort_ascending;
                    self.fetch_proof_items();
                }
                if ui
                    .button("Error")
                    .on_hover_text("Click to sort by Error")
                    .clicked()
                {
                    self.sort_column = ProofLogColumn::Error;
                    self.sort_ascending = !self.sort_ascending;
                    self.fetch_proof_items();
                }
                ui.end_row();
            });

        // Scrollable area for proof items
        ScrollArea::vertical().show(ui, |ui| {
            for (index, item) in self.proof_items.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            self.selected_proof_index == Some(index),
                            format!("{:?}", item.request_type),
                        )
                        .clicked()
                    {
                        self.selected_proof_index = Some(index);
                    }
                    ui.label(item.height.to_string());
                    ui.label(item.time_ms.to_string());
                    ui.label(
                        item.error
                            .as_ref()
                            .map_or("No Error".to_string(), |e| e.clone()),
                    );
                });
            }
        });

        // Pagination controls
        ui.horizontal(|ui| {
            if ui.button("Previous").clicked() && self.pagination_range.start >= self.items_per_page
            {
                self.pagination_range = (self.pagination_range.start - self.items_per_page)
                    ..(self.pagination_range.end - self.items_per_page);
                self.fetch_proof_items();
            }
            if ui.button("Next").clicked() {
                self.pagination_range = (self.pagination_range.start + self.items_per_page)
                    ..(self.pagination_range.end + self.items_per_page);
                self.fetch_proof_items();
            }
            ui.label(format!(
                "Showing items {} to {}",
                self.pagination_range.start + 1,
                self.pagination_range.end
            ));
        });
    }

    /// Renders the right side of the screen with proof details.
    fn render_proof_details(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Display Mode:");
            ui.radio_value(&mut self.display_mode, DisplayMode::Hex, "Hex");
            ui.radio_value(&mut self.display_mode, DisplayMode::Json, "JSON");
        });

        if let Some(index) = self.selected_proof_index {
            if let Some(proof_item) = self.proof_items.get(index) {
                // Display basic information
                ui.label(format!("Request Type: {:?}", proof_item.request_type));
                ui.label(format!("Height: {}", proof_item.height));
                ui.label(format!("Time: {}", proof_item.time_ms));
                if let Some(error) = &proof_item.error {
                    ui.label(format!("Error: {}", error));
                } else {
                    ui.label("Error: None");
                }
                //display grovedb_proof info

                // Display proof based on display mode
                let proof_display = match self.display_mode {
                    DisplayMode::Hex => hex::encode(&proof_item.proof_bytes),
                    DisplayMode::Json => {
                        let config = bincode::config::standard()
                            .with_big_endian()
                            .with_no_limit();
                        let grovedb_proof: Result<GroveDBProof, _> =
                            bincode::decode_from_slice(&proof_item.proof_bytes, config)
                                .map(|(a, _)| a);
                        match grovedb_proof {
                            Ok(proof) => format!("{}", proof),
                            Err(_) => "Invalid GroveDBProof".to_string(),
                        }
                    }
                };

                // Display proof in a text area
                ui.add(
                    TextEdit::multiline(&mut proof_display.clone())
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_rows(20),
                );
            }
        } else {
            ui.label("No proof selected.");
        }
    }
}

impl ScreenLike for ProofLogScreen {
    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Implement message display if needed
    }

    /// Renders the UI components for the proof viewer screen.
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Proof Viewer", AppAction::None)],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            // Fetch proof items if not already fetched
            if self.proof_items.is_empty() {
                self.fetch_proof_items();
            }

            ui.horizontal(|ui| {
                let available_width = ui.available_width();
                let half_width = available_width / 2.0;

                // Left side: Proof list
                ui.allocate_ui_with_layout(
                    Vec2::new(half_width, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.render_proof_list(ui);
                    },
                );

                // Right side: Proof details
                ui.allocate_ui_with_layout(
                    Vec2::new(half_width, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.render_proof_details(ui);
                    },
                );
            });
        });

        action
    }
}
