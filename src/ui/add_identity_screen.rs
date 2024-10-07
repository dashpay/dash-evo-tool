use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::EncryptedPrivateKeyTarget::{
    PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
use dash_sdk::dpp::dashcore::PrivateKey;
use dash_sdk::dpp::identifier::{Identifier, MasternodeIdentifiers};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, KeyID, KeyType, Purpose};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::platform::Fetch;
use eframe::egui::Context;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub struct AddIdentityScreen {
    identity_id_input: String,
    identity_type: IdentityType,
    alias_input: String,
    voting_private_key_input: String,
    owner_private_key_input: String,
    error_message: Option<String>,
    identity_result:
        Option<Arc<Mutex<Option<Result<(Identity, Option<Identity>, Option<Vec<u8>>), String>>>>>,
    app_context: Arc<AppContext>,
}

impl AddIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            identity_id_input: String::new(),
            identity_type: IdentityType::User,
            alias_input: String::new(),
            voting_private_key_input: String::new(),
            owner_private_key_input: String::new(),
            error_message: None,
            identity_result: None,
            app_context: app_context.clone(),
        }
    }
}

fn verify_key_input(
    untrimmed_private_key: String,
    type_key: &str,
) -> Result<Option<Vec<u8>>, String> {
    let private_key = untrimmed_private_key.trim().to_string();
    match private_key.len() {
        64 => {
            // hex
            match hex::decode(private_key.as_str()) {
                Ok(decoded) => Ok(Some(decoded)),
                Err(_) => Err(format!(
                    "{} key is the size of a hex key but isn't hex",
                    type_key
                )),
            }
        }
        51 | 52 => {
            // wif
            match PrivateKey::from_wif(private_key.as_str()) {
                Ok(key) => Ok(Some(key.to_bytes())),
                Err(_) => Err(format!(
                    "{} key is the length of a WIF key but is invalid",
                    type_key
                )),
            }
        }
        0 => Ok(None),
        _ => Err(format!("{} key is of incorrect size", type_key)),
    }
}

