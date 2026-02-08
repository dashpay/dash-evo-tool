use std::collections::BTreeMap;

use chrono::{Duration, Utc};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::v0::TokenPerpetualDistributionV0;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::TokenPerpetualDistribution;
use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::v0::TokenPreProgrammedDistributionV0;
use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::TokenPreProgrammedDistribution;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillisInterval;
use dash_sdk::platform::Identifier;

use crate::ui::theme::DashColors;
use crate::ui::tokens::tokens_screen::{TokensScreen, sanitize_i64, sanitize_u64};
use eframe::epaint::Color32;
use egui::{ComboBox, Context, RichText, TextEdit};

/// A lightweight enum for the user's choice of distribution type
#[derive(Debug, Clone, PartialEq)]
pub enum PerpetualDistributionIntervalTypeUI {
    None,
    BlockBased,
    TimeBased,
    EpochBased,
}

/// A lightweight enum for the user's choice of distribution function
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Hash)]
pub enum DistributionFunctionUI {
    FixedAmount,
    StepDecreasingAmount,
    Stepwise,
    Linear,
    Polynomial,
    Exponential,
    Logarithmic,
    InvertedLogarithmic,
}

impl DistributionFunctionUI {
    pub(crate) fn name(&self) -> &str {
        match self {
            DistributionFunctionUI::FixedAmount => "fixed_amount",
            DistributionFunctionUI::StepDecreasingAmount => "step_decreasing_amount",
            DistributionFunctionUI::Stepwise => "stepwise",
            DistributionFunctionUI::Linear => "linear",
            DistributionFunctionUI::Polynomial => "polynomial",
            DistributionFunctionUI::Exponential => "exponential",
            DistributionFunctionUI::Logarithmic => "logarithmic",
            DistributionFunctionUI::InvertedLogarithmic => "inverted_logarithmic",
        }
    }
}

/// A lightweight enum for the user's recipient selection
#[derive(Debug, Clone, PartialEq)]
pub enum TokenDistributionRecipientUI {
    ContractOwner,
    Identity,
    EvonodesByParticipation,
}

#[derive(Default, Clone)]
pub struct DistributionEntry {
    /// Time from token contract registration when the distribution should occur
    pub days: i32,
    pub hours: i32,
    pub minutes: i32,

    /// The base58 identity to receive distribution
    pub identity_str: String,

    /// The distribution amount
    pub amount_str: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntervalTimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Year,
}

impl IntervalTimeUnit {
    /// Returns the equivalent duration in milliseconds
    /// for the given amount of the unit.
    pub fn ms_for_amount(&self, amount: u64) -> TimestampMillisInterval {
        match self {
            IntervalTimeUnit::Second => amount.saturating_mul(1_000),
            IntervalTimeUnit::Minute => amount.saturating_mul(60_000),
            IntervalTimeUnit::Hour => amount.saturating_mul(3_600_000),
            IntervalTimeUnit::Day => amount.saturating_mul(86_400_000),
            IntervalTimeUnit::Week => amount.saturating_mul(7).saturating_mul(86_400_000),
            IntervalTimeUnit::Year => amount.saturating_mul(31_556_952_000),
        }
    }
    pub fn label_for_amount(&self, amount_str: &str) -> &'static str {
        let is_singular = amount_str == "1";
        match (self, is_singular) {
            (IntervalTimeUnit::Second, true) => "second",
            (IntervalTimeUnit::Second, false) => "seconds",
            (IntervalTimeUnit::Minute, true) => "minute",
            (IntervalTimeUnit::Minute, false) => "minutes",
            (IntervalTimeUnit::Hour, true) => "hour",
            (IntervalTimeUnit::Hour, false) => "hours",
            (IntervalTimeUnit::Day, true) => "day",
            (IntervalTimeUnit::Day, false) => "days",
            (IntervalTimeUnit::Week, true) => "week",
            (IntervalTimeUnit::Week, false) => "weeks",
            (IntervalTimeUnit::Year, true) => "year",
            (IntervalTimeUnit::Year, false) => "years",
        }
    }

    pub fn label_for_num_amount(&self, amount: u64) -> &'static str {
        let is_singular = amount == 1;
        match (self, is_singular) {
            (IntervalTimeUnit::Second, true) => "second",
            (IntervalTimeUnit::Second, false) => "seconds",
            (IntervalTimeUnit::Minute, true) => "minute",
            (IntervalTimeUnit::Minute, false) => "minutes",
            (IntervalTimeUnit::Hour, true) => "hour",
            (IntervalTimeUnit::Hour, false) => "hours",
            (IntervalTimeUnit::Day, true) => "day",
            (IntervalTimeUnit::Day, false) => "days",
            (IntervalTimeUnit::Week, true) => "week",
            (IntervalTimeUnit::Week, false) => "weeks",
            (IntervalTimeUnit::Year, true) => "year",
            (IntervalTimeUnit::Year, false) => "years",
        }
    }

    pub fn capitalized_label_for_num_amount(&self, amount: u64) -> &'static str {
        let is_singular = amount == 1;
        match (self, is_singular) {
            (IntervalTimeUnit::Second, true) => "Second",
            (IntervalTimeUnit::Second, false) => "Seconds",
            (IntervalTimeUnit::Minute, true) => "Minute",
            (IntervalTimeUnit::Minute, false) => "Minutes",
            (IntervalTimeUnit::Hour, true) => "Hour",
            (IntervalTimeUnit::Hour, false) => "Hours",
            (IntervalTimeUnit::Day, true) => "Day",
            (IntervalTimeUnit::Day, false) => "Days",
            (IntervalTimeUnit::Week, true) => "Week",
            (IntervalTimeUnit::Week, false) => "Weeks",
            (IntervalTimeUnit::Year, true) => "Year",
            (IntervalTimeUnit::Year, false) => "Years",
        }
    }
}

