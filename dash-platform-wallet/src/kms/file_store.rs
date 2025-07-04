use core::{cmp::Ord, fmt::Debug, prelude::v1::derive};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use super::KVStore;

/// JSON storage for Key Management Service (KMS).
///
/// This module provides a JSON-based storage implementation for managing cryptographic keys
/// and related operations in a Key Management Service (KMS).
///
/// It encrypts values before storing them and decrypts them when retrieving.
///
/// Keys are stored in plaintext.
///
/// Data is base64 encoded before storing and decoded after retrieving.
///
///
/// ## Cloning
///
/// The `JsonStore` is designed to be cloneable, allowing multiple instances to share the same underlying data.
#[derive(Clone)]
pub(super) struct FileStore<K: Ord, V> {
    // where
    // K: Clone + std::fmt::Debug + Serialize + for<'de> Deserialize<'de> + Ord,
    // V: Clone + Serialize + for<'de> Deserialize<'de>,
    path: PathBuf,
    db: Arc<RwLock<Database<K, V>>>,
}

impl<K: Ord, V> Debug for FileStore<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonStore")
            .field("path", &self.path)
            .finish()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Database<K: Ord, V> {
    records: BTreeMap<K, V>,
}

#[derive(Debug, thiserror::Error)]
pub enum JsonStoreError {
    #[error("I/O error: {0}")]
    IoError(String),
    #[error("JSON serialization/deserialization error: {0}")]
    JsonError(#[from] serde_json::Error),
}

impl From<std::io::Error> for JsonStoreError {
    fn from(err: std::io::Error) -> Self {
        JsonStoreError::IoError(err.to_string())
    }
}

impl<K, V> KVStore<K, V> for FileStore<K, V>
where
    K: Clone + std::fmt::Debug + Serialize + for<'de> Deserialize<'de> + Ord,
    V: Clone + Serialize + for<'de> Deserialize<'de>,
{
    type Error = JsonStoreError;

    fn get(&self, key: &K) -> Result<Option<V>, Self::Error> {
        let db = self.db.read().expect("Failed to read from JSON store");
        let record = db.records.get(key).cloned();
        Ok(record)
    }

    fn set(&mut self, key: K, value: V) -> Result<(), Self::Error> {
        let mut db = self.db.write().unwrap();
        db.records.insert(key, value);
        self.save(db.deref())?;
        Ok(())
    }

    fn delete(&mut self, key: &K) -> Result<bool, Self::Error> {
        let mut db = self.db.write().unwrap();
        let removed = db.records.remove(key).is_some();
        if removed {
            self.save(db.deref())?;
        }
        Ok(removed)
    }

    fn keys(&self) -> Result<Vec<K>, Self::Error> {
        let db = self.db.read().expect("Failed to read from JSON store");
        Ok(db.records.keys().cloned().collect())
    }

    fn contains_key(&self, key: &K) -> Result<bool, Self::Error> {
        let db = self.db.read().expect("Failed to read from JSON store");
        Ok(db.records.contains_key(key))
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        let mut db = self.db.write().unwrap();
        db.records.clear();
        self.save(db.deref())?;
        Ok(())
    }
}

impl<K, V> FileStore<K, V>
where
    K: Clone + std::fmt::Debug + Serialize + for<'de> Deserialize<'de> + Ord,
    V: Clone + Serialize + for<'de> Deserialize<'de>,
{
    /// Creates a new instance of `JsonStore`.
    pub fn new(path: &Path) -> Result<Self, JsonStoreError> {
        let me = Self {
            path: path.to_path_buf(),
            db: Arc::new(RwLock::new(Database {
                records: BTreeMap::new(),
            })),
        };
        if path.exists() {
            me.load()?;
        } else {
            // Create the file if it does not exist. We do this here to fail early if the file cannot be created.
            let db = me.db.read().unwrap();
            me.save(db.deref())?;
        }

        Ok(me)
    }

    /// Load database from the JSON store.
    fn load(&self) -> Result<(), JsonStoreError> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(false)
            .open(&self.path)?;

        let database: Database<K, V> = match serde_json::from_reader(&file) {
            Ok(db) => db,
            Err(e) => return Err(e.into()),
        };

        self.db.write().unwrap().records = database.records;

        Ok(())
    }

    /// Save current database to the JSON store.
    fn save(&self, db: &Database<K, V>) -> Result<(), JsonStoreError> {
        let json = serde_json::to_vec_pretty(db)?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        file.write_all(&json)?;
        Ok(())
    }
}
