//! Editable recurring functions, evaluated by the pinned protocol implementation.

use std::collections::BTreeMap;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
use dash_sdk::dpp::version::PlatformVersion;
use super::{DraftError, DraftIssue, Section};
use super::amount::{TokenAmountInput, parse_amount};

/// The eight supported recurring functions. Random is not accepted by Platform.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum FunctionKind {
    #[default]
    Fixed,
    StepDecreasing,
    Stepwise,
    Linear,
    Polynomial,
    Exponential,
    Logarithmic,
    InvertedLogarithmic,
}

impl FunctionKind {
    pub const ALL: [Self; 8] = [
        Self::Fixed,
        Self::StepDecreasing,
        Self::Stepwise,
        Self::Linear,
        Self::Polynomial,
        Self::Exponential,
        Self::Logarithmic,
        Self::InvertedLogarithmic,
    ];
    /// Readable name, followed by the protocol spelling where different.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed amount (FixedAmount)",
            Self::StepDecreasing => "Step decrease (StepDecreasing)",
            Self::Stepwise => "Phased amounts (Stepwise)",
            Self::Linear => "Linear",
            Self::Polynomial => "Polynomial",
            Self::Exponential => "Exponential",
            Self::Logarithmic => "Logarithmic",
            Self::InvertedLogarithmic => "Inverted logarithmic",
        }
    }
    /// Stable contextual-help topic.
    pub fn help_key(self) -> &'static str {
        match self {
            Self::Fixed => "function.FixedAmount",
            Self::StepDecreasing => "function.StepDecreasing",
            Self::Stepwise => "function.Stepwise",
            Self::Linear => "function.Linear",
            Self::Polynomial => "function.Polynomial",
            Self::Exponential => "function.Exponential",
            Self::Logarithmic => "function.Logarithmic",
            Self::InvertedLogarithmic => "function.InvertedLogarithmic",
        }
    }
    /// Fields actually accepted for this function.
    pub fn parameters(self) -> &'static [Parameter] {
        use Parameter::*;
        match self {
            Self::Fixed => &[Amount],
            Self::StepDecreasing => &[
                StepCount,
                Numerator,
                Denominator,
                S,
                Initial,
                Minimum,
                MaxEra,
                Trailing,
            ],
            Self::Stepwise => &[],
            Self::Linear => &[A, D, S, B, Minimum, Maximum],
            Self::Polynomial
            | Self::Exponential
            | Self::Logarithmic
            | Self::InvertedLogarithmic => &[A, D, M, N, S, O, B, Minimum, Maximum],
        }
    }
}

/// Each coefficient has a distinct dimension; only amount fields scale with precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Parameter {
    Amount,
    A,
    B,
    D,
    M,
    N,
    O,
    S,
    StepCount,
    Numerator,
    Denominator,
    Initial,
    Minimum,
    Maximum,
    MaxEra,
    Trailing,
}

impl Parameter {
    /// Whether this value represents tokens rather than a coefficient or count.
    pub fn is_amount(self) -> bool {
        matches!(
            self,
            Self::Amount | Self::B | Self::Initial | Self::Minimum | Self::Maximum | Self::Trailing
        )
    }
    /// Whether an empty entry means an absent protocol option, distinct from zero.
    pub fn optional(self) -> bool {
        matches!(self, Self::S | Self::Minimum | Self::Maximum | Self::MaxEra)
    }
    /// Label including the dimension.
    pub fn label(self) -> &'static str {
        match self {
            Self::Amount => "Amount per interval",
            Self::A => "a · scaling coefficient (base-unit calculation)",
            Self::B => "b · added token amount",
            Self::D => "d · positive scaling divisor",
            Self::M => "m · ratio numerator",
            Self::N => "n · positive ratio denominator",
            Self::O => "o · input offset (index)",
            Self::S => "s · start index (optional)",
            Self::StepCount => "Intervals per decrease",
            Self::Numerator => "Removed fraction numerator",
            Self::Denominator => "Removed fraction denominator",
            Self::Initial => "Initial recurring amount",
            Self::Minimum => "Minimum per interval (optional)",
            Self::Maximum => "Maximum per interval (optional)",
            Self::MaxEra => "Maximum reduction era count (optional)",
            Self::Trailing => "Trailing amount",
        }
    }
    /// Contextual-help topic for the parameter.
    pub fn help_key(self) -> &'static str {
        match self {
            Self::Amount => "param.amount",
            Self::A => "param.a",
            Self::B => "param.b",
            Self::D => "param.d",
            Self::M => "param.m",
            Self::N => "param.n",
            Self::O => "param.o",
            Self::S => "param.s",
            Self::StepCount => "param.step_count",
            Self::Numerator => "param.decrease_numerator",
            Self::Denominator => "param.decrease_denominator",
            Self::Initial => "param.initial",
            Self::Minimum => "param.minimum",
            Self::Maximum => "param.maximum",
            Self::MaxEra => "param.maximum_interval_count",
            Self::Trailing => "param.trailing",
        }
    }
}

