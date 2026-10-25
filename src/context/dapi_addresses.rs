//! Moving the SDK to new DAPI addresses without replacing it.
//!
//! Every `Sdk` clone shares its DAPI address list (the addresses, their ban
//! state and the request rotation), and each request picks its address from
//! that list. Changing the list in place therefore moves every clone, the
//! wallet backend's and the upstream wallet components' included.

use dash_sdk::dapi_client::{Address, AddressList};

/// Make `shared` hold exactly the addresses of `wanted`, in place, so every
/// SDK sharing `shared` follows.
///
/// An address in both lists keeps its status, bans included. The new
/// addresses are added before the old ones are removed, so a request sent
/// meanwhile always finds an address. `wanted` is a freshly parsed list that
/// nothing else shares; consuming it does not touch `shared`.
pub(super) fn retarget_address_list(shared: &AddressList, wanted: AddressList) {
    // A clone shares the addresses, their status and the rotation.
    let mut shared = shared.clone();
    let wanted: Vec<Address> = wanted.into_iter().map(|(address, _)| address).collect();
    for address in &wanted {
        shared.add(address.clone());
    }
    for stale in held_addresses(&shared)
        .into_iter()
        .filter(|held| !wanted.contains(held))
    {
        shared.remove(&stale);
    }
}

/// Every address `list` holds, banned ones included, read without draining
/// it (its `IntoIterator` empties the shared map).
fn held_addresses(list: &AddressList) -> Vec<Address> {
    list.ban_info()
        .into_iter()
        .filter_map(|info| match info.uri.parse::<Address>() {
            Ok(address) => Some(address),
            Err(error) => {
                tracing::warn!(uri = %info.uri, ?error, "A held DAPI address could not be read back");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn address(uri: &str) -> Address {
        uri.parse().expect("valid address")
    }

    fn addresses_of(list: &AddressList) -> HashSet<String> {
        list.ban_info().into_iter().map(|info| info.uri).collect()
    }

    /// The list ends up holding exactly the wanted addresses, a kept address
    /// keeps its ban, and every clone of the list sees the change.
    #[test]
    fn retargeting_keeps_bans_of_kept_addresses_and_reaches_every_clone() {
        let kept = address("https://10.0.0.1:443");
        let dropped = address("https://10.0.0.2:443");
        let dropped_banned = address("https://10.0.0.3:443");
        let added = address("https://10.0.0.4:443");
        let shared: AddressList = [kept.clone(), dropped.clone(), dropped_banned.clone()]
            .into_iter()
            .collect();
        let clone = shared.clone();
        assert!(shared.ban(&kept));
        assert!(shared.ban(&dropped_banned));

        retarget_address_list(&shared, [kept.clone(), added.clone()].into_iter().collect());

        let expected: HashSet<String> = [&kept, &added].map(ToString::to_string).into();
        assert_eq!(addresses_of(&shared), expected);
        assert_eq!(addresses_of(&clone), expected, "a clone follows the list");
        assert!(clone.is_banned(&kept), "a kept address keeps its ban");
        assert!(!clone.is_banned(&added));
    }

    /// Retargeting to the addresses already held changes nothing.
    #[test]
    fn retargeting_to_the_same_addresses_keeps_them_all() {
        let first = address("https://10.0.0.1:443");
        let second = address("http://10.0.0.2:1443");
        let shared: AddressList = [first.clone(), second.clone()].into_iter().collect();
        assert!(shared.ban(&second));

        retarget_address_list(
            &shared,
            [first.clone(), second.clone()].into_iter().collect(),
        );

        assert_eq!(shared.len(), 2);
        assert!(shared.is_banned(&second));
    }
}
