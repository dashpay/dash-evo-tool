//! The system data contracts, as defined at the protocol version in use.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

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

/// [`SystemContracts`] that follow the protocol version in use.
///
/// Lock-free: the set lives in an [`ArcSwap`]. Two callers reloading at once
/// store equivalent sets; a set stored late for an older version is replaced
/// on the next access, so the cache converges on the current version.
#[derive(Debug)]
pub(crate) struct SystemContractsCache {
    current: ArcSwap<SystemContracts>,
    /// The last version whose reload failed, so the failure is logged once.
    failed_version: AtomicU32,
}

impl SystemContractsCache {
    pub(crate) fn new(contracts: SystemContracts) -> Self {
        Self {
            current: ArcSwap::from_pointee(contracts),
            failed_version: AtomicU32::new(0),
        }
    }

    /// The set for exactly `platform_version`, reloaded if the cached one was
    /// built for another version. A failed reload is an error, never the set
    /// of another version: what verifies a proof must not mix schemas.
    pub(crate) fn try_get(
        &self,
        platform_version: &PlatformVersion,
    ) -> Result<Arc<SystemContracts>, Box<ProtocolError>> {
        let cached = self.current.load_full();
        if cached.protocol_version == platform_version.protocol_version {
            return Ok(cached);
        }
        let fresh = Arc::new(SystemContracts::load(platform_version)?);
        self.current.store(Arc::clone(&fresh));
        Ok(fresh)
    }

    /// The set for `platform_version`, for the app's own reads (names,
    /// documents, forms). A failed reload keeps the cached set, so the app
    /// keeps working on its last loaded contracts; proof verification uses
    /// [`Self::try_get`] instead.
    pub(crate) fn get(&self, platform_version: &PlatformVersion) -> Arc<SystemContracts> {
        match self.try_get(platform_version) {
            Ok(contracts) => contracts,
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

    /// The same version serves the cached set itself, not a reload.
    #[test]
    fn the_same_protocol_version_keeps_the_cached_set() {
        let v13 = version(13);
        let cache = SystemContractsCache::new(SystemContracts::load(v13).unwrap());
        assert!(Arc::ptr_eq(&cache.get(v13), &cache.get(v13)));
    }
}
