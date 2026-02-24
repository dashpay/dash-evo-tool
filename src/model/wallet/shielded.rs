use dash_sdk::dpp::dashcore::Network;
use dash_sdk::grovedb_commitment_tree::RandomSeed;
use dash_sdk::grovedb_commitment_tree::{
    ClientPersistentCommitmentTree, FullViewingKey, IncomingViewingKey, Note, NoteValue, Nullifier,
    OutgoingViewingKey, PaymentAddress, Position, Rho, Scope, SpendAuthorizingKey, SpendingKey,
};
use std::sync::Mutex;
use zip32::AccountId;

/// Dash coin types per BIP44
const DASH_COIN_TYPE_MAINNET: u32 = 5;
const DASH_COIN_TYPE_TESTNET: u32 = 1;

/// Orchard key set derived from a wallet seed via ZIP32.
///
/// ZIP32 derivation path: `m / 32' / coin_type' / account'`
/// where `purpose = 32` is the ZIP number for Orchard key derivation.
pub struct OrchardKeySet {
    pub sk: SpendingKey,
    pub fvk: FullViewingKey,
    pub ask: SpendAuthorizingKey,
    pub ivk: IncomingViewingKey,
    pub ovk: OutgoingViewingKey,
    pub default_address: PaymentAddress,
}

/// Derive Orchard keys from an HD wallet seed using ZIP32.
///
/// The seed can be 32-252 bytes. We pass the wallet's 64-byte BIP39 seed directly.
/// `SpendingKey::from_zip32_seed()` internally derives
/// `ExtendedSpendingKey::from_path(seed, &[32', coin_type', account'])`.
pub fn derive_orchard_keys(
    seed_bytes: &[u8],
    network: Network,
    account: u32,
) -> Result<OrchardKeySet, String> {
    let coin_type = match network {
        Network::Dash => DASH_COIN_TYPE_MAINNET,
        _ => DASH_COIN_TYPE_TESTNET,
    };
    let account_id =
        AccountId::try_from(account).map_err(|_| "Invalid account index".to_string())?;

    let sk = SpendingKey::from_zip32_seed(seed_bytes, coin_type, account_id)
        .map_err(|e| format!("ZIP32 derivation failed: {e}"))?;

    let fvk = FullViewingKey::from(&sk);
    let ask = SpendAuthorizingKey::from(&sk);
    let ivk = fvk.to_ivk(Scope::External);
    let ovk = fvk.to_ovk(Scope::External);
    let default_address = fvk.address_at(0u32, Scope::External);

    Ok(OrchardKeySet {
        sk,
        fvk,
        ask,
        ivk,
        ovk,
        default_address,
    })
}

/// A decrypted shielded note owned by the wallet.
pub struct ShieldedNote {
    /// The decrypted Orchard note
    pub note: Note,
    /// Position in the commitment tree (global index)
    pub position: Position,
    /// Extracted note commitment bytes
    pub cmx: [u8; 32],
    /// Nullifier for detecting when spent
    pub nullifier: Nullifier,
    /// Block height where the note appeared
    pub block_height: u64,
    /// Whether the nullifier was seen on-chain
    pub is_spent: bool,
    /// Note value in credits (cached from note.value().inner())
    pub value: u64,
}

/// Per-wallet shielded state, initialized lazily when the shielded tab is first opened.
///
/// Manual `Debug` impl because `ClientPersistentCommitmentTree` does not implement `Debug`.
pub struct ShieldedWalletState {
    /// ZIP32-derived Orchard keys (requires wallet to be unlocked)
    pub keys: OrchardKeySet,
    /// All tracked shielded notes
    pub notes: Vec<ShieldedNote>,
    /// Client-side Sinsemilla commitment tree for witness generation (SQLite-backed).
    ///
    /// Wrapped in `Mutex` because `ClientPersistentCommitmentTree` contains a
    /// SQLite `Connection` which is `!Sync`. The mutex makes
    /// `ShieldedWalletState` `Sync` so `&ShieldedWalletState` is `Send` across
    /// `.await` points in tokio tasks.
    pub commitment_tree: Mutex<ClientPersistentCommitmentTree>,
    /// Last note global position synced from platform
    pub last_synced_index: u64,
    /// Block height up to which nullifier sync has been completed
    pub last_nullifier_sync_height: u64,
    /// Timestamp of last nullifier sync
    pub last_nullifier_sync_timestamp: u64,
    /// Sum of unspent note values (cached)
    pub shielded_balance: u64,
    /// When notes were last synced (in-memory only, resets on app restart)
    pub last_notes_synced_at: Option<std::time::Instant>,
    /// When nullifiers were last checked (in-memory only, resets on app restart)
    pub last_nullifiers_synced_at: Option<std::time::Instant>,
}

impl std::fmt::Debug for ShieldedWalletState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShieldedWalletState")
            .field("notes_count", &self.notes.len())
            .field("last_synced_index", &self.last_synced_index)
            .field(
                "last_nullifier_sync_height",
                &self.last_nullifier_sync_height,
            )
            .field("shielded_balance", &self.shielded_balance)
            .finish_non_exhaustive()
    }
}

impl ShieldedWalletState {
    /// Recalculate the cached shielded balance from unspent notes.
    pub fn recalculate_balance(&mut self) {
        self.shielded_balance = self
            .notes
            .iter()
            .filter(|n| !n.is_spent)
            .map(|n| n.value)
            .sum();
    }

    /// Get unspent notes.
    pub fn unspent_notes(&self) -> Vec<&ShieldedNote> {
        self.notes.iter().filter(|n| !n.is_spent).collect()
    }
}

/// Note serialization format (115 bytes):
/// `recipient(43) || value(8 LE) || rho(32) || rseed(32)`
const SERIALIZED_NOTE_LEN: usize = 43 + 8 + 32 + 32;

/// Serialize an Orchard Note to 115 bytes for database persistence.
pub fn serialize_note(note: &Note) -> Vec<u8> {
    let mut data = Vec::with_capacity(SERIALIZED_NOTE_LEN);
    data.extend_from_slice(&note.recipient().to_raw_address_bytes());
    data.extend_from_slice(&note.value().inner().to_le_bytes());
    data.extend_from_slice(&note.rho().to_bytes());
    data.extend_from_slice(note.rseed().as_bytes());
    data
}

/// Deserialize an Orchard Note from 115 bytes.
pub fn deserialize_note(data: &[u8]) -> Option<Note> {
    if data.len() != SERIALIZED_NOTE_LEN {
        return None;
    }

    let recipient_bytes: [u8; 43] = data[0..43].try_into().ok()?;
    let recipient = PaymentAddress::from_raw_address_bytes(&recipient_bytes).into_option()?;

    let value_bytes: [u8; 8] = data[43..51].try_into().ok()?;
    let value = NoteValue::from_raw(u64::from_le_bytes(value_bytes));

    let rho_bytes: [u8; 32] = data[51..83].try_into().ok()?;
    let rho = Rho::from_bytes(&rho_bytes).into_option()?;

    let rseed_bytes: [u8; 32] = data[83..115].try_into().ok()?;
    let rseed = RandomSeed::from_bytes(rseed_bytes, &rho).into_option()?;

    Note::from_parts(recipient, value, rho, rseed).into_option()
}