/// Retains every entered parameter, including incomplete and invalid text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDraft {
    pub kind: FunctionKind,
    pub values: BTreeMap<Parameter, String>,
    pub steps: Vec<(String, TokenAmountInput)>,
    pub amounts_in_base_units: bool,
}

impl FunctionDraft {
    /// A verified tutorial recipe at precision two, expressed in displayed tokens.
    pub fn example(kind: FunctionKind) -> Self {
        use Parameter::*;
        let entries: &[(Parameter, &str)] = match kind {
            FunctionKind::Fixed => &[(Amount, "1.00")],
            FunctionKind::StepDecreasing => &[
                (StepCount, "2"),
                (Numerator, "1"),
                (Denominator, "2"),
                (S, ""),
                (Initial, "1.00"),
                (Minimum, "0.10"),
                (MaxEra, "3"),
                (Trailing, "0.10"),
            ],
            FunctionKind::Stepwise => &[],
            FunctionKind::Linear => &[
                (A, "2"),
                (D, "1"),
                (S, ""),
                (B, "0.10"),
                (Minimum, "0"),
                (Maximum, "0.20"),
            ],
            FunctionKind::Polynomial => &[
                (A, "1"),
                (M, "2"),
                (N, "1"),
                (D, "1"),
                (S, ""),
                (O, "1"),
                (B, "0"),
                (Minimum, "0"),
                (Maximum, "1.00"),
            ],
            FunctionKind::Exponential => &[
                (A, "10"),
                (M, "1"),
                (N, "1"),
                (D, "1"),
                (S, ""),
                (O, "0"),
                (B, "0"),
                (Minimum, "0"),
                (Maximum, "1.00"),
            ],
            FunctionKind::Logarithmic => &[
                (A, "10"),
                (M, "1"),
                (N, "1"),
                (D, "1"),
                (S, ""),
                (O, "1"),
                (B, "0.10"),
                (Minimum, "0"),
                (Maximum, "1.00"),
            ],
            FunctionKind::InvertedLogarithmic => &[
                (A, "10"),
                (M, "1"),
                (N, "1"),
                (D, "1"),
                (S, ""),
                (O, "1"),
                (B, "0.40"),
                (Minimum, "0"),
                (Maximum, "1.00"),
            ],
        };
        Self {
            kind,
            values: entries
                .iter()
                .map(|(key, value)| (*key, (*value).into()))
                .collect(),
            amounts_in_base_units: false,
            steps: if kind == FunctionKind::Stepwise {
                vec![
                    ("0".into(), TokenAmountInput::tokens("0.10")),
                    ("3".into(), TokenAmountInput::tokens("0.05")),
                    ("5".into(), TokenAmountInput::tokens("0")),
                ]
            } else {
                Vec::new()
            },
        }
    }

