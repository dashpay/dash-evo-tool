//! Which balance pays a send, and where a send to one's own wallet lands.
//!
//! Pure rules behind the Send form's "Auto" choices
//! (`docs/ai-design/2026-10-23-send-screen-redesign/02-ux-spec.md` §4–§5).
//! The screen gathers each balance's spendable maximum — the same figure
//! "Max" offers for that route — and renders whatever these functions decide.
//!
//! Auto never spends identity credits, never unshields private funds for a
//! non-private recipient, and never splits one payment across two balances.

use crate::model::address::AddressKind;

/// A concrete balance a send can pay from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PayFromKind {
    /// The wallet's pooled Core (L1) accounts.
    Core,
    /// The wallet's Platform addresses.
    Platform,
    /// One of the wallet's identities.
    Identity,
    /// The wallet's private (shielded) balance.
    Shielded,
}

impl PayFromKind {
    /// The kind of address a balance of this kind receives into.
    pub fn address_kind(self) -> AddressKind {
        match self {
            Self::Core => AddressKind::Core,
            Self::Platform => AddressKind::Platform,
            Self::Identity => AddressKind::Identity,
            Self::Shielded => AddressKind::Shielded,
        }
    }
}

/// What one balance can send to the current destination: the amount "Max"
/// would offer for that route, in credits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CandidateMax {
    /// The maximum is known.
    Ready(u64),
    /// The maximum comes from a backend quote that has not arrived yet.
    Pending,
    /// The backend quote for the maximum failed.
    Failed,
    /// The balance holds funds, but another transaction from the wallet holds
    /// them until it finishes, so how much it can send is unknown.
    Withheld,
    /// The balance cannot pay this destination at all (absent, gated off,
    /// not yet verified).
    #[default]
    Unavailable,
}

/// Spendable maxima of the balances Auto may choose from, for one destination.
/// Identity credits are deliberately absent: Auto never spends them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CandidateMaxima {
    pub core: CandidateMax,
    pub platform: CandidateMax,
    pub shielded: CandidateMax,
}

impl CandidateMaxima {
    /// The maximum of `kind`; identities are never a candidate.
    pub fn get(&self, kind: PayFromKind) -> CandidateMax {
        match kind {
            PayFromKind::Core => self.core,
            PayFromKind::Platform => self.platform,
            PayFromKind::Shielded => self.shielded,
            PayFromKind::Identity => CandidateMax::Unavailable,
        }
    }
}

/// The destination Auto resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoDestination {
    /// Kind of the destination, or `None` while nobody has been entered.
    pub kind: Option<AddressKind>,
    /// For a shielded destination: whether it is the paying wallet's own
    /// private balance. Core and Platform can only shield into the paying
    /// wallet's own pool, so a stranger's private address is payable from
    /// the private balance alone.
    pub shielded_is_own_pool: bool,
    /// The balance that is the destination account itself (same wallet and
    /// same kind). It is never a candidate: paying it would only cost a fee.
    pub same_account: Option<PayFromKind>,
}

impl AutoDestination {
    /// A destination of `kind` belonging to someone else.
    pub fn external(kind: Option<AddressKind>) -> Self {
        Self {
            kind,
            shielded_is_own_pool: false,
            same_account: None,
        }
    }
}

/// What source Auto settled on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoSource {
    /// Pay from `kind`, which can send up to `available` credits. `passed_over`
    /// lists the earlier candidates that could not cover the amount, in order.
    Chosen {
        kind: PayFromKind,
        available: u64,
        passed_over: Vec<PayFromKind>,
    },
    /// The candidate being checked still waits for its quote; later candidates
    /// are not consulted until it arrives.
    Checking { kind: PayFromKind },
    /// The quote of the candidate being checked failed.
    QuoteFailed { kind: PayFromKind },
    /// The candidate being checked is held by another transaction; Auto
    /// waits rather than switch to a different kind of payment.
    Withheld { kind: PayFromKind },
    /// No destination yet: the larger of Core and Platform, as a preview only.
    Preview { kind: PayFromKind, available: u64 },
    /// No single balance covers the amount. `best` is the largest amount one
    /// payment can send and the balance that sends it.
    NothingCovers { best: Option<(PayFromKind, u64)> },
    /// None of the candidate balances holds anything this destination accepts.
    NoFunds,
}

