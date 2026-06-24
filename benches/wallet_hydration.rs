//! Cold-start hydration benchmarks (T-PERF-01).
//!
//! Measures the per-network rehydration paths that replaced the legacy
//! `db.get_wallets` / `db.get_single_key_wallets` reads after the
//! `platform-wallet` migration:
//!
//! - `hydrate_hd_wallets` exercises
//!   [`hydrate_hd_wallets_from_views`](dash_evo_tool::wallet_backend::hydration::hydrate_hd_wallets_from_views),
//!   the free-function version of `WalletBackend::hydrate_wallets_for_network`
//!   (T-W-01). Setup writes N HD wallets through the
//!   [`WalletSeedView`] + [`WalletMetaView`] sidecars; the measured
//!   region opens a fresh `SqlitePersister` + `SecretStore` and walks
//!   them cold.
//!
//! - `hydrate_single_key_wallets` exercises
//!   [`SingleKeyView::hydrate_wallets`] +
//!   [`SingleKeyView::rehydrate_index`] (T-W-01b). Setup imports N
//!   single-key WIFs through `import_wif`; the measured region drops
//!   the in-memory index and replays both reads.
//!
//! Each iteration runs in a fresh `tempfile::tempdir()` so iterations
//! are hermetic — no real network, no shared paths.

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use dash_sdk::dpp::dashcore::{Network, PrivateKey};
use platform_wallet_storage::secrets::SecretStore;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};

use dash_evo_tool::model::wallet::Wallet;
use dash_evo_tool::model::wallet::meta::WalletMeta;
use dash_evo_tool::model::wallet::seed_envelope::StoredSeedEnvelope;
use dash_evo_tool::wallet_backend::hydration::hydrate_hd_wallets_from_views;
use dash_evo_tool::wallet_backend::single_key::{SingleKeyView, open_secret_store};
use dash_evo_tool::wallet_backend::wallet_meta::WalletMetaView;
use dash_evo_tool::wallet_backend::wallet_seed_store::WalletSeedView;
use dash_evo_tool::wallet_backend::{DetKv, KvAdapterError};

/// Wall-clock budget per measured sample. The 100-wallet case writes
/// the most sidecar rows at setup and benefits from the longer window;
/// the smaller cases finish well inside the default budget anyway.
const MEASUREMENT_TIME: Duration = Duration::from_secs(5);

/// Wallet counts the bench groups iterate over. The 100-case is the
/// upper end of what a single power-user install plausibly carries.
const WALLET_COUNTS: [usize; 3] = [1, 10, 100];

/// Bench network. Hydration is per-network so the choice is arbitrary
/// — Testnet matches the dev-loop everyone runs locally.
const BENCH_NETWORK: Network = Network::Testnet;

/// Build a unique 64-byte seed for the i-th wallet. `i` lands in bytes
/// 0..8 so every seed differs even when the high tail is constant.
fn seed_for(i: usize) -> [u8; 64] {
    let mut seed = [0u8; 64];
    seed[..8].copy_from_slice(&(i as u64).to_be_bytes());
    // Sprinkle the rest so successive entropy bytes are not all zero
    // (defensive — `Wallet::new_from_seed` derives an xpub regardless).
    for (idx, b) in seed.iter_mut().enumerate().skip(8) {
        *b = (idx as u8).wrapping_mul(31).wrapping_add(0x5A);
    }
    seed
}

/// Generate the i-th deterministic 32-byte secp256k1 secret. Bytes
/// 0..8 carry the index so each WIF derives a distinct P2PKH address.
/// The constant tail keeps the bytes inside the secp256k1 group order
/// — `PrivateKey::from_byte_array` rejects zero / out-of-range keys,
/// so we sprinkle non-zero data.
fn priv_key_bytes_for(i: usize) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
    for (idx, b) in bytes.iter_mut().enumerate().skip(8) {
        *b = (idx as u8).wrapping_mul(17).wrapping_add(0x33);
    }
    bytes
}

/// Open a fresh `DetKv` over a tempdir-backed `SqlitePersister`.
fn open_kv(dir: &std::path::Path) -> Arc<DetKv> {
    let cfg = SqlitePersisterConfig::new(dir.join("det-app.sqlite"));
    let persister = Arc::new(SqlitePersister::open(cfg).expect("open persister"));
    Arc::new(DetKv::new(persister))
}

/// Open a fresh upstream secret-store vault under `dir/secrets.pwsvault`.
fn open_store(dir: &std::path::Path) -> Arc<SecretStore> {
    let path = dir.join("secrets.pwsvault");
    Arc::new(open_secret_store(&path).expect("open secret store"))
}

