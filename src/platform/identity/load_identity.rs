use crate::context::AppContext;
use crate::model::qualified_identity::EncryptedPrivateKeyTarget::{
    PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::platform::identity::{verify_key_input, IdentityInputToLoad};
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use dash_sdk::dpp::identifier::MasternodeIdentifiers;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Fetch, Identifier, Identity};
use dash_sdk::Sdk;
use std::collections::BTreeMap;

impl AppContext {
    pub(super) async fn load_identity(
        &self,
        sdk: &Sdk,
        input: IdentityInputToLoad,
    ) -> Result<(), String> {
        let IdentityInputToLoad {
            identity_id_input,
            identity_type,
            voting_private_key_input,
            alias_input,
            owner_private_key_input,
            payout_address_private_key_input,
            keys_input: _,
        } = input;

        // Verify the voting private key
        let owner_private_key_bytes = verify_key_input(owner_private_key_input, "Owner")?;

        // Verify the voting private key
        let voting_private_key_bytes = verify_key_input(voting_private_key_input, "Voting")?;

        let payout_address_private_key_bytes =
            verify_key_input(payout_address_private_key_input, "Payout Address")?;

        // Parse the identity ID
        let identity_id = match Identifier::from_string(&identity_id_input, Encoding::Base58)
            .or_else(|_| Identifier::from_string(&identity_id_input, Encoding::Hex))
        {
            Ok(id) => id,
            Err(e) => return Err(format!("Identifier error: {}", e)),
        };

        // Fetch the identity using the SDK
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err("Identity not found".to_string()),
            Err(e) => return Err(format!("Error fetching identity: {}", e)),
        };

        let mut encrypted_private_keys = BTreeMap::new();

        if identity_type != IdentityType::User && owner_private_key_bytes.is_some() {
            let owner_private_key_bytes = owner_private_key_bytes.unwrap();
            let key =
                self.verify_owner_key_exists_on_identity(&identity, &owner_private_key_bytes)?;
            encrypted_private_keys.insert(
                (PrivateKeyOnMainIdentity, key.id()),
                (key.clone(), owner_private_key_bytes),
            );
        }

        if identity_type != IdentityType::User && payout_address_private_key_bytes.is_some() {
            let payout_address_private_key_bytes = payout_address_private_key_bytes.unwrap();
            let key = self.verify_payout_address_key_exists_on_identity(
                &identity,
                &payout_address_private_key_bytes,
            )?;
            encrypted_private_keys.insert(
                (PrivateKeyOnMainIdentity, key.id()),
                (key.clone(), payout_address_private_key_bytes),
            );
        }

        // If the identity type is not a User, and we have a voting private key, verify it
        let associated_voter_identity = if identity_type != IdentityType::User
            && voting_private_key_bytes.is_some()
        {
            let voting_private_key_bytes = voting_private_key_bytes.unwrap();
            if let Ok(private_key) =
                PrivateKey::from_slice(voting_private_key_bytes.as_slice(), self.network)
            {
                // Make the vote identifier
                let address = private_key.public_key(&Secp256k1::new()).pubkey_hash();
                let voter_identifier =
                    Identifier::create_voter_identifier(identity_id.as_bytes(), address.as_ref());

                // Fetch the voter identifier
                let voter_identity =
                    match Identity::fetch_by_identifier(sdk, voter_identifier).await {
                        Ok(Some(identity)) => identity,
                        Ok(None) => return Err("Voter Identity not found".to_string()),
                        Err(e) => return Err(format!("Error fetching voter identity: {}", e)),
                    };

                let key = self.verify_voting_key_exists_on_identity(
                    &voter_identity,
                    &voting_private_key_bytes,
                )?;
                encrypted_private_keys.insert(
                    (PrivateKeyOnVoterIdentity, key.id()),
                    (key.clone(), voting_private_key_bytes),
                );
                Some((voter_identity, key))
            } else {
                return Err("Voting private key is not valid".to_string());
            }
        } else {
            None
        };

        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: if alias_input.is_empty() {
                None
            } else {
                Some(alias_input)
            },
            encrypted_private_keys,
        };

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity)
            .map_err(|e| format!("Database error: {}", e))?;

        Ok(())
    }
}