    fn text(&self, key: Parameter) -> &str {
        self.values.get(&key).map_or("", String::as_str).trim()
    }
    fn error(&self, key: Parameter, issue: DraftIssue) -> DraftError {
        DraftError::new(Section::Distribution, key.label(), issue)
    }
    fn unsigned(&self, key: Parameter) -> Result<u64, DraftError> {
        let text = self.text(key);
        if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
            return Err(self.error(key, DraftIssue::Unsigned));
        }
        text.parse()
            .map_err(|_| self.error(key, DraftIssue::Unsigned))
    }
    fn signed(&self, key: Parameter) -> Result<i64, DraftError> {
        self.text(key)
            .parse()
            .map_err(|_| self.error(key, DraftIssue::Signed))
    }
    fn optional_integer(&self, key: Parameter) -> Result<Option<u64>, DraftError> {
        if self.text(key).is_empty() {
            Ok(None)
        } else {
            self.unsigned(key).map(Some)
        }
    }
    fn amount(&self, key: Parameter, decimals: u8) -> Result<u64, DraftError> {
        parse_amount(
            self.text(key),
            if self.amounts_in_base_units {
                0
            } else {
                decimals
            },
        )
        .map_err(|source| self.error(key, DraftIssue::Amount(source)))
    }
    fn optional_amount(&self, key: Parameter, decimals: u8) -> Result<Option<u64>, DraftError> {
        if self.text(key).is_empty() {
            Ok(None)
        } else {
            self.amount(key, decimals).map(Some)
        }
    }

    /// Parses all coefficients and validates the result with the pinned protocol.
    pub fn parse(
        &self,
        decimals: u8,
        registration_index: u64,
        version: &PlatformVersion,
    ) -> Result<DistributionFunction, DraftError> {
        use Parameter::*;
        let function = match self.kind {
            FunctionKind::Fixed => DistributionFunction::FixedAmount {
                amount: self.amount(Amount, decimals)?,
            },
            FunctionKind::StepDecreasing => DistributionFunction::StepDecreasingAmount {
                step_count: self
                    .unsigned(StepCount)?
                    .try_into()
                    .map_err(|_| self.error(StepCount, DraftIssue::Unsigned))?,
                decrease_per_interval_numerator: self
                    .unsigned(Numerator)?
                    .try_into()
                    .map_err(|_| self.error(Numerator, DraftIssue::Unsigned))?,
                decrease_per_interval_denominator: self
                    .unsigned(Denominator)?
                    .try_into()
                    .map_err(|_| self.error(Denominator, DraftIssue::Unsigned))?,
                start_decreasing_offset: self.optional_integer(S)?,
                max_interval_count: self
                    .optional_integer(MaxEra)?
                    .map(u16::try_from)
                    .transpose()
                    .map_err(|_| self.error(MaxEra, DraftIssue::Unsigned))?,
                distribution_start_amount: self.amount(Initial, decimals)?,
                trailing_distribution_interval_amount: self.amount(Trailing, decimals)?,
                min_value: self.optional_amount(Minimum, decimals)?,
            },
            FunctionKind::Stepwise => {
                let mut steps = BTreeMap::new();
                for (index, amount) in &self.steps {
                    let index = index
                        .parse::<u64>()
                        .map_err(|_| self.error(S, DraftIssue::Unsigned))?;
                    let amount = amount
                        .parse(decimals)
                        .map_err(|source| self.error(Amount, DraftIssue::Amount(source)))?;
                    if steps.insert(index, amount).is_some() {
                        return Err(self.error(S, DraftIssue::DuplicateStep));
                    }
                }
                DistributionFunction::Stepwise(steps)
            }
            FunctionKind::Linear => DistributionFunction::Linear {
                a: self.signed(A)?,
                d: self.unsigned(D)?,
                start_step: self.optional_integer(S)?,
                starting_amount: self.amount(B, decimals)?,
                min_value: self.optional_amount(Minimum, decimals)?,
                max_value: self.optional_amount(Maximum, decimals)?,
            },
            FunctionKind::Polynomial => DistributionFunction::Polynomial {
                a: self.signed(A)?,
                d: self.unsigned(D)?,
                m: self.signed(M)?,
                n: self.unsigned(N)?,
                start_moment: self.optional_integer(S)?,
                o: self.signed(O)?,
                b: self.amount(B, decimals)?,
                min_value: self.optional_amount(Minimum, decimals)?,
                max_value: self.optional_amount(Maximum, decimals)?,
            },
            FunctionKind::Exponential => DistributionFunction::Exponential {
                a: self.unsigned(A)?,
                d: self.unsigned(D)?,
                m: self.signed(M)?,
                n: self.unsigned(N)?,
                start_moment: self.optional_integer(S)?,
                o: self.signed(O)?,
                b: self.amount(B, decimals)?,
                min_value: self.optional_amount(Minimum, decimals)?,
                max_value: self.optional_amount(Maximum, decimals)?,
            },
            FunctionKind::Logarithmic => DistributionFunction::Logarithmic {
                a: self.signed(A)?,
                d: self.unsigned(D)?,
                m: self.unsigned(M)?,
                n: self.unsigned(N)?,
                start_moment: self.optional_integer(S)?,
                o: self.signed(O)?,
                b: self.amount(B, decimals)?,
                min_value: self.optional_amount(Minimum, decimals)?,
                max_value: self.optional_amount(Maximum, decimals)?,
            },
            FunctionKind::InvertedLogarithmic => DistributionFunction::InvertedLogarithmic {
                a: self.signed(A)?,
                d: self.unsigned(D)?,
                m: self.unsigned(M)?,
                n: self.unsigned(N)?,
                start_moment: self.optional_integer(S)?,
                o: self.signed(O)?,
                b: self.amount(B, decimals)?,
                min_value: self.optional_amount(Minimum, decimals)?,
                max_value: self.optional_amount(Maximum, decimals)?,
            },
        };
        let validation = function
            .validate(registration_index, version)
            .map_err(|source| {
                DraftError::protocol(Section::Distribution, "Recurring function", source)
            })?;
        if !validation.is_valid() {
            return Err(DraftError::consensus(
                Section::Distribution,
                "Recurring function",
                validation.errors,
            ));
        }
        Ok(function)
    }

    /// Changes all amount representations atomically; coefficients never change units.
    pub fn change_units(&mut self, base_units: bool, decimals: u8) -> Result<(), DraftError> {
        if self.amounts_in_base_units == base_units {
            return Ok(());
        }
        let mut converted = self.clone();
        for key in self
            .kind
            .parameters()
            .iter()
            .copied()
            .filter(|key| key.is_amount())
        {
            if key.optional() && self.text(key).is_empty() {
                continue;
            }
            let mut amount = TokenAmountInput {
                text: self.text(key).into(),
                base_units: self.amounts_in_base_units,
            };
            amount
                .change_units(base_units, decimals)
                .map_err(|source| self.error(key, DraftIssue::Amount(source)))?;
            converted.values.insert(key, amount.text);
        }
        for (_, amount) in &mut converted.steps {
            amount
                .change_units(base_units, decimals)
                .map_err(|source| self.error(Parameter::Amount, DraftIssue::Amount(source)))?;
        }
        converted.amounts_in_base_units = base_units;
        *self = converted;
        Ok(())
    }
}