/// Outcome of "Max" while the source is Auto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoMax {
    /// Pay everything `kind` can send: `max` credits.
    Chosen { kind: PayFromKind, max: u64 },
    /// Some candidates' quotes are still outstanding.
    Checking { pending: Vec<PayFromKind> },
    /// The maximum of `kind` could not be checked, so the largest is unknown
    /// until that quote is retried.
    QuoteFailed { kind: PayFromKind },
    /// `kind` is held by another transaction, so the largest is unknown
    /// until it finishes.
    Withheld { kind: PayFromKind },
    /// Nothing can be sent to this destination.
    NoFunds,
}

/// Why a source cannot pay a destination kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteRefusal {
    /// Identities cannot pay a private address directly.
    IdentityToShielded,
    /// The private balance cannot pay an identity directly.
    ShieldedToIdentity,
    /// Core and Platform can only shield into the paying wallet's own private
    /// balance, never into someone else's.
    ShieldedNotOwnPool,
}

/// Candidates Auto tries for `destination`, in order, before exclusions.
pub fn candidate_order(destination: &AutoDestination) -> Vec<PayFromKind> {
    use PayFromKind::*;
    match destination.kind {
        Some(AddressKind::Core) => vec![Core, Platform],
        Some(AddressKind::Platform) | Some(AddressKind::Identity) => vec![Platform, Core],
        Some(AddressKind::Shielded) if destination.shielded_is_own_pool => {
            vec![Shielded, Platform, Core]
        }
        Some(AddressKind::Shielded) => vec![Shielded],
        None => vec![Core, Platform],
    }
}

/// Candidates Auto may actually use: [`candidate_order`] without the
/// destination account itself and without a balance this destination cannot
/// accept.
fn usable_candidates(destination: &AutoDestination) -> Vec<PayFromKind> {
    candidate_order(destination)
        .into_iter()
        .filter(|kind| Some(*kind) != destination.same_account)
        .filter(|kind| {
            destination.kind.is_none_or(|dest| {
                route_refusal(*kind, dest, destination.shielded_is_own_pool).is_none()
            })
        })
        .collect()
}

/// Resolve source Auto (§4.1): the first candidate whose maximum covers
/// `amount_credits` (or holds anything, with no amount entered).
pub fn resolve_source_auto(
    destination: &AutoDestination,
    amount_credits: Option<u64>,
    maxima: &CandidateMaxima,
) -> AutoSource {
    let candidates = usable_candidates(destination);
    if destination.kind.is_none() {
        let best = candidates
            .iter()
            .filter_map(|kind| match maxima.get(*kind) {
                CandidateMax::Ready(max) if max > 0 => Some((*kind, max)),
                _ => None,
            })
            .fold(None::<(PayFromKind, u64)>, |best, (kind, max)| match best {
                Some((_, best_max)) if best_max >= max => best,
                _ => Some((kind, max)),
            });
        return match best {
            Some((kind, available)) => AutoSource::Preview { kind, available },
            None => AutoSource::NoFunds,
        };
    }

    let amount = amount_credits.filter(|amount| *amount > 0);
    let mut passed_over = Vec::new();
    let mut best: Option<(PayFromKind, u64)> = None;
    for kind in candidates {
        match maxima.get(kind) {
            CandidateMax::Pending => return AutoSource::Checking { kind },
            CandidateMax::Failed => return AutoSource::QuoteFailed { kind },
            CandidateMax::Withheld => return AutoSource::Withheld { kind },
            CandidateMax::Unavailable => {}
            CandidateMax::Ready(max) => {
                let covers = match amount {
                    Some(amount) => max >= amount,
                    None => max > 0,
                };
                if covers {
                    return AutoSource::Chosen {
                        kind,
                        available: max,
                        passed_over,
                    };
                }
                if max > 0 {
                    if best.is_none_or(|(_, best_max)| max > best_max) {
                        best = Some((kind, max));
                    }
                    passed_over.push(kind);
                }
            }
        }
    }
    match (amount, best) {
        (Some(_), Some(best)) => AutoSource::NothingCovers { best: Some(best) },
        _ => AutoSource::NoFunds,
    }
}

