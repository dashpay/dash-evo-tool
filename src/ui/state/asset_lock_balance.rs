//! Per-screen cache of live asset-lock builder maximum amounts.

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::WalletSeedHash;
use crate::wallet_backend::AssetLockInputState;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const LOADING_VALIDATION_MESSAGE: &str =
    "Your wallet's available amount is still being checked. Wait a moment and try again.";
const FAILED_VALIDATION_MESSAGE: &str =
    "The available amount could not be checked. Use Retry and try again.";
const ASSET_LOCK_REQUEST_DEADLINE: Duration = Duration::from_secs(15);
/// Consecutive accepted replies whose observed composition failed to match the
/// dispatched key before automatic re-dispatch stops. The first mismatch gets
/// one automatic re-probe (it may be transient, e.g. an observation-deadline
/// expiry under momentary lock contention); a persistent divergence then waits
/// for a composition change or an explicit Retry.
const MAX_CONSECUTIVE_MISMATCHED_REPLIES: u8 = 2;

#[derive(Clone)]
struct RequestKey {
    generation: u64,
    inputs: AssetLockInputState,
    revision: u64,
}

impl RequestKey {
    fn matches(&self, generation: u64, inputs: &AssetLockInputState, revision: u64) -> bool {
        self.generation == generation && self.inputs == *inputs && self.revision == revision
    }

    fn matches_composition(&self, inputs: &AssetLockInputState, revision: u64) -> bool {
        self.inputs == *inputs && self.revision == revision
    }
}

struct InFlight {
    request_id: u64,
    key: RequestKey,
    started_at: Instant,
}

struct LoadedQuote {
    observed_inputs: AssetLockInputState,
    amount_duffs: u64,
}

struct FailedRequest {
    key: RequestKey,
}

/// Preserves result ordering within one generation sequence without making old
/// high-water marks or unresolved requests permanent dispatch barriers.
struct FetchState {
    request_key: RequestKey,
    loaded: Option<LoadedQuote>,
    in_flight: Option<InFlight>,
    failed: Option<FailedRequest>,
    retry_available: bool,
    /// Consecutive replies for the current composition whose observed inputs
    /// did not match it — see [`MAX_CONSECUTIVE_MISMATCHED_REPLIES`].
    mismatched_replies: u8,
}

/// Async fetch state for asset-lock maximum amounts, keyed by wallet.
#[derive(Default)]
pub struct AssetLockBalanceCache {
    states: BTreeMap<WalletSeedHash, FetchState>,
    next_request_id: u64,
}

impl AssetLockBalanceCache {
    /// Dispatch at most one live-builder query per relevant wallet snapshot.
    ///
    /// A final-funds or eligible-UTXO-composition change supersedes unresolved
    /// work. Irrelevant generation churn keeps the existing request, while a
    /// lower generation starts a fresh sequence after a counter reset. Replies
    /// that persistently cannot match the published composition stop automatic
    /// re-dispatch ([`MAX_CONSECUTIVE_MISMATCHED_REPLIES`]) until the
    /// composition changes or [`Self::invalidate_one`] re-arms the wallet.
    pub fn ensure_requested(
        &mut self,
        seed_hash: WalletSeedHash,
        snapshot_generation: u64,
        inputs: AssetLockInputState,
        utxo_revision: u64,
    ) -> Option<BackendTask> {
        self.ensure_requested_at(
            seed_hash,
            snapshot_generation,
            inputs,
            utxo_revision,
            Instant::now(),
        )
    }

