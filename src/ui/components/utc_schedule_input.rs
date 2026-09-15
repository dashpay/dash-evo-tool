//! Absolute UTC date-and-time input for scheduling deferred work.

use chrono::{DateTime, Timelike, Utc};
use egui::{InnerResponse, Ui};

use crate::model::dpns_vote_schedule::parse_utc_schedule;
use crate::ui::components::component_trait::{Component, ComponentResponse};

/// The row's own copy, shared by every caller so the sentence stays one
/// translation unit instead of drifting per screen.
const UTC_SCHEDULE_LABEL: &str = "Cast on (UTC):";
const DATE_HINT_TEXT: &str = "YYYY-MM-DD";
const HOUR_PREFIX: &str = "Hour: ";
const MINUTE_PREFIX: &str = "Minute: ";
const DATE_FIELD_WIDTH: f32 = 100.0;

/// Response from [`UtcScheduleInput::show`].
#[derive(Clone)]
pub struct UtcScheduleInputResponse {
    changed: bool,
    changed_value: Option<u64>,
    value: Option<u64>,
}

impl ComponentResponse for UtcScheduleInputResponse {
    /// Unix milliseconds of the chosen UTC instant.
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

/// A labelled ISO date field plus hour and minute spinners, read as UTC.
///
/// The widget parses only — it reports whether the entry is a real UTC instant
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
pub struct UtcScheduleInput {
    label: String,
    date: String,
    hour: u32,
    minute: u32,
}

impl UtcScheduleInput {
    /// An empty input at midnight UTC.
    pub fn new() -> Self {
        Self {
            label: UTC_SCHEDULE_LABEL.to_owned(),
            date: String::new(),
            hour: 0,
            minute: 0,
        }
    }

    /// Replace the leading label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Pre-fill the fields from an instant. Seconds are dropped: the widget's
    /// resolution is one minute.
    pub fn with_time(mut self, time: DateTime<Utc>) -> Self {
        self.date = time.format("%Y-%m-%d").to_string();
        self.hour = time.hour();
        self.minute = time.minute();
        self
    }
}

impl Default for UtcScheduleInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for UtcScheduleInput {
    type DomainType = u64;
    type Response = UtcScheduleInputResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let InnerResponse { inner, response } = ui.horizontal(|ui| {
            ui.label(&self.label);
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
            UtcScheduleInputResponse {
                changed: inner,
                changed_value: inner.then_some(value).flatten(),
                value,
            },
            response,
        )
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        parse_utc_schedule(&self.date, self.hour, self.minute)
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
    fn utc_schedule_input_round_trips_the_instant_it_was_given() {
        let requested = at(2031, 2, 3, 4, 5);

        let input = UtcScheduleInput::new().with_time(requested);

        assert_eq!(
            input.current_value(),
            Some(requested.timestamp_millis() as u64)
        );
    }

    #[test]
    fn utc_schedule_input_reports_an_unparseable_entry_as_having_no_value() {
        assert_eq!(UtcScheduleInput::new().current_value(), None);
        assert_eq!(
            UtcScheduleInput::new()
                .with_time(at(2031, 2, 3, 4, 5))
                .with_label("Other label:")
                .current_value(),
            Some(at(2031, 2, 3, 4, 5).timestamp_millis() as u64),
        );
    }

    #[test]
    fn utc_schedule_input_drops_seconds_so_the_minute_is_what_was_chosen() {
        let with_seconds = at(2031, 2, 3, 4, 5) + chrono::Duration::seconds(37);

        let input = UtcScheduleInput::new().with_time(with_seconds);

        assert_eq!(
            input.current_value(),
            Some(at(2031, 2, 3, 4, 5).timestamp_millis() as u64)
        );
    }
}