/// "Max" with source Auto (§4.1): the candidate that can send the most. Every
/// candidate's maximum must be known; ties go to the earlier candidate.
pub fn resolve_auto_max(destination: &AutoDestination, maxima: &CandidateMaxima) -> AutoMax {
    let candidates = usable_candidates(destination);
    let pending: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|kind| maxima.get(*kind) == CandidateMax::Pending)
        .collect();
    if !pending.is_empty() {
        return AutoMax::Checking { pending };
    }
    if let Some(kind) = candidates
        .iter()
        .copied()
        .find(|kind| maxima.get(*kind) == CandidateMax::Failed)
    {
        return AutoMax::QuoteFailed { kind };
    }
    if let Some(kind) = candidates
        .iter()
        .copied()
        .find(|kind| maxima.get(*kind) == CandidateMax::Withheld)
    {
        return AutoMax::Withheld { kind };
    }
    candidates
        .into_iter()
        .filter_map(|kind| match maxima.get(kind) {
            CandidateMax::Ready(max) if max > 0 => Some((kind, max)),
            _ => None,
        })
        .fold(None::<(PayFromKind, u64)>, |best, (kind, max)| match best {
            Some((_, best_max)) if best_max >= max => best,
            _ => Some((kind, max)),
        })
        .map_or(AutoMax::NoFunds, |(kind, max)| AutoMax::Chosen {
            kind,
            max,
        })
}

/// Resolve destination Auto (§4.2): the kind of address that receives a send
/// from `source` into one of the user's own wallets.
///
/// Same kind as the source; within the paying wallet the other kind
/// (Core ↔ Platform, private → Platform) so it never pays itself; from an
/// identity always a Platform address.
pub fn resolve_destination_auto(source: PayFromKind, same_wallet: bool) -> AddressKind {
    match (source, same_wallet) {
        (PayFromKind::Identity, _) => AddressKind::Platform,
        (PayFromKind::Core, true) => AddressKind::Platform,
        (PayFromKind::Platform, true) => AddressKind::Core,
        (PayFromKind::Shielded, true) => AddressKind::Platform,
        (kind, false) => kind.address_kind(),
    }
}

/// Receive-into default when the user switches To to "My wallets" (§5),
/// given the paying wallet's Core and Platform funds. A different wallet takes
/// the kind holding the most funds; the same wallet takes the other kind, so
/// Auto pays from the larger balance and never sends to itself. Ties go to
/// Core.
pub fn default_receive_kind(
    core_funds: u64,
    platform_funds: u64,
    same_wallet: bool,
) -> AddressKind {
    let largest = if platform_funds > core_funds {
        AddressKind::Platform
    } else {
        AddressKind::Core
    };
    match (same_wallet, largest) {
        (false, kind) => kind,
        (true, AddressKind::Core) => AddressKind::Platform,
        (true, _) => AddressKind::Core,
    }
}