impl TokensScreen {
    pub(super) fn build_distribution_rules(&mut self) -> Result<TokenDistributionRulesV0, String> {
        // 1) Validate distribution input, parse numeric fields, etc.
        let distribution_function = match self.perpetual_dist_function {
            DistributionFunctionUI::FixedAmount => DistributionFunction::FixedAmount {
                amount: self.fixed_amount_input.parse::<u64>().unwrap_or(0),
            },
            DistributionFunctionUI::StepDecreasingAmount => {
                DistributionFunction::StepDecreasingAmount {
                    step_count: self.step_count_input.parse::<u32>().unwrap_or(0),
                    decrease_per_interval_numerator: self
                        .decrease_per_interval_numerator_input
                        .parse::<u16>()
                        .unwrap_or(0),
                    decrease_per_interval_denominator: self
                        .decrease_per_interval_denominator_input
                        .parse::<u16>()
                        .unwrap_or(0),
                    start_decreasing_offset: if self
                        .step_decreasing_start_period_offset_input
                        .is_empty()
                    {
                        None
                    } else {
                        match self
                            .step_decreasing_start_period_offset_input
                            .parse::<u64>()
                        {
                            Ok(0) => None,
                            Ok(v) => Some(v),
                            Err(_) => {
                                return Err("Invalid start decreasing offset for StepDecreasingAmount distribution. Put 0 for None.".to_string());
                            }
                        }
                    },
                    distribution_start_amount: self
                        .step_decreasing_initial_emission_input
                        .parse::<u64>()
                        .unwrap_or(0),
                    min_value: if self.step_decreasing_min_value_input.is_empty() {
                        None
                    } else {
                        match self.step_decreasing_min_value_input.parse::<u64>() {
                            Ok(0) => None,
                            Ok(v) => Some(v),
                            Err(_) => {
                                return Err(
                                    "Invalid min value for StepDecreasingAmount distribution. Put 0 for None."
                                        .to_string(),
                                );
                            }
                        }
                    },
                    max_interval_count: if self.step_decreasing_max_interval_count_input.is_empty()
                    {
                        None
                    } else {
                        match self.step_decreasing_max_interval_count_input.parse::<u16>() {
                            Ok(0) => None,
                            Ok(v) => Some(v),
                            Err(_) => {
                                return Err(
                                    "Invalid max interval count for StepDecreasingAmount distribution. Put 0 for None."
                                        .to_string(),
                                );
                            }
                        }
                    },
                    trailing_distribution_interval_amount: self
                        .step_decreasing_trailing_distribution_interval_amount_input
                        .parse::<u64>()
                        .unwrap_or(0),
                }
            }
            DistributionFunctionUI::Stepwise => {
                let steps: BTreeMap<u64, TokenAmount> = self
                    .stepwise_steps
                    .iter()
                    .map(|(k, v)| (k.parse::<u64>().unwrap_or(0), v.parse::<u64>().unwrap_or(0)))
                    .collect();
                DistributionFunction::Stepwise(steps)
            }
            DistributionFunctionUI::Linear => DistributionFunction::Linear {
                a: self.linear_int_a_input.parse::<i64>().unwrap_or(0),
                d: self.linear_int_d_input.parse::<u64>().unwrap_or(1),
                start_step: self
                    .linear_int_start_step_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                starting_amount: self
                    .linear_int_starting_amount_input
                    .parse::<u64>()
                    .unwrap_or(0),
                min_value: self
                    .linear_int_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .linear_int_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::Polynomial => DistributionFunction::Polynomial {
                a: self.poly_int_a_input.parse::<i64>().unwrap_or(0),
                m: self.poly_int_m_input.parse::<i64>().unwrap_or(0),
                n: self.poly_int_n_input.parse::<u64>().unwrap_or(0),
                d: self.poly_int_d_input.parse::<u64>().unwrap_or(0),
                start_moment: self
                    .poly_int_s_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                o: self.poly_int_o_input.parse::<i64>().unwrap_or(0),
                b: self.poly_int_b_input.parse::<u64>().unwrap_or(0),
                min_value: self
                    .poly_int_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .poly_int_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::Exponential => DistributionFunction::Exponential {
                a: self.exp_a_input.parse::<u64>().unwrap_or(0),
                d: self.exp_d_input.parse::<u64>().unwrap_or(0),
                m: self.exp_m_input.parse::<i64>().unwrap_or(0),
                n: self.exp_n_input.parse::<u64>().unwrap_or(0),
                o: self.exp_o_input.parse::<i64>().unwrap_or(0),
                start_moment: self.exp_s_input.parse::<u64>().map(Some).unwrap_or(None),
                b: self.exp_b_input.parse::<u64>().unwrap_or(0),
                min_value: self
                    .exp_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .exp_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::Logarithmic => DistributionFunction::Logarithmic {
                a: self.log_a_input.parse::<i64>().unwrap_or(0),
                d: self.log_d_input.parse::<u64>().unwrap_or(0),
                m: self.log_m_input.parse::<u64>().unwrap_or(0),
                n: self.log_n_input.parse::<u64>().unwrap_or(0),
                start_moment: self.log_s_input.parse::<u64>().map(Some).unwrap_or(None),
                o: self.log_o_input.parse::<i64>().unwrap_or(0),
                b: self.log_b_input.parse::<u64>().unwrap_or(0),
                min_value: self
                    .log_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .log_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::InvertedLogarithmic => {
                DistributionFunction::InvertedLogarithmic {
                    a: self.inv_log_a_input.parse::<i64>().unwrap_or(0),
                    d: self.inv_log_d_input.parse::<u64>().unwrap_or(0),
                    m: self.inv_log_m_input.parse::<u64>().unwrap_or(0),
                    n: self.inv_log_n_input.parse::<u64>().unwrap_or(0),
                    start_moment: self
                        .inv_log_s_input
                        .parse::<u64>()
                        .map(Some)
                        .unwrap_or(None),
                    o: self.inv_log_o_input.parse::<i64>().unwrap_or(0),
                    b: self.inv_log_b_input.parse::<u64>().unwrap_or(0),
                    min_value: self
                        .inv_log_min_value_input
                        .parse::<u64>()
                        .map(Some)
                        .unwrap_or(None),
                    max_value: self
                        .inv_log_max_value_input
                        .parse::<u64>()
                        .map(Some)
                        .unwrap_or(None),
                }
            }
        };
        let maybe_perpetual_distribution = if self.enable_perpetual_distribution {
            // Construct the `TokenPerpetualDistributionV0` from your selected type + function
            let dist_type =
                match self.perpetual_dist_type {
                    PerpetualDistributionIntervalTypeUI::BlockBased => {
                        // parse interval, parse emission
                        // parse distribution function
                        RewardDistributionType::BlockBasedDistribution {
                            interval: self.perpetual_dist_interval_input.parse::<u64>().map_err(
                                |_| "Distribution interval not a valid number".to_string(),
                            )?,
                            function: distribution_function,
                        }
                    }
                    PerpetualDistributionIntervalTypeUI::EpochBased => {
                        RewardDistributionType::EpochBasedDistribution {
                            interval: self.perpetual_dist_interval_input.parse::<u16>().map_err(
                                |_| "Distribution interval not a valid number".to_string(),
                            )?,
                            function: distribution_function,
                        }
                    }
                    PerpetualDistributionIntervalTypeUI::TimeBased => {
                        RewardDistributionType::TimeBasedDistribution {
                            interval: self.perpetual_dist_interval_unit.ms_for_amount(
                                self.perpetual_dist_interval_input.parse::<u64>().map_err(
                                    |_| "Distribution interval not a valid number".to_string(),
                                )?,
                            ),
                            function: distribution_function,
                        }
                    }
                    _ => RewardDistributionType::BlockBasedDistribution {
                        interval: 0,
                        function: DistributionFunction::FixedAmount { amount: 0 },
                    },
                };

            let recipient = match self.perpetual_dist_recipient {
                TokenDistributionRecipientUI::ContractOwner => {
                    TokenDistributionRecipient::ContractOwner
                }
                TokenDistributionRecipientUI::Identity => {
                    if let Some(id) = self.perpetual_dist_recipient_identity_input.as_ref() {
                        let id_res = Identifier::from_string(id, Encoding::Base58);
                        TokenDistributionRecipient::Identity(id_res.unwrap_or_default())
                    } else {
                        self.token_creator_error_message = Some(
                            "Invalid base58 identifier for perpetual distribution recipient"
                                .to_string(),
                        );
                        return Err(
                            "Invalid base58 identifier for perpetual distribution recipient"
                                .to_string(),
                        );
                    }
                }
                TokenDistributionRecipientUI::EvonodesByParticipation => {
                    TokenDistributionRecipient::EvonodesByParticipation
                }
            };

            Some(TokenPerpetualDistribution::V0(
                TokenPerpetualDistributionV0 {
                    distribution_type: dist_type,
                    distribution_recipient: recipient,
                },
            ))
        } else {
            None
        };

        // 2) Build the distribution rules structure
        let dist_rules_v0 = TokenDistributionRulesV0 {
            perpetual_distribution: maybe_perpetual_distribution,
            perpetual_distribution_rules: self
                .perpetual_distribution_rules
                .extract_change_control_rules("Perpetual Distribution")?,
            pre_programmed_distribution: if self.enable_pre_programmed_distribution {
                let distributions: BTreeMap<u64, BTreeMap<Identifier, u64>> =
                    match self.parse_pre_programmed_distributions() {
                        Ok(distributions) => distributions,
                        Err(err) => {
                            self.token_creator_error_message = Some(err.clone());
                            return Err(err.to_string());
                        }
                    };

                Some(TokenPreProgrammedDistribution::V0(
                    TokenPreProgrammedDistributionV0 { distributions },
                ))
            } else {
                None
            },
            new_tokens_destination_identity: if self
                .new_tokens_destination_identity_should_default_to_contract_owner
            {
                Some(
                    self.selected_identity
                        .as_ref()
                        .ok_or("No selected identity".to_string())?
                        .identity
                        .id(),
                )
            } else if self.new_tokens_destination_other_identity_enabled {
                Some(
                    Identifier::from_string(
                        &self.new_tokens_destination_other_identity,
                        Encoding::Base58,
                    )
                    .map_err(|e| e.to_string())?,
                )
            } else {
                None
            },
            new_tokens_destination_identity_rules: self
                .new_tokens_destination_identity_rules
                .extract_change_control_rules("New Tokens Destination Identity")?,
            minting_allow_choosing_destination: self.minting_allow_choosing_destination,
            minting_allow_choosing_destination_rules: self
                .minting_allow_choosing_destination_rules
                .extract_change_control_rules("Minting Allow Choosing Destination")?,
            change_direct_purchase_pricing_rules: self
                .change_direct_purchase_pricing_rules
                .extract_change_control_rules("Change Direct Purchase Pricing")?,
        };

        Ok(dist_rules_v0)
    }

    /// Attempts to parse the `pre_programmed_distributions` into a BTreeMap.
    /// Returns an error string if any row fails.
    /// Now supports multiple identities per timestamp.
    pub fn parse_pre_programmed_distributions(
        &mut self,
    ) -> Result<BTreeMap<u64, BTreeMap<Identifier, u64>>, String> {
        let mut map: BTreeMap<u64, BTreeMap<Identifier, u64>> = BTreeMap::new();

        let now = Utc::now();

        for (i, entry) in self.pre_programmed_distributions.iter().enumerate() {
            // Convert days/hours/minutes into a timestamp.
            let offset = Duration::days(entry.days as i64)
                + Duration::hours(entry.hours as i64)
                + Duration::minutes(entry.minutes as i64);
            let timestamp = (now + offset).timestamp_millis() as u64;

            // Parse identity
            let id =
                Identifier::from_string(&entry.identity_str, Encoding::Base58).map_err(|_| {
                    format!(
                        "Row {}: invalid base58 identity '{}'",
                        i + 1,
                        entry.identity_str
                    )
                })?;

            // Parse amount
            let amount = entry.amount_str.parse::<u64>().map_err(|_| {
                format!(
                    "Row {}: invalid distribution amount (expected u64). Got '{}'",
                    i + 1,
                    entry.amount_str
                )
            })?;

            // Insert into the map, supporting multiple identities per timestamp
            map.entry(timestamp).or_default().insert(id, amount);
        }
        Ok(map)
    }

    pub(super) fn render_distributions(&mut self, context: &Context, ui: &mut egui::Ui) {
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            // +/- button
            let button_text = if self.token_creator_distribution_expanded {
                "−"
            } else {
                "+"
            };
            let button_response = ui.add(
                egui::Button::new(
                    RichText::new(button_text)
                        .size(20.0)
                        .color(DashColors::DASH_BLUE),
                )
                .fill(Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE),
            );
            if button_response.clicked() {
                self.token_creator_distribution_expanded =
                    !self.token_creator_distribution_expanded;
            }
            ui.label("Distribution");
        });

        if self.token_creator_distribution_expanded {
            ui.add_space(3.0);

            ui.indent("distribution_section", |ui| {
                // PERPETUAL DISTRIBUTION SETTINGS
            if ui
                .checkbox(
                    &mut self.enable_perpetual_distribution,
                    "Enable Perpetual Distribution",
                )
                .clicked()
            {
                self.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::TimeBased;
            };
            if self.enable_perpetual_distribution {
                ui.add_space(5.0);

                // 2) Select the distribution type
                ui.horizontal(|ui| {
                    ui.label("     Type:");
                    ComboBox::from_id_salt("perpetual_dist_type_selector")
                        .selected_text(format!("{:?}", self.perpetual_dist_type))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.perpetual_dist_type,
                                PerpetualDistributionIntervalTypeUI::TimeBased,
                                "Time-Based",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_type,
                                PerpetualDistributionIntervalTypeUI::EpochBased,
                                "Epoch-Based",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_type,
                                PerpetualDistributionIntervalTypeUI::BlockBased,
                                "Block-Based",
                            );
                        });
                });

                // If user picked a real distribution type:
                match self.perpetual_dist_type {
                    PerpetualDistributionIntervalTypeUI::TimeBased => {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label("        - Distributes every ");

                            // Restrict input to digits only
                            let response = ui.add(TextEdit::singleline(
                                &mut self.perpetual_dist_interval_input,
                            ));

                            // Optionally filter out non-digit input
                            if response.changed() {
                                self.perpetual_dist_interval_input
                                    .retain(|c| c.is_ascii_digit());
                            }

                            // Dropdown for selecting unit
                            ComboBox::from_id_salt("interval_unit_selector")
                                .selected_text(
                                    self.perpetual_dist_interval_unit
                                        .label_for_amount(&self.perpetual_dist_interval_input),
                                )
                                .show_ui(ui, |ui| {
                                    for unit in [
                                        IntervalTimeUnit::Second,
                                        IntervalTimeUnit::Minute,
                                        IntervalTimeUnit::Hour,
                                        IntervalTimeUnit::Day,
                                        IntervalTimeUnit::Week,
                                        IntervalTimeUnit::Year,
                                    ] {
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_interval_unit,
                                            unit.clone(),
                                            unit.label_for_amount(
                                                &self.perpetual_dist_interval_input,
                                            ),
                                        );
                                    }
                                });
                        });
                    }
                    PerpetualDistributionIntervalTypeUI::EpochBased => {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label("        - Distributes every (Epoch Interval):");
                            ui.text_edit_singleline(&mut self.perpetual_dist_interval_input);
                        });
                    }
                    PerpetualDistributionIntervalTypeUI::BlockBased => {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label("        - Distributes every (Block Interval):");
                            ui.text_edit_singleline(&mut self.perpetual_dist_interval_input);
                        });
                    }
                    PerpetualDistributionIntervalTypeUI::None => {
                        // Do nothing
                    }
                }

                ui.add_space(10.0);

                // 3) Select the distribution function
                ui.horizontal(|ui| {
                    ui.label("     Function:");
                    ComboBox::from_id_salt("perpetual_dist_function_selector")
                        .selected_text(format!("{:?}", self.perpetual_dist_function))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::FixedAmount,
                                "FixedAmount",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::StepDecreasingAmount,
                                "StepDecreasing",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::Stepwise,
                                "Stepwise",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::Linear,
                                "Linear",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::Polynomial,
                                "Polynomial",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::Exponential,
                                "Exponential",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::Logarithmic,
                                "Logarithmic",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_function,
                                DistributionFunctionUI::InvertedLogarithmic,
                                "InvertedLogarithmic",
                            );
                            // DistributionFunctionUI::Random is not supported
                        });

                    let response = crate::ui::helpers::info_icon_button(ui, "Info about distribution types");

                    // Check if the label was clicked
                    if response.clicked() {
                        self.show_pop_up_info = Some(r#"
# FixedAmount

Emits a constant (fixed) number of tokens for every period.

### Formula
For any period `x`, the emitted tokens are:

`f(x) = n`

### Use Case
- When a predictable, unchanging reward is desired.
- Simplicity and stable emissions.

### Example
- If `n = 5` tokens per block, then after 3 blocks the total emission is 15 tokens.

---

# StepDecreasingAmount

Emits a random number of tokens within a specified range.

### Description
- This function selects a **random** token emission amount between `min` and `max`.
- The value is drawn **uniformly** between the bounds.
- The randomness uses a Pseudo Random Function (PRF) from x.

### Formula
For any period `x`, the emitted tokens follow:

`f(x) ∈ [min, max]`

### Parameters
- `min`: The **minimum** possible number of tokens emitted.
- `max`: The **maximum** possible number of tokens emitted.

### Use Cases
- **Stochastic Rewards**: Introduces randomness into rewards to incentivize unpredictability.
- **Lottery-Based Systems**: Used for randomized emissions, such as block rewards with probabilistic payouts.

### Example
Suppose a system emits **between 10 and 100 tokens per period**.

`Random { min: 10, max: 100 }`

| Period (x) | Emitted Tokens (Random) |
|------------|------------------------|
| 1          | 27                     |
| 2          | 94                     |
| 3          | 63                     |
| 4          | 12                     |

- Each period, the function emits a **random number of tokens** between `min = 10` and `max = 100`.
- Over time, the **average reward trends toward the midpoint** `(min + max) / 2`.

### Constraints
- **`min` must be ≤ `max`**, otherwise the function is invalid.
- If `min == max`, this behaves like a `FixedAmount` function with a constant emission.

---

### LinearInteger

A linear function using integer precision.

- **Formula:** f(x) = a * x + b
- **Description:**
    - a > 0 -> tokens increase over time
    - a < 0 -> tokens decrease over time
    - b is the initial value
- **Use Case:** Incentivize early or match ecosystem growth
- **Example:** f(x) = 10x + 50

---

### LinearFloat

A linear function with fractional (floating-point) rates.

- **Formula:** f(x) = a * x + b
- **Description:** Similar to LinearInteger, but with fractional slope
- **Use Case:** Gradual fractional increases/decreases over time
- **Example:** f(x) = 0.5x + 50

---

### PolynomialInteger

A polynomial function (e.g. quadratic, cubic) using integer precision.

- **Formula:** f(x) = a * x^n + b
- **Description:** Flexible curves (growth/decay) beyond simple linear.
- **Use Case:** Diminishing or accelerating returns as time progresses
- **Example:** f(x) = 2x^2 + 20

---

### PolynomialFloat

A polynomial function supporting fractional exponents or coefficients.

- **Formula:** f(x) = a * x^n + b
- **Description:** Similar to PolynomialInteger, but with floats
- **Example:** f(x) = 0.5x^3 + 20

---

### Exponential

Exponential growth or decay of tokens.

- **Formula:** f(x) = a * e^(b * x) + c
- **Description:**
    - b > 0 -> rapid growth
    - b < 0 -> rapid decay
- **Use Case:** Early contributor boosts or quick emission tapering
- **Example:** f(x) = 100 * e^(-0.693 * x) + 5

---

### Logarithmic

Logarithmic growth of token emissions.

- **Formula:** f(x) = a * log_b(x) + c
- **Description:** Growth slows as x increases.
- **Use Case:** Sustainable long-term emission tapering
- **Example:** f(x) = 20 * log_2(x) + 5

---

### Stepwise

Emits tokens in fixed amounts for specific intervals.

- **Description:** Emissions remain constant within each step.
- **Use Case:** Adjust rewards at specific milestones
- **Example:** 100 tokens per block for first 1000 blocks, then 50 tokens thereafter.
"#
                            .to_string())};
                });

                ui.add_space(2.0);

                if let Some(texture) = self.function_textures.get(&self.perpetual_dist_function) {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(50.0); // Shift image right
                        ui.image(texture);
                    });
                    ui.add_space(10.0);
                } else if let Some(image) = self.function_images.get(&self.perpetual_dist_function)
                {
                    let texture = context.load_texture(
                        self.perpetual_dist_function.name(),
                        image.clone(),
                        Default::default(),
                    );
                    self.function_textures
                        .insert(self.perpetual_dist_function.clone(), texture.clone());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(50.0); // Shift image right
                        ui.image(&texture);
                    });
                    ui.add_space(10.0);
                }

                // Based on the user's chosen function, display relevant fields:
                match self.perpetual_dist_function {
                    DistributionFunctionUI::FixedAmount => {
                        ui.horizontal(|ui| {
                            ui.label("        - Fixed Amount per Interval:");
                            ui.text_edit_singleline(&mut self.fixed_amount_input);
                        });
                    }

                    DistributionFunctionUI::StepDecreasingAmount => {
                        ui.horizontal(|ui| {
                            ui.label("        - Step Count:");
                            let response = ui.add(TextEdit::singleline(&mut self.step_count_input));
                            if response.changed() {
                                sanitize_u64(&mut self.step_count_input);
                            }
                            if !self.step_count_input.is_empty()
                                && let Ok((perpetual_dist_interval_input, step_count_input)) = self
                                    .perpetual_dist_interval_input
                                    .parse::<u64>()
                                    .and_then(|perpetual_dist_interval_input| self.step_count_input.parse::<u64>().map(|step_count_input| (perpetual_dist_interval_input, step_count_input))) {
                                    let text = match self.perpetual_dist_type {
                                        PerpetualDistributionIntervalTypeUI::None => "".to_string(),
                                        PerpetualDistributionIntervalTypeUI::BlockBased => {
                                            let amount =
                                                perpetual_dist_interval_input * step_count_input;
                                            if amount == 1 {
                                                "Every Block".to_string()
                                            } else {
                                                format!("Every {} Blocks", amount)
                                            }
                                        }
                                        PerpetualDistributionIntervalTypeUI::TimeBased => {
                                            let amount =
                                                perpetual_dist_interval_input * step_count_input;
                                            format!(
                                                "Every {} {}",
                                                amount,
                                                self.perpetual_dist_interval_unit
                                                    .capitalized_label_for_num_amount(amount)
                                            )
                                        }
                                        PerpetualDistributionIntervalTypeUI::EpochBased => {
                                            let amount =
                                                perpetual_dist_interval_input * step_count_input;
                                            if amount == 1 {
                                                "Every Epoch Change".to_string()
                                            } else {
                                                format!("Every {} Epochs", amount)
                                            }
                                        }
                                    };

                                    ui.label(RichText::new(text).color(Color32::GRAY));
                                }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Decrease per Interval Numerator (n < 65,536):");
                            let response = ui.add(TextEdit::singleline(
                                &mut self.decrease_per_interval_numerator_input,
                            ));
                            if response.changed() {
                                sanitize_u64(&mut self.decrease_per_interval_numerator_input);
                                self.decrease_per_interval_numerator_input.truncate(5);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Decrease per Interval Denominator (d < 65,536):");
                            let response = ui.add(TextEdit::singleline(
                                &mut self.decrease_per_interval_denominator_input,
                            ));
                            if response.changed() {
                                sanitize_u64(&mut self.decrease_per_interval_denominator_input);
                                self.decrease_per_interval_denominator_input.truncate(5);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Start Period Offset (i64, optional):");
                            let response = ui.add(
                                TextEdit::singleline(
                                    &mut self.step_decreasing_start_period_offset_input,
                                )
                                .hint_text("None"),
                            );
                            if response.changed() {
                                sanitize_i64(&mut self.step_decreasing_start_period_offset_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Initial Token Emission Amount:");
                            let response = ui.add(TextEdit::singleline(
                                &mut self.step_decreasing_initial_emission_input,
                            ));
                            if response.changed() {
                                sanitize_u64(&mut self.step_decreasing_initial_emission_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Minimum Emission Value (optional):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.step_decreasing_min_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() {
                                sanitize_u64(&mut self.step_decreasing_min_value_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Maximum Interval Count (optional):");
                            let response = ui.add(
                                TextEdit::singleline(
                                    &mut self.step_decreasing_max_interval_count_input,
                                )
                                .hint_text("None"),
                            );
                            if response.changed() {
                                sanitize_u64(&mut self.step_decreasing_max_interval_count_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Trailing Distribution Interval Token Amount:");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.step_decreasing_trailing_distribution_interval_amount_input),
                            );
                            if response.changed() {
                                sanitize_u64(&mut self.step_decreasing_trailing_distribution_interval_amount_input);
                            }
                        });
                    }

                    DistributionFunctionUI::Stepwise => {
                        // Example: multiple steps (u64 block => some token amount).
                        // Each element in `stepwise_steps` is (String, String) = (block, amount).
                        // You can show them in a loop and let users edit each pair.
                        let mut i = 0;
                        while i < self.stepwise_steps.len() {
                            let (mut steps_str, mut amount_str) = self.stepwise_steps[i].clone();

                            ui.horizontal(|ui| {
                                ui.label(format!("        - Step #{}:", i));
                                ui.label("Start Step:");
                                let response = ui.add(TextEdit::singleline(&mut steps_str).desired_width(50.0));
                                if response.changed() {
                                    sanitize_u64(&mut steps_str);
                                }

                                ui.label("Amount:");
                                let response = ui.add(TextEdit::singleline(&mut amount_str).desired_width(50.0));
                                if response.changed() {
                                    sanitize_u64(&mut amount_str);
                                }

                                if let Ok((perpetual_dist_interval_input, step_position)) = self.perpetual_dist_interval_input.parse::<u64>().and_then(|perpetual_dist_interval_input| steps_str.parse::<u64>().map(|step_count_input| (perpetual_dist_interval_input, step_count_input)))
                                    && let Ok(amount) = amount_str.parse::<u64>() {
                                        let every_text = match self.perpetual_dist_type {
                                            PerpetualDistributionIntervalTypeUI::None => "".to_string(),
                                            PerpetualDistributionIntervalTypeUI::BlockBased => {
                                                if perpetual_dist_interval_input == 1 {
                                                    "every block".to_string()
                                                } else {
                                                    format!("every {} blocks", perpetual_dist_interval_input)
                                                }
                                            }
                                            PerpetualDistributionIntervalTypeUI::TimeBased => {
                                                format!("every {} {}", perpetual_dist_interval_input, self.perpetual_dist_interval_unit.label_for_num_amount(perpetual_dist_interval_input))
                                            }
                                            PerpetualDistributionIntervalTypeUI::EpochBased => {
                                                if perpetual_dist_interval_input == 1 {
                                                    "every epoch change".to_string()
                                                } else {
                                                    format!("every {} epochs", perpetual_dist_interval_input)
                                                }
                                            }
                                        };

                                        let text = match self.perpetual_dist_type {
                                            PerpetualDistributionIntervalTypeUI::None => "".to_string(),
                                            PerpetualDistributionIntervalTypeUI::BlockBased => {
                                                let block = step_position * perpetual_dist_interval_input;
                                                if block == 0 {
                                                    if amount == 0 {
                                                        "At start don't distribute tokens".to_string()
                                                    } else {
                                                        format!("At start distribute {} tokens {}", amount, every_text)
                                                    }
                                                } else if amount == 0 {
                                                    format!("After block {} stop distributing tokens", block)
                                                } else {
                                                    format!("After block {} distribute {} tokens {}", block, amount, every_text)
                                                }
                                            }
                                            PerpetualDistributionIntervalTypeUI::TimeBased => {
                                                let time = step_position * perpetual_dist_interval_input;
                                                if time == 0 {
                                                    if amount == 0 {
                                                        "At start don't distribute tokens".to_string()
                                                    } else {
                                                        format!("At start distribute {} tokens {}", amount, every_text)
                                                    }
                                                } else if amount == 0 {
                                                    format!("{} {} after the contract is registered stop distributing tokens", time, self.perpetual_dist_interval_unit.label_for_num_amount(perpetual_dist_interval_input))
                                                } else {
                                                    format!("{} {} after the contract is registered distribute {} tokens {}", time, self.perpetual_dist_interval_unit.label_for_num_amount(perpetual_dist_interval_input), amount, every_text)
                                                }
                                            }
                                            PerpetualDistributionIntervalTypeUI::EpochBased => {
                                                let epoch = step_position * perpetual_dist_interval_input;
                                                if epoch == 0 {
                                                    if amount == 0 {
                                                        "At start don't distribute tokens".to_string()
                                                    } else {
                                                        format!("At start distribute {} tokens {}", amount, every_text)
                                                    }
                                                } else if amount == 0 {
                                                    format!("After epoch {} stop distributing tokens", epoch)
                                                } else {
                                                    format!("After epoch {} distribute {} tokens {}", epoch, amount, every_text)
                                                }
                                            }
                                        };

                                        ui.label(RichText::new(text).color(Color32::GRAY));
                                    }

                                // If remove is clicked, remove the step at index i
                                // and *do not* increment i, because the next element
                                // now "shifts" into this index.
                                if ui.button("Remove").clicked() {
                                    self.stepwise_steps.remove(i);
                                } else {
                                    // Otherwise, update the vector with any edits and move to the next step
                                    self.stepwise_steps[i] = (steps_str, amount_str);
                                    i += 1;
                                }
                            });
                        }

                        // A button to add new steps
                        ui.horizontal(|ui| {
                            ui.label("     ");
                            if ui.button("Add Step").clicked() {
                                self.stepwise_steps.push(("0".to_owned(), "0".to_owned()));
                            }
                        });
                    }

                    DistributionFunctionUI::Linear => {
                        ui.horizontal(|ui| {
                            ui.label("        - Slope Numerator (a, { -255 ≤ a ≤ 256 }):");
                            let response =
                                ui.add(TextEdit::singleline(&mut self.linear_int_a_input));
                            if response.changed() {
                                sanitize_i64(&mut self.linear_int_a_input);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Slope Divisor (d, u64):");
                            let response =
                                ui.add(TextEdit::singleline(&mut self.linear_int_d_input));
                            if response.changed() {
                                sanitize_u64(&mut self.linear_int_d_input);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Start Step (s, i64, optional):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.linear_int_start_step_input)
                                    .hint_text("None"),
                            );
                            if response.changed() {
                                sanitize_i64(&mut self.linear_int_start_step_input);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Starting Amount (b, i64):");
                            let response = ui.add(TextEdit::singleline(
                                &mut self.linear_int_starting_amount_input,
                            ));
                            if response.changed() {
                                sanitize_i64(&mut self.linear_int_starting_amount_input);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Minimum Emission Value (optional):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.linear_int_min_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() {
                                sanitize_u64(&mut self.linear_int_min_value_input);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Maximum Emission Value (optional):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.linear_int_max_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() {
                                sanitize_u64(&mut self.linear_int_max_value_input);
                            }
                        });
                    }

                    DistributionFunctionUI::Polynomial => {
                        ui.horizontal(|ui| {
                            ui.label("        - Scaling Factor (a, { -255 ≤ a ≤ 256 }):");
                            ui.text_edit_singleline(&mut self.poly_int_a_input);
                            sanitize_i64(&mut self.poly_int_a_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Exponent Numerator (m, { -8 ≤ m ≤ 8 }):");
                            ui.text_edit_singleline(&mut self.poly_int_m_input);
                            sanitize_i64(&mut self.poly_int_m_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Exponent Denominator (n, { 0 < n ≤ 32 }):");
                            ui.text_edit_singleline(&mut self.poly_int_n_input);
                            sanitize_u64(&mut self.poly_int_n_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Divisor (d, u64):");
                            ui.text_edit_singleline(&mut self.poly_int_d_input);
                            sanitize_u64(&mut self.poly_int_d_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Start Period Offset (s, optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.poly_int_s_input).hint_text("None"),
                            );
                            if response.changed() && !self.poly_int_s_input.trim().is_empty() {
                                sanitize_u64(&mut self.poly_int_s_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Offset (o, i64):");
                            ui.text_edit_singleline(&mut self.poly_int_o_input);
                            sanitize_i64(&mut self.poly_int_o_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Initial Token Emission (b, u64):");
                            ui.text_edit_singleline(&mut self.poly_int_b_input);
                            sanitize_u64(&mut self.poly_int_b_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Minimum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.poly_int_min_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed()
                                && !self.poly_int_min_value_input.trim().is_empty()
                            {
                                sanitize_u64(&mut self.poly_int_min_value_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Maximum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.poly_int_max_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed()
                                && !self.poly_int_max_value_input.trim().is_empty()
                            {
                                sanitize_u64(&mut self.poly_int_max_value_input);
                            }
                        });
                    }

                    DistributionFunctionUI::Exponential => {
                        ui.horizontal(|ui| {
                            ui.label("        - Scaling Factor (a, { 0 < a ≤ 256 }):");
                            ui.text_edit_singleline(&mut self.exp_a_input);
                            sanitize_u64(&mut self.exp_a_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label(
                                "        - Exponent Rate Numerator (m, { -8 ≤ m ≤ 8 ; m ≠ 0 }):",
                            );
                            ui.text_edit_singleline(&mut self.exp_m_input);
                            sanitize_i64(&mut self.exp_m_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Exponent Rate Denominator (n, { 0 < n ≤ 32 }):");
                            ui.text_edit_singleline(&mut self.exp_n_input);
                            sanitize_u64(&mut self.exp_n_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Divisor (d, { u64 ; d ≠ 0 }):");
                            ui.text_edit_singleline(&mut self.exp_d_input);
                            sanitize_u64(&mut self.exp_d_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Start Period Offset (s, optional, u64):");
                            let response = ui
                                .add(TextEdit::singleline(&mut self.exp_s_input).hint_text("None"));
                            if response.changed() && !self.exp_s_input.trim().is_empty() {
                                sanitize_u64(&mut self.exp_s_input);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Offset (o, i64):");
                            ui.text_edit_singleline(&mut self.exp_o_input);
                            sanitize_i64(&mut self.exp_o_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Base amount (b, u64):");
                            ui.text_edit_singleline(&mut self.exp_b_input);
                            sanitize_i64(&mut self.exp_b_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label("        - Minimum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.exp_min_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() && !self.exp_min_value_input.trim().is_empty() {
                                sanitize_u64(&mut self.exp_min_value_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Maximum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.exp_max_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() && !self.exp_max_value_input.trim().is_empty() {
                                sanitize_u64(&mut self.exp_max_value_input);
                            }
                        });
                    }

                    DistributionFunctionUI::Logarithmic => {
                        ui.horizontal(|ui| {
                            ui.label(
                                "        - Scaling Factor (a, i64, { -32_766 ≤ a ≤ 32_767 }):",
                            );
                            ui.text_edit_singleline(&mut self.log_a_input);
                            sanitize_i64(&mut self.log_a_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Divisor (d, u64):");
                            ui.text_edit_singleline(&mut self.log_d_input);
                            sanitize_u64(&mut self.log_d_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Exponent Numerator (m, u64):");
                            ui.text_edit_singleline(&mut self.log_m_input);
                            sanitize_u64(&mut self.log_m_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Exponent Denominator (n, u64):");
                            ui.text_edit_singleline(&mut self.log_n_input);
                            sanitize_u64(&mut self.log_n_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Start Period Offset (s, optional, u64):");
                            let response = ui
                                .add(TextEdit::singleline(&mut self.log_s_input).hint_text("None"));
                            if response.changed() && !self.log_s_input.trim().is_empty() {
                                sanitize_u64(&mut self.log_s_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Offset (o, i64):");
                            ui.text_edit_singleline(&mut self.log_o_input);
                            sanitize_i64(&mut self.log_o_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Base Amount (b, u64):");
                            ui.text_edit_singleline(&mut self.log_b_input);
                            sanitize_u64(&mut self.log_b_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Minimum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.log_min_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() && !self.log_min_value_input.trim().is_empty() {
                                sanitize_u64(&mut self.log_min_value_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Maximum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.log_max_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() && !self.log_max_value_input.trim().is_empty() {
                                sanitize_u64(&mut self.log_max_value_input);
                            }
                        });
                    }

                    DistributionFunctionUI::InvertedLogarithmic => {
                        ui.horizontal(|ui| {
                            ui.label(
                                "        - Scaling Factor (a, i64, { -32_766 ≤ a ≤ 32_767 }):",
                            );
                            ui.text_edit_singleline(&mut self.inv_log_a_input);
                            sanitize_i64(&mut self.inv_log_a_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Divisor (d, u64):");
                            ui.text_edit_singleline(&mut self.inv_log_d_input);
                            sanitize_u64(&mut self.inv_log_d_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Exponent Numerator (m, u64):");
                            ui.text_edit_singleline(&mut self.inv_log_m_input);
                            sanitize_u64(&mut self.inv_log_m_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Exponent Denominator (n, u64):");
                            ui.text_edit_singleline(&mut self.inv_log_n_input);
                            sanitize_u64(&mut self.inv_log_n_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Start Period Offset (s, optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.inv_log_s_input).hint_text("None"),
                            );
                            if response.changed() && !self.inv_log_s_input.trim().is_empty() {
                                sanitize_u64(&mut self.inv_log_s_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Offset (o, i64):");
                            ui.text_edit_singleline(&mut self.inv_log_o_input);
                            sanitize_i64(&mut self.inv_log_o_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Base Amount (b, u64):");
                            ui.text_edit_singleline(&mut self.inv_log_b_input);
                            sanitize_u64(&mut self.inv_log_b_input);
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Minimum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.inv_log_min_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() && !self.inv_log_min_value_input.trim().is_empty()
                            {
                                sanitize_u64(&mut self.inv_log_min_value_input);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("        - Maximum Emission Value (optional, u64):");
                            let response = ui.add(
                                TextEdit::singleline(&mut self.inv_log_max_value_input)
                                    .hint_text("None"),
                            );
                            if response.changed() && !self.inv_log_max_value_input.trim().is_empty()
                            {
                                sanitize_u64(&mut self.inv_log_max_value_input);
                            }
                        });
                    }
                }

                ui.add_space(10.0);

                // 4) Choose the distribution recipient
                ui.horizontal(|ui| {
                    ui.label("     Recipient:");
                    ComboBox::from_id_salt("perpetual_dist_recipient_selector")
                        .selected_text(format!("{:?}", self.perpetual_dist_recipient))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.perpetual_dist_recipient,
                                TokenDistributionRecipientUI::ContractOwner,
                                "Contract Owner",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_recipient,
                                TokenDistributionRecipientUI::Identity,
                                "Specific Identity",
                            );
                            ui.selectable_value(
                                &mut self.perpetual_dist_recipient,
                                TokenDistributionRecipientUI::EvonodesByParticipation,
                                "Evonodes",
                            );
                        });

                    // If user selected Identity or Group, show extra text edit
                    if self.perpetual_dist_recipient == TokenDistributionRecipientUI::Identity {
                        if self.perpetual_dist_recipient_identity_input.is_none() {
                            self.perpetual_dist_recipient_identity_input = Some(String::new());
                        }
                        if let Some(ref mut id) = self.perpetual_dist_recipient_identity_input {
                            ui.add(TextEdit::singleline(id).hint_text("Enter base58 id"));
                        }
                    }
                });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label(" ");
                    self.perpetual_distribution_rules
                        .render_control_change_rules_ui(
                            ui,
                            &self.groups_ui,
                            "Perpetual Distribution Rules",
                            None,
                            &mut self.token_creator_perpetual_distribution_rules_expanded,
                            None,
                        );
                });

                ui.add_space(5.0);
            } else {
                self.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::None;
            }

            ui.separator();

            // PRE-PROGRAMMED DISTRIBUTION
            ui.checkbox(
                &mut self.enable_pre_programmed_distribution,
                "Enable Pre-Programmed Distribution",
            );

            if self.enable_pre_programmed_distribution {
                ui.add_space(2.0);

                let mut i = 0;
                while i < self.pre_programmed_distributions.len() {
                    // Clone the current entry
                    let mut entry = self.pre_programmed_distributions[i].clone();

                    // Render row
                    ui.horizontal(|ui| {
                        ui.label(format!("Timestamp #{}:", i + 1));

                        // Replace text-edit timestamp with days/hours/minutes
                        ui.add(
                            egui::DragValue::new(&mut entry.days)
                                .prefix("Days: ")
                                .range(0..=3650),
                        );
                        ui.add(
                            egui::DragValue::new(&mut entry.hours)
                                .prefix("Hours: ")
                                .range(0..=23),
                        );
                        ui.add(
                            egui::DragValue::new(&mut entry.minutes)
                                .prefix("Minutes: ")
                                .range(0..=59),
                        );

                        ui.label("Identity:");
                        ui.text_edit_singleline(&mut entry.identity_str);

                        ui.label("Amount:");
                        ui.text_edit_singleline(&mut entry.amount_str);

                        // Remove button
                        if ui.button("Remove").clicked() {
                            self.pre_programmed_distributions.remove(i);
                        } else {
                            self.pre_programmed_distributions[i] = entry;
                        }
                    });

                    i += 1;
                }

                ui.add_space(2.0);

                // Add a button to insert a blank row
                ui.horizontal(|ui| {
                    ui.label("   ");
                    if ui.button("Add New Distribution Entry").clicked() {
                        self.pre_programmed_distributions
                            .push(DistributionEntry::default());
                    }
                });
            }
            });
        }
    }
}

// validate_perpetual_distribution_recipient has been moved to crate::model::tokens
pub use crate::model::tokens::validate_perpetual_distribution_recipient;
