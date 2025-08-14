use crate::ui::theme::DashColors;
use crate::ui::tokens::tokens_screen::{
    DistributionEntry, DistributionFunctionUI, IntervalTimeUnit,
    PerpetualDistributionIntervalTypeUI, TokenDistributionRecipientUI, TokensScreen, sanitize_i64,
    sanitize_u64,
};
use eframe::epaint::Color32;
use egui::{ComboBox, Context, RichText, TextEdit};

impl TokensScreen {
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

                // Based on the user’s chosen function, display relevant fields:
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
                            if !self.step_count_input.is_empty() {
                                if let Ok((perpetual_dist_interval_input, step_count_input)) = self
                                    .perpetual_dist_interval_input
                                    .parse::<u64>()
                                    .and_then(|perpetual_dist_interval_input| {
                                        self.step_count_input.parse::<u64>().map(
                                            |step_count_input| {
                                                (perpetual_dist_interval_input, step_count_input)
                                            },
                                        )
                                    })
                                {
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

                                if let Ok((perpetual_dist_interval_input, step_position)) = self.perpetual_dist_interval_input.parse::<u64>().and_then(|perpetual_dist_interval_input| steps_str.parse::<u64>().map(|step_count_input| (perpetual_dist_interval_input, step_count_input))) {
                                    if let Ok(amount) = amount_str.parse::<u64>() {
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


                                }

                                // If remove is clicked, remove the step at index i
                                // and *do not* increment i, because the next element
                                // now “shifts” into this index.
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