/// An illustrative evaluated period, excluding index zero from cumulative allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmissionRow {
    pub index: u64,
    pub amount: u64,
    pub cumulative: u64,
}

/// Evaluates the current parameters, not a static chart. Registration index is explicit.
pub fn evaluate_preview(
    function: &DistributionFunction,
    registration_index: u64,
    version: &PlatformVersion,
) -> Result<Vec<EmissionRow>, DraftError> {
    let mut cumulative = 0u64;
    let mut rows = Vec::new();
    for index in 0..=10 {
        let x = registration_index.checked_add(index).ok_or_else(|| {
            DraftError::new(Section::Distribution, "Preview index", DraftIssue::Unsigned)
        })?;
        let amount = function
            .evaluate(registration_index, x, version)
            .map_err(|source| DraftError::protocol(Section::Distribution, "Preview", source))?;
        if index > 0 {
            cumulative = cumulative.checked_add(amount).ok_or_else(|| {
                DraftError::new(Section::Distribution, "Preview total", DraftIssue::Unsigned)
            })?;
        }
        rows.push(EmissionRow {
            index,
            amount,
            cumulative,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_examples_match_the_pinned_evaluator() {
        let expected = [
            [100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100],
            [100, 100, 50, 50, 25, 25, 12, 12, 10, 10, 10],
            [10, 10, 10, 5, 5, 0, 0, 0, 0, 0, 0],
            [10, 12, 14, 16, 18, 20, 20, 20, 20, 20, 20],
            [1, 4, 9, 16, 25, 36, 49, 64, 81, 100, 100],
            [10, 27, 73, 100, 100, 100, 100, 100, 100, 100, 100],
            [10, 16, 20, 23, 26, 27, 29, 30, 31, 33, 33],
            [40, 33, 29, 26, 23, 22, 20, 19, 18, 16, 16],
        ];
        let version = PlatformVersion::latest();
        for (kind, amounts) in FunctionKind::ALL.into_iter().zip(expected) {
            let function = FunctionDraft::example(kind).parse(2, 0, version).unwrap();
            let rows = evaluate_preview(&function, 0, version).unwrap();
            assert_eq!(
                rows.iter().map(|row| row.amount).collect::<Vec<_>>(),
                amounts,
                "{kind:?}"
            );
            assert_eq!(rows[5].cumulative, amounts[1..=5].iter().sum::<u64>());
        }
    }

    #[test]
    fn edited_parameters_recalculate_and_duplicate_steps_are_refused() {
        let version = PlatformVersion::latest();
        let mut linear = FunctionDraft::example(FunctionKind::Linear);
        linear.values.insert(Parameter::A, "1".into());
        let rows = evaluate_preview(&linear.parse(2, 0, version).unwrap(), 0, version).unwrap();
        assert_eq!(rows[1].amount, 11);
        let mut steps = FunctionDraft::example(FunctionKind::Stepwise);
        steps
            .steps
            .push(("3".into(), TokenAmountInput::tokens("1")));
        assert!(steps.parse(2, 0, version).is_err());
    }

    #[test]
    fn explicit_zero_start_is_distinct_from_absent_start() {
        let mut draft = FunctionDraft::example(FunctionKind::Linear);
        draft.values.insert(Parameter::S, "0".into());
        let function = draft.parse(2, 0, PlatformVersion::latest()).unwrap();
        assert!(matches!(
            function,
            DistributionFunction::Linear {
                start_step: Some(0),
                ..
            }
        ));
    }

    #[test]
    fn negative_linear_added_amount_is_not_supported_by_pinned_protocol() {
        let mut draft = FunctionDraft::example(FunctionKind::Linear);
        draft.values.insert(Parameter::B, "-0.10".into());
        assert!(draft.parse(2, 0, PlatformVersion::latest()).is_err());
    }
}
