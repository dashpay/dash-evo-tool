use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::proof_log_item::ProofLogItem;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::drive::grovedb::operations::proof::GroveDBProof;
use dash_sdk::drive::query::PathQuery;
use eframe::egui::{self, Context, Grid, ScrollArea, TextEdit, Ui};
use egui::text::LayoutJob;
use egui::{Color32, FontId, Frame, Stroke, TextFormat, TextStyle, Vec2};
use regex::Regex;
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

fn extract_hashes_from_error(error: &str) -> Vec<String> {
    let re = Regex::new(r"[a-fA-F0-9]{64}").unwrap();
    re.find_iter(error)
        .map(|mat| mat.as_str().to_string())
        .collect()
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
    PathQuery,
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
            ui.add(egui::DragValue::new(&mut self.items_per_page).range(10..=1000));
            if ui.button("Refresh").clicked() {
                self.fetch_proof_items();
            }
        });

        // Scrollable area for the table
        ScrollArea::vertical()
            .id_salt("proof_list_scroll_area")
            .show(ui, |ui| {
                Grid::new("proof_log_table")
                    .num_columns(4)
                    .striped(true)
                    .show(ui, |ui| {
                        // Table headers with sorting
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

                        // Data rows
                        for (index, item) in self.proof_items.iter().enumerate() {
                            // First column: selectable label for Request Type
                            if ui
                                .selectable_label(
                                    self.selected_proof_index == Some(index),
                                    format!("{:?}", item.request_type),
                                )
                                .clicked()
                            {
                                self.selected_proof_index = Some(index);
                            }

                            // Second column: Height
                            ui.label(item.height.to_string());

                            // Third column: Time
                            ui.label(item.time_ms.to_string());

                            // Fourth column: Error (first 20 chars, full error on hover)
                            let error_text =
                                item.error.as_ref().map_or("No Error".to_string(), |e| {
                                    if e.len() > 20 {
                                        format!("{}...", &e[..20])
                                    } else {
                                        e.clone()
                                    }
                                });
                            ui.label(error_text)
                                .on_hover_text(item.error.as_deref().unwrap_or("No Error"));

                            ui.end_row();
                        }
                    });
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

    fn highlight_proof_text(proof_text: &str, hashes: &[String], font_id: FontId) -> LayoutJob {
        let mut job = LayoutJob::default();
        let mut remaining_text = proof_text;

        while !remaining_text.is_empty() {
            let empty_string = String::new();
            // Find the earliest occurrence of any hash
            let (earliest_pos, matched_hash) = hashes
                .iter()
                .filter_map(|hash| remaining_text.find(hash).map(|pos| (pos, hash)))
                .min_by_key(|&(pos, _)| pos)
                .unwrap_or((remaining_text.len(), &empty_string));

            // Add text before the matched hash
            if earliest_pos > 0 {
                let before_text = &remaining_text[..earliest_pos];
                job.append(
                    before_text,
                    0.0,
                    TextFormat {
                        font_id: font_id.clone(),
                        color: Color32::BLACK,
                        ..Default::default()
                    },
                );
            }

            if !matched_hash.is_empty() {
                // Add the matched hash with highlight
                job.append(
                    matched_hash,
                    0.0,
                    TextFormat {
                        font_id: font_id.clone(),
                        color: Color32::from_rgb(0x9b, 0x87, 0x0c), // Highlight color
                        ..Default::default()
                    },
                );
                // Move past the hash
                remaining_text = &remaining_text[earliest_pos + matched_hash.len()..];
            } else {
                break;
            }
        }

        job
    }

    /// Renders the right side of the screen with proof details.
    fn render_proof_details(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Display Mode:");
            ui.radio_value(&mut self.display_mode, DisplayMode::Hex, "Hex");
            ui.radio_value(&mut self.display_mode, DisplayMode::Json, "JSON");
            ui.radio_value(&mut self.display_mode, DisplayMode::PathQuery, "Path Query");
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

                // Display proof based on display mode
                let (proof_display, hashes) = match self.display_mode {
                    DisplayMode::Hex => {
                        let encoded = hex::encode(&proof_item.proof_bytes);
                        // Extract hashes from the error message
                        let hashes = if let Some(error) = &proof_item.error {
                            extract_hashes_from_error(error)
                        } else {
                            Vec::new()
                        };
                        (encoded, hashes)
                    }
                    DisplayMode::Json => {
                        let hashes = if let Some(error) = &proof_item.error {
                            extract_hashes_from_error(error)
                        } else {
                            Vec::new()
                        };
                        let config = bincode::config::standard()
                            .with_big_endian()
                            .with_no_limit();
                        let grovedb_proof: Result<GroveDBProof, _> =
                            bincode::decode_from_slice(&proof_item.proof_bytes, config)
                                .map(|(a, _)| a);
                        let text = match grovedb_proof {
                            Ok(proof) => format!("{}", proof),
                            Err(_) => "Invalid GroveDBProof".to_string(),
                        };
                        (text, hashes)
                    }
                    DisplayMode::PathQuery => {
                        let config = bincode::config::standard()
                            .with_big_endian()
                            .with_no_limit();
                        let verification_path_query: Result<PathQuery, _> =
                            bincode::decode_from_slice(
                                &proof_item.verification_path_query_bytes,
                                config,
                            )
                            .map(|(a, _)| a);
                        let text = match verification_path_query {
                            Ok(path_query) => format!("{}", path_query),
                            Err(_) => "Invalid Path Query".to_string(),
                        };
                        (text, vec![])
                    }
                };

                // Create the layout job with highlighted hashes
                let font_id = TextStyle::Monospace.resolve(ui.style());
                let layout_job = Self::highlight_proof_text(&proof_display, &hashes, font_id);

                let frame = Frame::none()
                    .stroke(Stroke::new(1.0, Color32::BLACK))
                    .fill(Color32::TRANSPARENT)
                    .rounding(2.0); // Set margins to zero

                frame.show(ui, |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), 300.0));

                    ScrollArea::vertical()
                        .id_salt("proof_display_scroll_area")
                        .show(ui, |ui| {
                            ui.label(layout_job);
                        });
                });
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

    fn refresh_on_arrival(&mut self) {
        self.fetch_proof_items()
    }

    /// Renders the UI components for the proof viewer screen.
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Proof Viewer", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenToolsProofLogScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        egui::CentralPanel::default().show(ctx, |ui| {
            // Fetch proof items if not already fetched
            if self.proof_items.is_empty() {
                self.fetch_proof_items();
            }

            ui.columns(2, |columns| {
                // Left side: Proof list
                columns[0].with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ScrollArea::vertical()
                        .id_salt("proof_list_scroll_area")
                        .show(ui, |ui| {
                            self.render_proof_list(ui);
                        });
                });

                // Right side: Proof details
                columns[1].with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ScrollArea::vertical()
                        .id_salt("proof_details_scroll_area")
                        .show(ui, |ui| {
                            self.render_proof_details(ui);
                        });
                });
            });
        });

        action
    }
}