/// Seed N HD wallets into the on-disk sidecars. Mirrors the writes the
/// import path performs: an envelope blob behind the vault per wallet
/// plus a `<network>:wallet_meta:<seed_hash>` row in the app k/v.
fn seed_hd_wallets(
    seed_view: &WalletSeedView<'_>,
    meta_view: &WalletMetaView<'_>,
    network: Network,
    count: usize,
) {
    for i in 0..count {
        let seed = seed_for(i);
        let wallet = Wallet::new_from_seed(seed, network, Some(format!("bench-{i}")), None)
            .expect("derive bench wallet");
        let xpub = wallet
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec();
        let envelope = StoredSeedEnvelope {
            encrypted_seed: seed.to_vec(),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: xpub.clone(),
        };
        let meta = WalletMeta {
            alias: format!("bench-{i}"),
            is_main: i == 0,
            core_wallet_name: None,
            xpub_encoded: xpub,
            uses_password: false,
            password_hint: None,
        };
        let seed_hash = wallet.seed_hash();
        seed_view.set(&seed_hash, &envelope).expect("set envelope");
        meta_view.set(network, &seed_hash, &meta).expect("set meta");
    }
}

/// Seed N single-key entries through `SingleKeyView::import_wif`. The
/// in-memory index is populated as a side-effect; callers that want to
/// measure cold hydration clear it explicitly.
fn seed_single_key_wallets(view: &SingleKeyView<'_>, count: usize) -> Result<(), KvAdapterError> {
    for i in 0..count {
        let bytes = priv_key_bytes_for(i);
        let priv_key =
            PrivateKey::from_byte_array(&bytes, BENCH_NETWORK).expect("valid secp256k1 secret");
        let wif = priv_key.to_wif();
        view.import_wif(&wif, Some(format!("bench-sk-{i}")))
            .expect("import wif");
    }
    Ok(())
}

fn bench_hydrate_hd_wallets(c: &mut Criterion) {
    let mut group = c.benchmark_group("hydrate_hd_wallets");
    group.measurement_time(MEASUREMENT_TIME);
    for &n in &WALLET_COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bencher, &n| {
            bencher.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    // Setup is intentionally outside the timer.
                    let dir = tempfile::tempdir().expect("tempdir");
                    let kv = open_kv(dir.path());
                    let store = open_store(dir.path());
                    {
                        let seed_view = WalletSeedView::new(&store);
                        let meta_view = WalletMetaView::new(&kv);
                        seed_hd_wallets(&seed_view, &meta_view, BENCH_NETWORK, n);
                    }
                    // Reopen everything so the measured region sees cold
                    // SQLite caches and a fresh upstream vault handle —
                    // the closest a bench can get to a real cold boot.
                    drop(kv);
                    drop(store);
                    let kv = open_kv(dir.path());
                    let store = open_store(dir.path());
                    let seed_view = WalletSeedView::new(&store);
                    let meta_view = WalletMetaView::new(&kv);

                    let start = Instant::now();
                    let out = hydrate_hd_wallets_from_views(&seed_view, &meta_view, BENCH_NETWORK)
                        .expect("hydrate hd wallets");
                    total += start.elapsed();
                    assert_eq!(out.len(), n, "hydration must yield every seeded wallet");
                }
                total
            });
        });
    }
    group.finish();
}

fn bench_hydrate_single_key_wallets(c: &mut Criterion) {
    let mut group = c.benchmark_group("hydrate_single_key_wallets");
    group.measurement_time(MEASUREMENT_TIME);
    for &n in &WALLET_COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bencher, &n| {
            bencher.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let dir = tempfile::tempdir().expect("tempdir");
                    let kv = open_kv(dir.path());
                    let store = open_store(dir.path());
                    let index = std::sync::RwLock::new(std::collections::BTreeMap::new());
                    {
                        let view =
                            SingleKeyView::from_views(&store, &index, BENCH_NETWORK, Some(&kv));
                        seed_single_key_wallets(&view, n).expect("seed");
                    }
                    drop(kv);
                    drop(store);

                    // Cold reopen of vault + k/v, with an empty index.
                    let kv = open_kv(dir.path());
                    let store = open_store(dir.path());
                    let cold_index = std::sync::RwLock::new(std::collections::BTreeMap::new());
                    let view =
                        SingleKeyView::from_views(&store, &cold_index, BENCH_NETWORK, Some(&kv));

                    let start = Instant::now();
                    let wallets = view.hydrate_wallets();
                    view.rehydrate_index().expect("rehydrate index");
                    total += start.elapsed();
                    assert_eq!(wallets.len(), n, "every imported key must rehydrate");
                }
                total
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_hydrate_hd_wallets,
    bench_hydrate_single_key_wallets,
);
criterion_main!(benches);
