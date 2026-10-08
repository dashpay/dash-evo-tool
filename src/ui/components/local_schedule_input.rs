//! Date-and-time input in the user's local time zone, for scheduling deferred work.

use std::sync::Arc;

use chrono::{DateTime, Local, Timelike, Utc};
use egui::{InnerResponse, Ui};

use crate::model::datetime::{self, Clocks, WallClock};
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
/// A time the clocks show twice, at the end of daylight saving time, is the
/// earlier instant when typed. A pre-filled time keeps the instant it came
/// from until a field is edited.
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
    /// The instant the fields were filled from, until the user edits one. At
    /// the end of daylight saving time the clocks show an hour twice, so the
    /// fields alone cannot say which of the two instants they came from.
    filled_from: Option<DateTime<Utc>>,
    zone: Arc<dyn Clocks>,
}

impl LocalScheduleInput {
    /// An empty input at midnight, on the host's clocks.
    pub fn new() -> Self {
        Self {
            label: None,
            date: String::new(),
            hour: 0,
            minute: 0,
            filled_from: None,
            zone: Arc::new(Local),
        }
    }

    /// Read the clocks of `zone` instead of the host's.
    pub fn with_zone(mut self, zone: impl Clocks + 'static) -> Self {
        self.zone = Arc::new(zone);
        if let Some(time) = self.filled_from {
            self.set_time(time);
        }
        self
    }

    /// Replace the leading label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Pre-fill the fields with the local time of an instant. Seconds are
    /// dropped: the widget's resolution is one minute. Until the user edits a
    /// field the value is that instant, even where the clocks show its local
    /// time twice.
    pub fn with_time(mut self, time: DateTime<Utc>) -> Self {
        self.set_time(time);
        self
    }

    /// [`Self::with_time`] on an input that already exists.
    pub fn set_time(&mut self, time: DateTime<Utc>) {
        let WallClock { date, hour, minute } = self.zone.wall_clock(time);
        self.date = date;
        self.hour = hour;
        self.minute = minute;
        self.filled_from = time.with_second(0).and_then(|time| time.with_nanosecond(0));
    }

    /// The instant shown: the one the fields were filled from, else the
    /// fields read as local time.
    fn instant(&self) -> Option<DateTime<Utc>> {
        self.filled_from.or_else(|| {
            self.zone
                .instant_from_wall_clock(&self.date, self.hour, self.minute)
        })
    }

    /// The default label names the offset at the shown time, because daylight
    /// saving time moves it, and the present offset while no time is shown.
    fn label(&self) -> String {
        match &self.label {
            Some(label) => label.clone(),
            None => schedule_label(
                &self
                    .zone
                    .utc_offset(self.instant().unwrap_or_else(Utc::now)),
            ),
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
        if inner {
            self.filled_from = None;
        }
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
    use crate::model::datetime::test_zone::CentralEurope2031;
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

        let offset = Local.utc_offset(time);
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

    /// Central European clocks show 02:30 twice on 2031-10-26: at 00:30 UTC
    /// (UTC+2) and again at 01:30 UTC (UTC+1).
    fn half_past_two(occurrence: u32) -> DateTime<Utc> {
        at(2031, 10, 26, occurrence - 1, 30)
    }

    fn shown_in_central_europe(time: DateTime<Utc>) -> LocalScheduleInput {
        LocalScheduleInput::new()
            .with_zone(CentralEurope2031)
            .with_time(time)
    }

    #[test]
    fn local_schedule_input_keeps_the_instant_behind_a_repeated_time() {
        for occurrence in [1, 2] {
            let time = half_past_two(occurrence);

            let input = shown_in_central_europe(time);

            assert_eq!(
                (input.date.as_str(), input.hour, input.minute),
                ("2031-10-26", 2, 30)
            );
            assert_eq!(
                input.current_value(),
                Some(time.timestamp_millis() as u64),
                "occurrence {occurrence}"
            );
        }
    }

    #[test]
    fn local_schedule_input_label_names_the_offset_of_the_instant_it_shows() {
        use egui_kittest::kittest::Queryable;
        for (occurrence, offset) in [(1, "UTC+02:00"), (2, "UTC+01:00")] {
            let mut input = shown_in_central_europe(half_past_two(occurrence));
            let harness = egui_kittest::Harness::new_ui(move |ui| {
                input.show(ui);
            });

            assert!(
                harness
                    .query_by_label(&format!("Cast on (your local time, {offset}):"))
                    .is_some(),
                "occurrence {occurrence}"
            );
        }
    }

    /// Typing into a field makes the entry the user's own: a repeated time
    /// they typed is its first occurrence.
    #[test]
    fn local_schedule_input_reads_an_edited_repeated_time_as_its_first_occurrence() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder().build_ui_state(
            |ui, input: &mut LocalScheduleInput| {
                input.show(ui);
            },
            shown_in_central_europe(half_past_two(2)),
        );
        harness.run();

        let date = harness.get_by_role(Role::TextInput);
        date.click();
        date.type_text("x");
        harness.run();
        assert_eq!(harness.state().current_value(), None);
        harness.key_press(egui::Key::Backspace);
        harness.run();

        assert_eq!(harness.state().date, "2031-10-26");
        assert_eq!(
            harness.state().current_value(),
            Some(half_past_two(1).timestamp_millis() as u64)
        );
        assert!(
            harness
                .query_by_label("Cast on (your local time, UTC+02:00):")
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