    fn ensure_requested_at(
        &mut self,
        seed_hash: WalletSeedHash,
        snapshot_generation: u64,
        inputs: AssetLockInputState,
        utxo_revision: u64,
        now: Instant,
    ) -> Option<BackendTask> {
        let state = self.states.entry(seed_hash).or_insert(FetchState {
            request_key: RequestKey {
                generation: snapshot_generation,
                inputs: inputs.clone(),
                revision: utxo_revision,
            },
            loaded: None,
            in_flight: None,
            failed: None,
            retry_available: false,
            mismatched_replies: 0,
        });
        let generation_restarted = snapshot_generation < state.request_key.generation;
        let input_composition_changed =
            inputs != state.request_key.inputs || utxo_revision != state.request_key.revision;
        if generation_restarted || input_composition_changed {
            state.request_key = RequestKey {
                generation: snapshot_generation,
                inputs: inputs.clone(),
                revision: utxo_revision,
            };
            state.in_flight = None;
            state.failed = None;
            state.retry_available = false;
            state.mismatched_replies = 0;
            if generation_restarted {
                state.loaded = None;
            }
        }

        let in_flight_expired = state.in_flight.as_ref().is_some_and(|in_flight| {
            in_flight.key.matches_composition(&inputs, utxo_revision)
                && now
                    .checked_duration_since(in_flight.started_at)
                    .is_some_and(|elapsed| elapsed >= ASSET_LOCK_REQUEST_DEADLINE)
        });
        if in_flight_expired {
            state.in_flight = None;
            state.retry_available = true;
            state.request_key = RequestKey {
                generation: snapshot_generation,
                inputs: inputs.clone(),
                revision: utxo_revision,
            };
        }

        if state
            .in_flight
            .as_ref()
            .is_some_and(|in_flight| in_flight.key.matches_composition(&inputs, utxo_revision))
            || state
                .loaded
                .as_ref()
                .is_some_and(|loaded| loaded.observed_inputs == inputs)
            || state.failed.as_ref().is_some_and(|failed| {
                failed
                    .key
                    .matches(snapshot_generation, &inputs, utxo_revision)
            })
            // Composition-keyed (not generation-keyed) on purpose: SPV event
            // churn republishes the same composition under new generations and
            // must not re-arm a probe that cannot match it.
            || state.mismatched_replies >= MAX_CONSECUTIVE_MISMATCHED_REPLIES
        {
            return None;
        }
        let request_id = self.next_request_id.checked_add(1)?;
        self.next_request_id = request_id;
        state.request_key = RequestKey {
            generation: snapshot_generation,
            inputs: inputs.clone(),
            revision: utxo_revision,
        };
        state.in_flight = Some(InFlight {
            request_id,
            key: RequestKey {
                generation: snapshot_generation,
                inputs,
                revision: utxo_revision,
            },
            started_at: now,
        });
        state.failed = None;
        Some(BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
            seed_hash,
            snapshot_generation,
            request_id,
        }))
    }

    /// Store the maximum returned by the live wallet backend.
    pub fn store(
        &mut self,
        seed_hash: WalletSeedHash,
        snapshot_generation: u64,
        request_id: u64,
        amount_duffs: u64,
        observed_inputs: AssetLockInputState,
        is_partial: bool,
    ) {
        let Some(state) = self.states.get_mut(&seed_hash) else {
            return;
        };
        if let Some(in_flight) = state.in_flight.as_ref()
            && (in_flight.request_id, in_flight.key.generation) == (request_id, snapshot_generation)
        {
            let in_flight_key = in_flight.key.clone();
            state.in_flight = None;
            if state.request_key.matches(
                in_flight_key.generation,
                &in_flight_key.inputs,
                in_flight_key.revision,
            ) {
                let mismatched = observed_inputs != in_flight_key.inputs;
                state.loaded = Some(LoadedQuote {
                    observed_inputs,
                    amount_duffs,
                });
                if mismatched {
                    state.mismatched_replies = state.mismatched_replies.saturating_add(1);
                    if state.mismatched_replies >= MAX_CONSECUTIVE_MISMATCHED_REPLIES {
                        // Surface the dead end as a failed check so the UI
                        // offers Retry instead of an indefinite loading state.
                        state.failed = Some(FailedRequest { key: in_flight_key });
                    }
                } else {
                    state.mismatched_replies = 0;
                    state.failed = None;
                }
                state.retry_available = is_partial || mismatched;
            }
        }
    }

    /// Mark one in-flight wallet query retryable after a backend failure.
    pub fn mark_loading_failed(
        &mut self,
        seed_hash: &WalletSeedHash,
        snapshot_generation: u64,
        request_id: u64,
    ) {
        let Some(state) = self.states.get_mut(seed_hash) else {
            return;
        };
        if let Some(in_flight) = state.in_flight.as_ref()
            && (in_flight.request_id, in_flight.key.generation) == (request_id, snapshot_generation)
        {
            let in_flight_key = in_flight.key.clone();
            state.in_flight = None;
            if state.request_key.matches(
                in_flight_key.generation,
                &in_flight_key.inputs,
                in_flight_key.revision,
            ) {
                state.failed = Some(FailedRequest { key: in_flight_key });
                state.retry_available = true;
            }
        }
    }

    /// Return the most recent builder maximum, including during revalidation.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Option<u64> {
        self.states
            .get(seed_hash)
            .and_then(|state| state.loaded.as_ref().map(|loaded| loaded.amount_duffs))
    }

    /// Return a quote only when it matches the current validation inputs.
    pub fn get_current(
        &self,
        seed_hash: &WalletSeedHash,
        inputs: &AssetLockInputState,
    ) -> Option<u64> {
        self.states.get(seed_hash).and_then(|state| {
            state
                .loaded
                .as_ref()
                .filter(|loaded| loaded.observed_inputs == *inputs)
                .map(|loaded| loaded.amount_duffs)
        })
    }

    /// Whether the query failed and needs an explicit retry.
    pub fn is_failed(&self, seed_hash: &WalletSeedHash) -> bool {
        self.states.get(seed_hash).is_some_and(|state| {
            state.failed.as_ref().is_some_and(|failed| {
                failed.key.matches(
                    state.request_key.generation,
                    &state.request_key.inputs,
                    state.request_key.revision,
                )
            })
        })
    }

    /// Whether the current loading/partial state should show a Retry button.
    pub fn should_offer_retry(&self, seed_hash: &WalletSeedHash) -> bool {
        self.is_failed(seed_hash)
            || self
                .states
                .get(seed_hash)
                .is_some_and(|state| state.retry_available)
    }

    /// Explain why validation cannot use a builder quote yet.
    pub fn validation_unavailable_message(&self, seed_hash: &WalletSeedHash) -> &'static str {
        if self.is_failed(seed_hash) {
            FAILED_VALIDATION_MESSAGE
        } else {
            LOADING_VALIDATION_MESSAGE
        }
    }

    /// Re-arm one wallet's query.
    pub fn invalidate_one(&mut self, seed_hash: &WalletSeedHash) {
        self.states.remove(seed_hash);
    }

    /// Re-arm all wallet queries after a refresh or context change.
    pub fn invalidate(&mut self) {
        self.states.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::AssetLockBalanceCache;
    use crate::backend_task::BackendTask;
    use crate::backend_task::wallet::WalletTask;
    use crate::wallet_backend::AssetLockInputState;
    use crate::wallet_backend::DetWalletBalance;
    use dash_sdk::dpp::dashcore::{OutPoint, Txid};
    use std::time::{Duration, Instant};

    fn input_state(byte: u8, value: u64) -> AssetLockInputState {
        AssetLockInputState::from_inputs([(OutPoint::new(Txid::from([byte; 32]), 0), value)])
    }

    fn snapshot_inputs(final_funds_duffs: u64, utxo_revision: u64) -> AssetLockInputState {
        input_state(utxo_revision as u8, final_funds_duffs)
    }

    fn store(
        cache: &mut AssetLockBalanceCache,
        seed_hash: [u8; 32],
        snapshot_generation: u64,
        request_id: u64,
        amount_duffs: u64,
        final_funds_duffs: u64,
        utxo_revision: u64,
    ) {
        cache.store(
            seed_hash,
            snapshot_generation,
            request_id,
            amount_duffs,
            snapshot_inputs(final_funds_duffs, utxo_revision),
            false,
        );
    }

    fn request(
        cache: &mut AssetLockBalanceCache,
        seed_hash: [u8; 32],
        snapshot_generation: u64,
        final_funds_duffs: u64,
        utxo_revision: u64,
    ) -> u64 {
        match cache.ensure_requested(
            seed_hash,
            snapshot_generation,
            snapshot_inputs(final_funds_duffs, utxo_revision),
            utxo_revision,
        ) {
            Some(BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
                seed_hash: requested_seed,
                snapshot_generation: requested_generation,
                request_id,
            })) => {
                assert_eq!(requested_seed, seed_hash);
                assert_eq!(requested_generation, snapshot_generation);
                request_id
            }
            other => panic!("expected asset-lock maximum request, got {other:?}"),
        }
    }

    #[test]
    fn asset_lock_balance_cache_requeries_and_rejects_stale_results_after_snapshot_change() {
        let seed_hash = [0x29; 32];
        let mut cache = AssetLockBalanceCache::default();

        let request_7 = request(&mut cache, seed_hash, 7, 1_000, 1);
        store(&mut cache, seed_hash, 7, request_7, 1_000, 1_000, 1);
        assert_eq!(cache.get(&seed_hash), Some(1_000));

        let request_8 = request(&mut cache, seed_hash, 8, 900, 2);
        assert_eq!(
            cache.get(&seed_hash),
            Some(1_000),
            "the last loaded value must remain displayable while generation 8 refreshes"
        );
        assert!(
            cache
                .ensure_requested(seed_hash, 8, snapshot_inputs(900, 2), 2)
                .is_none(),
            "an in-flight refresh for the current generation must not dispatch twice"
        );

        store(&mut cache, seed_hash, 7, request_7, 1_000, 1_000, 1);
        assert_eq!(
            cache.get(&seed_hash),
            Some(1_000),
            "a stale response must not overwrite or erase the last displayable value"
        );

        store(&mut cache, seed_hash, 8, request_8, 900, 900, 2);
        assert_eq!(cache.get(&seed_hash), Some(900));
    }

    #[test]
    fn asset_lock_balance_cache_recovers_after_snapshot_generation_regression() {
        let seed_hash = [0x2a; 32];
        let mut cache = AssetLockBalanceCache::default();

        let old_request = request(&mut cache, seed_hash, 7, 1_000, 1);
        store(&mut cache, seed_hash, 7, old_request, 1_000, 1_000, 1);
        assert_eq!(cache.get(&seed_hash), Some(1_000));

        let restarted_request = request(&mut cache, seed_hash, 2, 1_000, 1);
        assert_eq!(
            cache.get(&seed_hash),
            None,
            "a restarted generation sequence must discard data from the previous sequence"
        );
        assert!(
            cache
                .ensure_requested(seed_hash, 2, snapshot_inputs(1_000, 1), 1)
                .is_none(),
            "the replacement request must still deduplicate its own generation"
        );

        store(&mut cache, seed_hash, 7, old_request, 2_000, 1_000, 1);
        assert_eq!(
            cache.get(&seed_hash),
            None,
            "a late result from before the generation restart must be ignored"
        );
        store(&mut cache, seed_hash, 2, restarted_request, 800, 1_000, 1);
        assert_eq!(cache.get(&seed_hash), Some(800));
    }

    #[test]
    fn asset_lock_balance_cache_supersedes_stuck_in_flight_request_on_newer_snapshot() {
        let seed_hash = [0x2b; 32];
        let mut cache = AssetLockBalanceCache::default();

        let old_request = request(&mut cache, seed_hash, 4, 1_000, 1);
        let current_request = request(&mut cache, seed_hash, 5, 900, 2);
        assert!(
            cache
                .ensure_requested(seed_hash, 5, snapshot_inputs(900, 2), 2)
                .is_none(),
            "the superseding request must deduplicate its own generation"
        );

        store(&mut cache, seed_hash, 4, old_request, 1_000, 1_000, 1);
        assert_eq!(
            cache.get(&seed_hash),
            None,
            "the superseded request must not populate the cache"
        );
        store(&mut cache, seed_hash, 5, current_request, 900, 900, 2);
        assert_eq!(cache.get(&seed_hash), Some(900));
    }

    #[test]
    fn asset_lock_balance_cache_ignores_generation_change_when_spendable_is_unchanged() {
        let seed_hash = [0x2c; 32];
        let mut cache = AssetLockBalanceCache::default();

        let request_id = request(&mut cache, seed_hash, 7, 1_000, 1);
        assert!(
            cache
                .ensure_requested(seed_hash, 8, snapshot_inputs(1_000, 1), 1)
                .is_none(),
            "a generation-only change must not restart the live-builder probe"
        );

        store(&mut cache, seed_hash, 7, request_id, 900, 1_000, 1);
        assert_eq!(
            cache.get(&seed_hash),
            Some(900),
            "the original request must remain current after a generation-only change"
        );
    }

    #[test]
    fn asset_lock_balance_cache_requeries_when_spendable_changes() {
        let seed_hash = [0x2d; 32];
        let mut cache = AssetLockBalanceCache::default();

        request(&mut cache, seed_hash, 7, 1_000, 1);
        request(&mut cache, seed_hash, 8, 1_500, 2);
    }

    #[test]
    fn asset_lock_balance_cache_requeries_when_unconfirmed_funds_become_final() {
        let seed_hash = [0x2e; 32];
        let mut cache = AssetLockBalanceCache::default();
        let unconfirmed = DetWalletBalance {
            confirmed: 0,
            unconfirmed: 1_000,
            total: 1_000,
        };
        let confirmed = DetWalletBalance {
            confirmed: 1_000,
            unconfirmed: 0,
            total: 1_000,
        };

        assert_eq!(unconfirmed.spendable(), confirmed.spendable());
        let unconfirmed_request = request(&mut cache, seed_hash, 7, unconfirmed.confirmed, 1);
        store(
            &mut cache,
            seed_hash,
            7,
            unconfirmed_request,
            0,
            unconfirmed.confirmed,
            1,
        );

        request(&mut cache, seed_hash, 8, confirmed.confirmed, 2);
    }

    #[test]
    fn asset_lock_balance_cache_requeries_same_generation_after_loaded_signal_changes() {
        let seed_hash = [0x2f; 32];
        let mut cache = AssetLockBalanceCache::default();

        let first_request = request(&mut cache, seed_hash, 7, 1_000, 1);
        store(&mut cache, seed_hash, 7, first_request, 900, 1_000, 1);
        assert_eq!(cache.get(&seed_hash), Some(900));

        let replacement_request = request(&mut cache, seed_hash, 7, 1_500, 2);
        store(
            &mut cache,
            seed_hash,
            7,
            replacement_request,
            1_400,
            1_500,
            2,
        );
        assert_eq!(cache.get(&seed_hash), Some(1_400));
        assert!(
            cache
                .ensure_requested(seed_hash, 7, snapshot_inputs(1_500, 2), 2)
                .is_none(),
            "the replacement result must deduplicate its own generation and signal"
        );
    }

    #[test]
    fn asset_lock_balance_cache_rejects_superseded_reply_at_same_generation() {
        let seed_hash = [0x31; 32];
        let mut cache = AssetLockBalanceCache::default();

        let request_t1 = request(&mut cache, seed_hash, 7, 1_000, 1);
        let request_t2 = request(&mut cache, seed_hash, 7, 1_500, 2);
        assert!(
            request_t2 > request_t1,
            "every actual dispatch must advance the request ID"
        );

        store(&mut cache, seed_hash, 7, request_t1, 900, 1_000, 1);
        assert_eq!(
            cache.get_current(&seed_hash, &snapshot_inputs(1_500, 2)),
            None,
            "T1's stale reply must not be tagged as T2's current quote"
        );
        cache.mark_loading_failed(&seed_hash, 7, request_t1);
        assert!(
            !cache.is_failed(&seed_hash),
            "T1's stale failure must not mark T2 as failed"
        );
        store(&mut cache, seed_hash, 7, request_t2, 1_400, 1_500, 2);
        assert_eq!(
            cache.get_current(&seed_hash, &snapshot_inputs(1_500, 2)),
            Some(1_400)
        );
    }

    #[test]
    fn asset_lock_balance_cache_blocks_stale_validation_after_utxo_composition_change() {
        let seed_hash = [0x30; 32];
        let mut cache = AssetLockBalanceCache::default();

        let stale_request = request(&mut cache, seed_hash, 7, 1_000, 1);
        store(&mut cache, seed_hash, 7, stale_request, 900, 1_000, 1);
        assert_eq!(cache.get(&seed_hash), Some(900));

        let current_request = request(&mut cache, seed_hash, 8, 1_000, 2);
        assert_eq!(
            cache.get(&seed_hash),
            Some(900),
            "stale-while-revalidate must keep the prior quote displayable"
        );
        assert_eq!(
            cache.get_current(&seed_hash, &snapshot_inputs(1_000, 2)),
            None,
            "validation must not use the stale higher quote for the new composition"
        );

        store(&mut cache, seed_hash, 8, current_request, 700, 1_000, 2);
        assert_eq!(
            cache.get_current(&seed_hash, &snapshot_inputs(1_000, 2)),
            Some(700)
        );
    }

    #[test]
    fn asset_lock_balance_cache_validates_the_observed_not_dispatched_composition() {
        let seed_hash = [0x32; 32];
        let mut cache = AssetLockBalanceCache::default();
        let dispatched = input_state(1, 1_000);
        let observed = input_state(2, 900);

        let request_id = cache
            .ensure_requested(seed_hash, 7, dispatched.clone(), 1)
            .and_then(|task| match task {
                BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
                    request_id, ..
                }) => Some(request_id),
                _ => None,
            })
            .expect("asset-lock maximum request");
        cache.store(seed_hash, 7, request_id, 800, observed.clone(), false);

        assert_eq!(
            cache.get_current(&seed_hash, &dispatched),
            None,
            "dispatch-time bookkeeping must not validate a quote measured against other inputs"
        );
        assert_eq!(cache.get_current(&seed_hash, &observed), Some(800));
    }

    #[test]
    fn asset_lock_balance_cache_redispatches_and_offers_retry_after_reply_deadline() {
        let seed_hash = [0x33; 32];
        let mut cache = AssetLockBalanceCache::default();
        let inputs = input_state(3, 1_000);
        let started_at = Instant::now();

        let first = cache
            .ensure_requested_at(seed_hash, 7, inputs.clone(), 1, started_at)
            .expect("initial request");
        assert!(
            cache
                .ensure_requested_at(
                    seed_hash,
                    7,
                    inputs.clone(),
                    1,
                    started_at + Duration::from_secs(1),
                )
                .is_none(),
            "a live request must still deduplicate"
        );

        let after_deadline =
            started_at + super::ASSET_LOCK_REQUEST_DEADLINE + Duration::from_nanos(1);
        let replacement = cache
            .ensure_requested_at(seed_hash, 8, inputs, 1, after_deadline)
            .expect("expired request must be replaced");

        assert_ne!(
            asset_lock_request_id(first),
            asset_lock_request_id(replacement),
            "the replacement must have a fresh request ID"
        );
        assert!(
            cache.should_offer_retry(&seed_hash),
            "the loading UI must expose Retry after a reply deadline expires"
        );
    }

    /// Dispatch for `published` at generation 7 / revision 1 and reply with the
    /// fail-closed empty marker, which can never match a non-empty composition.
    fn mismatched_reply_round(
        cache: &mut AssetLockBalanceCache,
        seed_hash: [u8; 32],
        published: &AssetLockInputState,
    ) {
        let request_id = cache
            .ensure_requested(seed_hash, 7, published.clone(), 1)
            .map(asset_lock_request_id)
            .expect("mismatch round must dispatch");
        cache.store(
            seed_hash,
            7,
            request_id,
            0,
            AssetLockInputState::default(),
            true,
        );
    }

    #[test]
    fn asset_lock_balance_cache_stops_redispatching_after_persistent_composition_mismatch() {
        let seed_hash = [0x38; 32];
        let mut cache = AssetLockBalanceCache::default();
        let published = input_state(6, 1_000);

        mismatched_reply_round(&mut cache, seed_hash, &published);
        // The second round's own dispatch `expect` asserts the first mismatch
        // still allows one automatic re-probe.
        mismatched_reply_round(&mut cache, seed_hash, &published);

        assert!(
            cache
                .ensure_requested(seed_hash, 7, published.clone(), 1)
                .is_none(),
            "a second consecutive mismatched reply must stop the automatic re-dispatch loop"
        );
        assert!(
            cache
                .ensure_requested(seed_hash, 9, published.clone(), 1)
                .is_none(),
            "generation-only churn must not re-arm a suppressed mismatch"
        );
        assert!(
            cache.is_failed(&seed_hash),
            "a persistent mismatch must surface as a failed check, not eternal loading"
        );
        assert!(
            cache.should_offer_retry(&seed_hash),
            "the suppressed state must expose an explicit Retry"
        );
        assert!(
            cache
                .ensure_requested(seed_hash, 10, input_state(7, 900), 2)
                .is_some(),
            "a real composition change must re-arm the probe"
        );
    }

    #[test]
    fn asset_lock_balance_cache_retry_rearms_a_mismatch_suppressed_wallet() {
        let seed_hash = [0x39; 32];
        let mut cache = AssetLockBalanceCache::default();
        let published = input_state(8, 1_000);

        mismatched_reply_round(&mut cache, seed_hash, &published);
        mismatched_reply_round(&mut cache, seed_hash, &published);
        assert!(
            cache
                .ensure_requested(seed_hash, 7, published.clone(), 1)
                .is_none(),
            "the mismatch loop must be suppressed before Retry"
        );

        cache.invalidate_one(&seed_hash);
        assert!(
            cache.ensure_requested(seed_hash, 7, published, 1).is_some(),
            "Retry (invalidate_one) must re-arm a mismatch-suppressed wallet"
        );
    }

    #[test]
    fn invalidate_one_rearms_only_the_selected_wallet() {
        let first_seed = [0x34; 32];
        let second_seed = [0x35; 32];
        let mut cache = AssetLockBalanceCache::default();
        let inputs = input_state(4, 1_000);

        assert!(
            cache
                .ensure_requested(first_seed, 1, inputs.clone(), 1)
                .is_some()
        );
        assert!(
            cache
                .ensure_requested(second_seed, 1, inputs.clone(), 1)
                .is_some()
        );
        cache.invalidate_one(&first_seed);

        assert!(
            cache
                .ensure_requested(first_seed, 1, inputs.clone(), 1)
                .is_some(),
            "the invalidated wallet must dispatch again"
        );
        assert!(
            cache.ensure_requested(second_seed, 1, inputs, 1).is_none(),
            "other wallet state must remain deduplicated"
        );
    }

    #[test]
    fn invalidate_rearms_every_wallet() {
        let first_seed = [0x36; 32];
        let second_seed = [0x37; 32];
        let mut cache = AssetLockBalanceCache::default();
        let inputs = input_state(5, 1_000);

        assert!(
            cache
                .ensure_requested(first_seed, 1, inputs.clone(), 1)
                .is_some()
        );
        assert!(
            cache
                .ensure_requested(second_seed, 1, inputs.clone(), 1)
                .is_some()
        );
        cache.invalidate();

        assert!(
            cache
                .ensure_requested(first_seed, 1, inputs.clone(), 1)
                .is_some()
        );
        assert!(cache.ensure_requested(second_seed, 1, inputs, 1).is_some());
    }

    fn asset_lock_request_id(task: BackendTask) -> u64 {
        match task {
            BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount { request_id, .. }) => {
                request_id
            }
            other => panic!("expected asset-lock maximum request, got {other:?}"),
        }
    }
}
