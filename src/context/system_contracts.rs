//! The system data contracts, as defined at the protocol version in use.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use arc_swap::ArcSwap;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::DataContract;

/// The system contracts DET uses, all loaded at one protocol version.
#[derive(Debug)]
pub(crate) struct SystemContracts {
    protocol_version: u32,
    pub(crate) dpns: Arc<DataContract>,
    pub(crate) withdrawals: Arc<DataContract>,
    pub(crate) dashpay: Arc<DataContract>,
    pub(crate) token_history: Arc<DataContract>,
    pub(crate) keyword_search: Arc<DataContract>,
}

impl SystemContracts {
    /// Load every system contract as `platform_version` defines it.
    pub(crate) fn load(platform_version: &PlatformVersion) -> Result<Self, Box<ProtocolError>> {
        let load = |contract| {
            load_system_data_contract(contract, platform_version)
                .map(Arc::new)
                .map_err(Box::new)
        };
        Ok(Self {
            protocol_version: platform_version.protocol_version,
            dpns: load(SystemDataContract::DPNS)?,
            withdrawals: load(SystemDataContract::Withdrawals)?,
            dashpay: load(SystemDataContract::Dashpay)?,
            token_history: load(SystemDataContract::TokenHistory)?,
            keyword_search: load(SystemDataContract::KeywordSearch)?,
        })
    }
}

/// [`SystemContracts`], one set per protocol version.
///
/// The proof verifier asks for the set at the version a response reports, the
/// app's own reads at the version the SDK runs; each version keeps its own
/// set, so neither lookup evicts the other's. The map holds at most one set
/// per protocol version table this build defines. Its lock is held only to
/// read or insert, never across a load.
#[derive(Debug)]
pub(crate) struct SystemContractsCache {
    sets: RwLock<BTreeMap<u32, Arc<SystemContracts>>>,
    /// The set the app's own reads last got, served when a reload fails.
    current: ArcSwap<SystemContracts>,
    /// The last version whose reload failed, so the failure is logged once.
    failed_version: AtomicU32,
}

impl SystemContractsCache {
    pub(crate) fn new(contracts: SystemContracts) -> Self {
        let contracts = Arc::new(contracts);
        Self {
            sets: RwLock::new(BTreeMap::from([(
                contracts.protocol_version,
                Arc::clone(&contracts),
            )])),
            current: ArcSwap::new(contracts),
            failed_version: AtomicU32::new(0),
        }
    }

    /// The set for exactly `platform_version`, loaded on first use. A failed
    /// load is an error, never the set of another version: what verifies a
    /// proof must not mix schemas.
    pub(crate) fn try_get(
        &self,
        platform_version: &PlatformVersion,
    ) -> Result<Arc<SystemContracts>, Box<ProtocolError>> {
        let version = platform_version.protocol_version;
        if let Some(cached) = self
            .sets
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&version)
        {
            return Ok(Arc::clone(cached));
        }
        let fresh = Arc::new(SystemContracts::load(platform_version)?);
        // Two callers loading at once keep the first set inserted.
        let mut sets = self.sets.write().unwrap_or_else(PoisonError::into_inner);
        Ok(Arc::clone(sets.entry(version).or_insert(fresh)))
    }

    /// The set for `platform_version`, for the app's own reads (names,
    /// documents, forms). A failed load keeps the set these reads last got,
    /// so the app keeps working on its last loaded contracts; proof
    /// verification uses [`Self::try_get`] instead.
    pub(crate) fn get(&self, platform_version: &PlatformVersion) -> Arc<SystemContracts> {
        match self.try_get(platform_version) {
            Ok(contracts) => {
                if !Arc::ptr_eq(&self.current.load(), &contracts) {
                    self.current.store(Arc::clone(&contracts));
                }
                contracts
            }
            Err(error) => {
                let cached = self.current.load_full();
                let version = platform_version.protocol_version;
                if self.failed_version.swap(version, Ordering::Relaxed) != version {
                    tracing::error!(
                        protocol_version = version,
                        kept_protocol_version = cached.protocol_version,
                        ?error,
                        "Could not load the system contracts for the network's protocol version; keeping the loaded ones"
                    );
                }
                cached
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;

    fn version(protocol_version: u32) -> &'static PlatformVersion {
        PlatformVersion::get(protocol_version).expect("known protocol version")
    }

    /// Protocol 14 redefines some system contracts, so a set loaded at 13
    /// must not be served once the network runs 14.
    #[test]
    fn a_new_protocol_version_reloads_the_contracts() {
        let v13 = version(13);
        let v14 = version(14);
        let dashpay_v13 = load_system_data_contract(SystemDataContract::Dashpay, v13).unwrap();
        let dashpay_v14 = load_system_data_contract(SystemDataContract::Dashpay, v14).unwrap();
        assert_ne!(
            dashpay_v13, dashpay_v14,
            "precondition: protocol 14 redefines the DashPay contract"
        );

        let cache = SystemContractsCache::new(SystemContracts::load(v13).unwrap());
        assert_eq!(*cache.get(v13).dashpay, dashpay_v13);
        let reloaded = cache.get(v14);
        assert_eq!(*reloaded.dashpay, dashpay_v14);
        assert_eq!(reloaded.dpns.id(), cache.get(v13).dpns.id());
        assert_eq!(
            *cache.get(v14).withdrawals,
            load_system_data_contract(SystemDataContract::Withdrawals, v14).unwrap()
        );
    }

    /// A version whose contracts cannot be loaded is an error for the exact
    /// lookup; the app's lookup keeps the set it has.
    #[test]
    fn an_unloadable_version_fails_the_exact_lookup() {
        let v13 = version(13);
        let cache = SystemContractsCache::new(SystemContracts::load(v13).unwrap());
        let mut unloadable = v13.clone();
        unloadable.protocol_version = u32::MAX;

        assert!(cache.try_get(&unloadable).is_err());
        assert_eq!(cache.get(&unloadable).protocol_version, 13);
        assert!(Arc::ptr_eq(&cache.get(v13), &cache.try_get(v13).unwrap()));
    }

    /// Regression: a lookup at another version (the proof verifier follows a
    /// response's version) swapped out the app's set, so every such lookup
    /// reloaded the contracts twice. Each version keeps its own set.
    #[test]
    fn a_lookup_at_another_version_keeps_the_current_set() {
        let v13 = version(13);
        let v14 = version(14);
        let cache = SystemContractsCache::new(SystemContracts::load(v13).unwrap());
        let current = cache.get(v13);

        let other = cache.try_get(v14).unwrap();
        assert!(Arc::ptr_eq(&current, &cache.get(v13)));
        assert!(Arc::ptr_eq(&other, &cache.try_get(v14).unwrap()));
    }

    /// The same version serves the cached set itself, not a reload.
    #[test]
    fn the_same_protocol_version_keeps_the_cached_set() {
        let v13 = version(13);
        let cache = SystemContractsCache::new(SystemContracts::load(v13).unwrap());
        assert!(Arc::ptr_eq(&cache.get(v13), &cache.get(v13)));
    }
}