impl AddIdentityScreen {
    fn verify_voting_key_exists_on_identity(
        &self,
        voting_identity: &Identity,
        private_voting_key: &[u8],
    ) -> Result<KeyID, String> {
        // We start by getting all the voting keys
        let voting_keys: Vec<IdentityPublicKey> = voting_identity
            .public_keys()
            .values()
            .filter_map(|key| {
                if key.purpose() != Purpose::VOTING {
                    return None;
                }
                Some(key.clone())
            })
            .collect();
        if voting_keys.is_empty() {
            return Err("This identity does not contain any voting keys".to_string());
        }
        // Then we get all the key types of the voting keys
        let key_types: HashSet<KeyType> = voting_keys.iter().map(|key| key.key_type()).collect();
        // For every key type get the associated public key data
        let public_key_bytes_for_each_key_type = key_types
            .into_iter()
            .map(|key_type| {
                Ok((
                    key_type,
                    key_type.public_key_data_from_private_key_data(
                        private_voting_key,
                        self.app_context.network,
                    )?,
                ))
            })
            .collect::<Result<HashMap<KeyType, Vec<u8>>, ProtocolError>>()
            .map_err(|e| e.to_string())?;
        let Some(key) = voting_keys.iter().find(|key| {
            let Some(public_key_bytes) = public_key_bytes_for_each_key_type.get(&key.key_type())
            else {
                return false;
            };
            key.data().as_slice() == public_key_bytes.as_slice()
        }) else {
            return Err(
                "Identity does not have a voting public key matching this private key".to_string(),
            );
        };
        Ok(key.id())
    }
}
impl ScreenLike for AddIdentityScreen {
    fn refresh(&mut self) {}

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Add Identity", AppAction::None),
            ],
            None,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Add Identity");

            ui.horizontal(|ui| {
                ui.label("Identity ID (Hex or Base58):");
                ui.text_edit_singleline(&mut self.identity_id_input);
            });

            ui.horizontal(|ui| {
                ui.label("Identity Type:");
                egui::ComboBox::from_label("")
                    .selected_text(format!("{:?}", self.identity_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.identity_type, IdentityType::User, "User");
                        ui.selectable_value(
                            &mut self.identity_type,
                            IdentityType::Masternode,
                            "Masternode",
                        );
                        ui.selectable_value(
                            &mut self.identity_type,
                            IdentityType::Evonode,
                            "Evonode",
                        );
                    });
            });

            // Input for Alias
            ui.horizontal(|ui| {
                ui.label("Alias:");
                ui.text_edit_singleline(&mut self.alias_input);
            });

            // Input for Voting Private Key
            ui.horizontal(|ui| {
                ui.label("Voting Private Key:");
                ui.text_edit_singleline(&mut self.voting_private_key_input);
            });

            // Input for Owner Private Key
            ui.horizontal(|ui| {
                ui.label("Owner Private Key:");
                ui.text_edit_singleline(&mut self.owner_private_key_input);
            });

            if ui.button("Load Identity").clicked() {
                let identity_id_input = self.identity_id_input.trim().to_string();
                let identity_type = self.identity_type.clone();
                let mut error_out = false;
                let voting_private_key_bytes =
                    match verify_key_input(self.voting_private_key_input.to_string(), "Voting") {
                        Ok(voting_private_key_bytes) => voting_private_key_bytes,
                        Err(e) => {
                            self.error_message = Some(e);
                            error_out = true;
                            None
                        }
                    };

                if !error_out {
                    let sdk_clone = Arc::clone(&self.app_context.sdk);

                    // Create the shared state
                    let identity_result = Arc::new(Mutex::new(None));
                    let identity_result_clone = Arc::clone(&identity_result);

                    // Lock the mutex and clone the Sdk
                    let sdk_instance = {
                        let sdk = sdk_clone.read().unwrap();
                        sdk.clone() // Assuming Sdk implements Clone
                    };
                    // Now sdk_instance is owned and can be moved into the async block

                    let network = self.app_context.network;

                    // Spawn the async task
                    tokio::spawn(async move {
                        // Parse the identity ID
                        let identity_id =
                            match Identifier::from_string(&identity_id_input, Encoding::Base58) {
                                Ok(id) => id,
                                Err(_) => {
                                    match Identifier::from_string(&identity_id_input, Encoding::Hex)
                                    {
                                        Ok(id) => id,
                                        Err(e) => {
                                            // Store the error and return early
                                            let mut identity_result =
                                                identity_result_clone.lock().unwrap();
                                            *identity_result =
                                                Some(Err(format!("Identifier error: {}", e)));
                                            return;
                                        }
                                    }
                                }
                            };

                        // Fetch the identity using the cloned sdk_instance
                        let fetch_result =
                            Identity::fetch_by_identifier(&sdk_instance, identity_id).await;

                        let mut result = match fetch_result {
                            Ok(Some(identity)) => Ok(identity),
                            Ok(None) => Err("Identity not found".to_string()),
                            Err(e) => Err(format!("Error fetching identity: {}", e)),
                        }
                        .map(|identity| {
                            (identity, None::<Identity>, voting_private_key_bytes.clone())
                        });

                        if identity_type != IdentityType::User
                            && voting_private_key_bytes.is_some()
                            && result.is_ok()
                        {
                            if let Ok(private_key) = PrivateKey::from_slice(
                                voting_private_key_bytes.as_ref().unwrap().as_slice(),
                                network,
                            ) {
                                let address =
                                    private_key.public_key(&Secp256k1::new()).pubkey_hash();
                                let voter_identifier = Identifier::create_voter_identifier(
                                    identity_id.as_bytes(),
                                    address.as_ref(),
                                );
                                let fetch_voter_result =
                                    Identity::fetch_by_identifier(&sdk_instance, voter_identifier)
                                        .await;

                                let voter_result = match fetch_voter_result {
                                    Ok(Some(identity)) => Ok(identity),
                                    Ok(None) => Err("Voter Identity not found".to_string()),
                                    Err(e) => Err(format!("Error fetching voter identity: {}", e)),
                                };

                                match voter_result {
                                    Ok(voter_identity) => {
                                        result = result.map(|(identity, _, private_key)| {
                                            (identity, Some(voter_identity), private_key)
                                        })
                                    }
                                    Err(e) => {
                                        result = Err(e);
                                    }
                                }
                            } else {
                                result = Err("Voting private key is not valid".to_string());
                            }
                        }

                        // Store the result in the shared state
                        let mut identity_result = identity_result_clone.lock().unwrap();
                        *identity_result = Some(result);
                    });

                    // Store the shared state in the struct
                    self.identity_result = Some(identity_result);
                }
            }

            // Handle the promise
            // Handle the result
            if let Some(identity_result) = &self.identity_result {
                let result = identity_result.lock().unwrap().clone();
                if let Some(result) = result {
                    match result {
                        Ok((identity, voting_identity, voting_private_key)) => {
                            let mut should_insert = true;
                            let mut encrypted_private_keys = BTreeMap::new();
                            let voting_identity_with_key_id = if let Some(voting_key) =
                                voting_private_key.as_ref()
                            {
                                let (identity_with_voter_key, target) =
                                    if let Some(mno_voter_identity) = voting_identity.as_ref() {
                                        (mno_voter_identity, PrivateKeyOnVoterIdentity)
                                    } else {
                                        (&identity, PrivateKeyOnMainIdentity)
                                    };
                                match self.verify_voting_key_exists_on_identity(
                                    identity_with_voter_key,
                                    voting_key,
                                ) {
                                    Ok(id) => {
                                        encrypted_private_keys
                                            .insert((target, id), voting_key.clone());
                                        voting_identity.map(|v| (v, id))
                                    }
                                    Err(e) => {
                                        self.error_message = Some(e);
                                        should_insert = false;
                                        None
                                    }
                                }
                            } else {
                                None
                            };
                            if should_insert {
                                let qualified_identity = QualifiedIdentity {
                                    identity,
                                    associated_voter_identity: voting_identity_with_key_id,
                                    associated_operator_identity: None,
                                    associated_owner_key_id: None,
                                    identity_type: self.identity_type.clone(),
                                    alias: if self.alias_input.is_empty() {
                                        None
                                    } else {
                                        Some(self.alias_input.clone())
                                    },
                                    encrypted_private_keys,
                                };

                                // Insert qualified identity into the database
                                if let Err(e) = self
                                    .app_context
                                    .insert_local_qualified_identity(&qualified_identity)
                                {
                                    self.error_message = Some(format!("Database error: {}", e));
                                } else {
                                    action = AppAction::PopScreen;
                                }
                            }
                        }
                        Err(e) => {
                            self.error_message = Some(e);
                        }
                    }
                    // Clear the shared state
                    self.identity_result = None;
                } else {
                    // The task is still running
                    ui.label("Loading...");
                    // Request a repaint to update the UI
                    ctx.request_repaint();
                }
            }

            if let Some(error) = &self.error_message {
                ui.label(format!("Error: {}", error));
            }
        });

        action
    }
}