/// Why `source` cannot pay a `destination` of this kind, if it cannot.
/// `shielded_is_own_pool` tells whether a shielded destination is the paying
/// wallet's own private balance.
pub fn route_refusal(
    source: PayFromKind,
    destination: AddressKind,
    shielded_is_own_pool: bool,
) -> Option<RouteRefusal> {
    match (source, destination) {
        (PayFromKind::Identity, AddressKind::Shielded) => Some(RouteRefusal::IdentityToShielded),
        (PayFromKind::Shielded, AddressKind::Identity) => Some(RouteRefusal::ShieldedToIdentity),
        (PayFromKind::Core | PayFromKind::Platform, AddressKind::Shielded)
            if !shielded_is_own_pool =>
        {
            Some(RouteRefusal::ShieldedNotOwnPool)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CandidateMax::{Failed, Pending, Ready, Unavailable, Withheld};
    use PayFromKind::{Core, Identity, Platform, Shielded};

    fn maxima(
        core: CandidateMax,
        platform: CandidateMax,
        shielded: CandidateMax,
    ) -> CandidateMaxima {
        CandidateMaxima {
            core,
            platform,
            shielded,
        }
    }

    fn to(kind: AddressKind) -> AutoDestination {
        AutoDestination::external(Some(kind))
    }

    fn own_pool() -> AutoDestination {
        AutoDestination {
            kind: Some(AddressKind::Shielded),
            shielded_is_own_pool: true,
            same_account: Some(Shielded),
        }
    }

    fn chosen(kind: PayFromKind, available: u64, passed_over: Vec<PayFromKind>) -> AutoSource {
        AutoSource::Chosen {
            kind,
            available,
            passed_over,
        }
    }

    #[test]
    fn candidate_order_follows_the_destination_kind() {
        assert_eq!(
            candidate_order(&to(AddressKind::Core)),
            vec![Core, Platform]
        );
        assert_eq!(
            candidate_order(&to(AddressKind::Platform)),
            vec![Platform, Core]
        );
        assert_eq!(
            candidate_order(&to(AddressKind::Identity)),
            vec![Platform, Core]
        );
        assert_eq!(candidate_order(&own_pool()), vec![Shielded, Platform, Core]);
    }

    #[test]
    fn a_strangers_private_address_is_payable_only_from_the_private_balance() {
        assert_eq!(candidate_order(&to(AddressKind::Shielded)), vec![Shielded]);
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Shielded),
                Some(10),
                &maxima(Ready(1_000), Ready(1_000), Ready(0)),
            ),
            AutoSource::NoFunds,
            "Core and Platform would shield into the sender's own pool"
        );
    }

    #[test]
    fn auto_pays_from_the_same_kind_when_it_covers_the_amount() {
        // AC-01: Core 0.8, Platform 0.3, a Core recipient, 0.5.
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Core),
                Some(50),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            chosen(Core, 80, vec![])
        );
    }

    #[test]
    fn auto_falls_back_and_says_what_it_passed_over() {
        // AC-03: a Platform recipient, Platform 0.3 cannot cover 0.5.
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Platform),
                Some(50),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            chosen(Core, 80, vec![Platform])
        );
        // 0.2 fits Platform again.
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Platform),
                Some(20),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            chosen(Platform, 30, vec![])
        );
    }

    #[test]
    fn an_identity_recipient_prefers_platform() {
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Identity),
                Some(20),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            chosen(Platform, 30, vec![])
        );
    }

    #[test]
    fn the_own_pool_is_funded_from_platform_then_core() {
        // AC-08 for the paying wallet's own private balance.
        assert_eq!(
            resolve_source_auto(
                &own_pool(),
                Some(50),
                &maxima(Ready(80), Ready(30), Ready(100))
            ),
            chosen(Core, 80, vec![Platform]),
            "the private balance is the destination itself and is skipped"
        );
    }

    #[test]
    fn no_single_balance_covering_offers_the_largest_one() {
        // AC-02: 1.0 with Core 0.8 and Platform 0.3.
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Core),
                Some(100),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            AutoSource::NothingCovers {
                best: Some((Core, 80))
            }
        );
    }

    #[test]
    fn auto_never_splits_a_payment_across_balances() {
        assert!(matches!(
            resolve_source_auto(
                &to(AddressKind::Core),
                Some(100),
                &maxima(Ready(60), Ready(60), Unavailable)
            ),
            AutoSource::NothingCovers {
                best: Some((Core, 60))
            }
        ));
    }

    #[test]
    fn an_empty_wallet_has_no_funds() {
        let empty = maxima(Ready(0), Ready(0), Unavailable);
        assert_eq!(
            resolve_source_auto(&to(AddressKind::Core), Some(1), &empty),
            AutoSource::NoFunds
        );
        assert_eq!(
            resolve_source_auto(&to(AddressKind::Core), None, &empty),
            AutoSource::NoFunds
        );
        assert_eq!(
            resolve_source_auto(&AutoDestination::external(None), None, &empty),
            AutoSource::NoFunds
        );
    }

    #[test]
    fn without_an_amount_auto_picks_the_first_balance_holding_anything() {
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Platform),
                None,
                &maxima(Ready(80), Ready(0), Unavailable)
            ),
            chosen(Core, 80, vec![])
        );
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Platform),
                Some(0),
                &maxima(Ready(80), Ready(5), Unavailable)
            ),
            chosen(Platform, 5, vec![]),
            "a zero amount counts as no amount"
        );
    }

    #[test]
    fn without_a_destination_auto_previews_the_larger_of_core_and_platform() {
        let unknown = AutoDestination::external(None);
        assert_eq!(
            resolve_source_auto(
                &unknown,
                Some(10),
                &maxima(Ready(30), Ready(80), Ready(500))
            ),
            AutoSource::Preview {
                kind: Platform,
                available: 80
            },
            "the private balance is never previewed"
        );
        assert_eq!(
            resolve_source_auto(&unknown, None, &maxima(Ready(80), Ready(80), Unavailable)),
            AutoSource::Preview {
                kind: Core,
                available: 80
            },
            "ties go to Core"
        );
    }

    #[test]
    fn a_pending_quote_stops_auto_at_that_candidate() {
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Core),
                Some(10),
                &maxima(Pending, Ready(80), Unavailable)
            ),
            AutoSource::Checking { kind: Core }
        );
        // A later candidate's pending quote is never consulted when an earlier one covers.
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Platform),
                Some(10),
                &maxima(Pending, Ready(80), Unavailable)
            ),
            chosen(Platform, 80, vec![])
        );
    }

    #[test]
    fn a_failed_quote_is_reported_instead_of_silently_skipped() {
        assert_eq!(
            resolve_source_auto(
                &to(AddressKind::Core),
                Some(10),
                &maxima(Failed, Ready(80), Unavailable)
            ),
            AutoSource::QuoteFailed { kind: Core }
        );
    }

    #[test]
    fn the_destination_account_itself_is_never_a_candidate() {
        let own_core = AutoDestination {
            kind: Some(AddressKind::Core),
            shielded_is_own_pool: false,
            same_account: Some(Core),
        };
        assert_eq!(
            resolve_source_auto(
                &own_core,
                Some(10),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            chosen(Platform, 30, vec![])
        );
    }

    #[test]
    fn an_unavailable_private_balance_is_skipped() {
        assert_eq!(
            resolve_source_auto(
                &own_pool(),
                Some(10),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            chosen(Platform, 30, vec![])
        );
        let other_wallet_pool = AutoDestination {
            kind: Some(AddressKind::Shielded),
            shielded_is_own_pool: false,
            same_account: None,
        };
        assert_eq!(
            resolve_source_auto(
                &other_wallet_pool,
                Some(10),
                &maxima(Ready(80), Ready(30), Ready(50))
            ),
            chosen(Shielded, 50, vec![])
        );
    }

    #[test]
    fn auto_never_spends_identity_credits() {
        let maxima = maxima(Ready(0), Ready(0), Unavailable);
        assert_eq!(maxima.get(Identity), Unavailable);
        for kind in [
            AddressKind::Core,
            AddressKind::Platform,
            AddressKind::Identity,
        ] {
            assert!(!candidate_order(&to(kind)).contains(&Identity));
        }
    }

    #[test]
    fn auto_max_takes_the_largest_single_balance() {
        assert_eq!(
            resolve_auto_max(
                &to(AddressKind::Platform),
                &maxima(Ready(80), Ready(30), Unavailable)
            ),
            AutoMax::Chosen {
                kind: Core,
                max: 80
            }
        );
        assert_eq!(
            resolve_auto_max(
                &to(AddressKind::Platform),
                &maxima(Ready(30), Ready(30), Unavailable)
            ),
            AutoMax::Chosen {
                kind: Platform,
                max: 30
            },
            "ties go to the earlier candidate"
        );
    }

    #[test]
    fn auto_max_waits_for_every_candidate_quote() {
        assert_eq!(
            resolve_auto_max(
                &to(AddressKind::Core),
                &maxima(Pending, Ready(30), Unavailable)
            ),
            AutoMax::Checking {
                pending: vec![Core]
            }
        );
        assert_eq!(
            resolve_auto_max(
                &to(AddressKind::Core),
                &maxima(Ready(0), Failed, Unavailable)
            ),
            AutoMax::QuoteFailed { kind: Platform }
        );
    }

    /// A candidate whose quote failed might hold the most, so Max is not
    /// chosen among the others until that quote is retried.
    #[test]
    fn auto_max_is_not_chosen_while_a_candidate_quote_has_failed() {
        let outcome = resolve_auto_max(
            &to(AddressKind::Core),
            &maxima(Ready(30), Failed, Unavailable),
        );
        assert_eq!(outcome, AutoMax::QuoteFailed { kind: Platform });
    }

    /// A held Core balance stops Auto instead of turning a Core payment
    /// into a Platform one.
    #[test]
    fn source_auto_waits_for_a_withheld_candidate() {
        let outcome = resolve_source_auto(
            &to(AddressKind::Core),
            Some(10),
            &maxima(Withheld, Ready(30), Unavailable),
        );
        assert_eq!(outcome, AutoSource::Withheld { kind: Core });
    }

    #[test]
    fn auto_max_is_not_chosen_while_a_candidate_is_withheld() {
        let outcome = resolve_auto_max(
            &to(AddressKind::Platform),
            &maxima(Withheld, Ready(30), Unavailable),
        );
        assert_eq!(outcome, AutoMax::Withheld { kind: Core });
    }

    #[test]
    fn destination_auto_uses_the_source_kind_in_another_wallet() {
        assert_eq!(resolve_destination_auto(Core, false), AddressKind::Core);
        assert_eq!(
            resolve_destination_auto(Platform, false),
            AddressKind::Platform
        );
        assert_eq!(
            resolve_destination_auto(Shielded, false),
            AddressKind::Shielded
        );
    }

    #[test]
    fn destination_auto_uses_the_other_kind_in_the_same_wallet() {
        assert_eq!(resolve_destination_auto(Core, true), AddressKind::Platform);
        assert_eq!(resolve_destination_auto(Platform, true), AddressKind::Core);
        assert_eq!(
            resolve_destination_auto(Shielded, true),
            AddressKind::Platform
        );
    }

    #[test]
    fn destination_auto_from_an_identity_is_a_platform_address() {
        assert_eq!(
            resolve_destination_auto(Identity, true),
            AddressKind::Platform
        );
        assert_eq!(
            resolve_destination_auto(Identity, false),
            AddressKind::Platform
        );
    }

    #[test]
    fn default_receive_kind_for_another_wallet_is_the_largest_balance() {
        assert_eq!(default_receive_kind(80, 30, false), AddressKind::Core);
        assert_eq!(default_receive_kind(30, 80, false), AddressKind::Platform);
        assert_eq!(
            default_receive_kind(30, 30, false),
            AddressKind::Core,
            "ties go to Core"
        );
    }

    #[test]
    fn default_receive_kind_for_the_same_wallet_is_the_other_kind() {
        assert_eq!(default_receive_kind(80, 30, true), AddressKind::Platform);
        assert_eq!(default_receive_kind(30, 80, true), AddressKind::Core);
        assert_eq!(default_receive_kind(0, 0, true), AddressKind::Platform);
    }

    #[test]
    fn unsupported_routes_are_refused() {
        assert_eq!(
            route_refusal(Identity, AddressKind::Shielded, true),
            Some(RouteRefusal::IdentityToShielded)
        );
        assert_eq!(
            route_refusal(Shielded, AddressKind::Identity, false),
            Some(RouteRefusal::ShieldedToIdentity)
        );
        for source in [Core, Platform] {
            assert_eq!(
                route_refusal(source, AddressKind::Shielded, false),
                Some(RouteRefusal::ShieldedNotOwnPool)
            );
            assert_eq!(route_refusal(source, AddressKind::Shielded, true), None);
        }
        assert_eq!(route_refusal(Shielded, AddressKind::Shielded, false), None);
        assert_eq!(route_refusal(Identity, AddressKind::Core, false), None);
    }
}
