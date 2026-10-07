//! Date-and-time input in the user's local time zone, for scheduling deferred work.

use chrono::{DateTime, Utc};
use egui::{InnerResponse, Ui};

use crate::model::datetime::{self, WallClock};
use crate::ui::components::component_trait::{Component, ComponentResponse};

/// The row's own copy, shared by every caller so the sentence stays one
/// translation unit instead of drifting per screen. `offset` is how far the
/// user's zone is from UTC, e.g. `UTC+02:00`.
fn schedule_label(offset: &str) -> String {
    format!("Cast on (your local time, {offset}):")
}

const DATE_HINT_TEXT: &str = "YYYY-MM-DD";
const HOUR_PREFIX: &str = "Hour: ";
const MINUTE_PREFIX: &str = "Minute: ";
const DATE_FIELD_WIDTH: f32 = 100.0;

/// Response from [`LocalScheduleInput::show`].
#[derive(Clone)]
pub struct LocalScheduleInputResponse {
    changed: bool,
    changed_value: Option<u64>,
    value: Option<u64>,
}

impl ComponentResponse for LocalScheduleInputResponse {
    /// Unix milliseconds of the chosen instant.
    type DomainType = u64;

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.changed_value
    }

    fn is_valid(&self) -> bool {
        self.value.is_some()
    }

    /// Always `None`: an unparseable entry is reported by `is_valid`, and the
    /// caller owns the sentence, because only the caller knows which instants
    /// its own workflow accepts.
    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// A labelled ISO date field plus hour and minute spinners, read as the user's
/// local time. The label says so and names the zone's offset from UTC.
///
/// The widget parses only — it reports whether the entry is a real instant
/// and says nothing about whether that instant is acceptable. Range rules
/// ("must be in the future", "must precede the contest deadline") stay with the
/// caller, which is the layer that knows them.
///
/// Deliberately not `egui_extras::DatePickerButton`: that widget is behind an
/// `egui_extras` feature this crate does not enable, and it replaces typed ISO
/// entry with a calendar popup — a different input model than the callers have.
///
/// # Usage
///
/// ```rust,ignore
/// let response = self.schedule.show(ui).inner;
/// if response.has_changed() {
///     response.update(&mut self.scheduled_at);
/// }
/// ```
#[derive(Clone)]
pub struct LocalScheduleInput {
    label: Option<String>,
    date: String,
    hour: u32,
    minute: u32,
}

impl LocalScheduleInput {
    /// An empty input at midnight.
    pub fn new() -> Self {
        Self {
            label: None,
            date: String::new(),
            hour: 0,
            minute: 0,
        }
    }

    /// Replace the leading label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Pre-fill the fields with the local time of an instant. Seconds are
    /// dropped: the widget's resolution is one minute.
    pub fn with_time(mut self, time: DateTime<Utc>) -> Self {
        let WallClock { date, hour, minute } = datetime::local_wall_clock(time);
        self.date = date;
        self.hour = hour;
        self.minute = minute;
        self
    }

    fn instant(&self) -> Option<DateTime<Utc>> {
        datetime::instant_from_local_wall_clock(&self.date, self.hour, self.minute)
    }

    /// The default label names the offset at the typed time, because daylight
    /// saving time moves it, and the present offset while no time is typed.
    fn label(&self) -> String {
        match &self.label {
            Some(label) => label.clone(),
            None => schedule_label(&datetime::local_utc_offset(
                self.instant().unwrap_or_else(Utc::now),
            )),
        }
    }
}

impl Default for LocalScheduleInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for LocalScheduleInput {
    type DomainType = u64;
    type Response = LocalScheduleInputResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let InnerResponse { inner, response } = ui.horizontal(|ui| {
            ui.label(self.label());
            let mut changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.date)
                        .desired_width(DATE_FIELD_WIDTH)
                        .hint_text(DATE_HINT_TEXT),
                )
                .changed();
            changed |= ui
                .add(
                    egui::DragValue::new(&mut self.hour)
                        .prefix(HOUR_PREFIX)
                        .range(0..=23),
                )
                .changed();
            changed |= ui
                .add(
                    egui::DragValue::new(&mut self.minute)
                        .prefix(MINUTE_PREFIX)
                        .range(0..=59),
                )
                .changed();
            changed
        });
        let value = self.current_value();
        InnerResponse::new(
            LocalScheduleInputResponse {
                changed: inner,
                changed_value: inner.then_some(value).flatten(),
                value,
            },
            response,
        )
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        self.instant().and_then(datetime::unix_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
            .expect("valid test instant")
            .and_utc()
    }

    #[test]
    fn local_schedule_input_round_trips_the_instant_it_was_given() {
        let requested = at(2031, 2, 3, 4, 5);

        let input = LocalScheduleInput::new().with_time(requested);

        assert_eq!(
            input.current_value(),
            Some(requested.timestamp_millis() as u64)
        );
    }

    #[test]
    fn local_schedule_input_reports_an_unparseable_entry_as_having_no_value() {
        assert_eq!(LocalScheduleInput::new().current_value(), None);
        assert_eq!(
            LocalScheduleInput::new()
                .with_time(at(2031, 2, 3, 4, 5))
                .with_label("Other label:")
                .current_value(),
            Some(at(2031, 2, 3, 4, 5).timestamp_millis() as u64),
        );
    }

    #[test]
    fn local_schedule_input_drops_seconds_so_the_minute_is_what_was_chosen() {
        let with_seconds = at(2031, 2, 3, 4, 5) + chrono::Duration::seconds(37);

        let input = LocalScheduleInput::new().with_time(with_seconds);

        assert_eq!(
            input.current_value(),
            Some(at(2031, 2, 3, 4, 5).timestamp_millis() as u64)
        );
    }

    #[test]
    fn local_schedule_input_label_names_local_time_and_its_utc_offset() {
        use egui_kittest::kittest::Queryable;
        let time = at(2031, 2, 3, 4, 5);
        let mut input = LocalScheduleInput::new().with_time(time);
        let harness = egui_kittest::Harness::new_ui(move |ui| {
            input.show(ui);
        });

        let offset = datetime::local_utc_offset(time);
        assert!(
            offset.starts_with("UTC+") || offset.starts_with("UTC-"),
            "{offset}"
        );
        assert!(
            harness
                .query_by_label(&format!("Cast on (your local time, {offset}):"))
                .is_some()
        );
    }

    #[test]
    fn local_schedule_input_names_the_present_offset_while_no_time_is_typed() {
        use egui_kittest::kittest::Queryable;
        let mut input = LocalScheduleInput::new();
        let harness = egui_kittest::Harness::new_ui(move |ui| {
            input.show(ui);
        });

        assert!(
            harness
                .query_by_label_contains("Cast on (your local time, UTC")
                .is_some()
        );
    }
}
