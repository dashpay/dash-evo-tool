//! Unit tests for DTO serialization and deserialization.
//!
//! Verifies that all DTOs round-trip through JSON correctly and that
//! serde rename conventions (camelCase) are applied.

use crate::dto::*;

// ─── FeeResultDto ────────────────────────────────────────

#[test]
fn fee_result_dto_roundtrip() {
    let dto = FeeResultDto {
        estimated_fee: 50_000,
        actual_fee: 42_000,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"estimatedFee\""));
    assert!(json.contains("\"actualFee\""));
    let back: FeeResultDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back, dto);
}

// ─── WalletDto ───────────────────────────────────────────

#[test]
fn wallet_dto_roundtrip() {
    let dto = WalletDto {
        seed_hash: "ab".repeat(32),
        uses_password: true,
        alias: Some("Test Wallet".into()),
        is_main: true,
        confirmed_balance: 1_000_000,
        unconfirmed_balance: 500,
        total_balance: 1_000_500,
        addresses: vec![WalletAddressDto {
            address: "XpYv3N7p1234".into(),
            balance: 1_000_000,
            total_received: 2_000_000,
            derivation_path: "m/44'/5'/0'/0/0".into(),
        }],
        transactions: vec![],
        unused_asset_locks: vec![],
        platform_addresses: vec![],
        identity_indexes: vec![0, 1],
        password_hint: Some("my hint".into()),
    };
    let json = serde_json::to_string(&dto).unwrap();
    // Verify camelCase
    assert!(json.contains("\"seedHash\""));
    assert!(json.contains("\"usesPassword\""));
    assert!(json.contains("\"isMain\""));
    assert!(json.contains("\"confirmedBalance\""));
    assert!(json.contains("\"totalBalance\""));
    assert!(json.contains("\"derivationPath\""));
    assert!(json.contains("\"identityIndexes\""));
    assert!(json.contains("\"passwordHint\""));
    let back: WalletDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.seed_hash, dto.seed_hash);
    assert_eq!(back.total_balance, dto.total_balance);
    assert_eq!(back.addresses.len(), 1);
}

#[test]
fn wallet_transaction_dto_roundtrip() {
    let dto = WalletTransactionDto {
        txid: "aabb".repeat(16),
        timestamp: 1_700_000_000,
        height: Some(12345),
        block_hash: Some("ccdd".repeat(16)),
        net_amount: -50_000,
        fee: Some(226),
        label: None,
        is_ours: true,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"netAmount\":-50000"));
    assert!(json.contains("\"blockHash\""));
    assert!(json.contains("\"isOurs\":true"));
    let back: WalletTransactionDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.net_amount, -50_000);
}

#[test]
fn asset_lock_dto_roundtrip() {
    let dto = AssetLockDto {
        txid: "ff".repeat(32),
        address: "XtestAddr123".into(),
        amount: 100_000,
        has_instant_lock: true,
        has_asset_lock_proof: false,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"hasInstantLock\":true"));
    assert!(json.contains("\"hasAssetLockProof\":false"));
    let back: AssetLockDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.amount, 100_000);
}

// ─── SingleKeyWalletDto ──────────────────────────────────

#[test]
fn single_key_wallet_dto_roundtrip() {
    let dto = SingleKeyWalletDto {
        key_hash: "ee".repeat(32),
        uses_password: false,
        public_key: "02abcdef".into(),
        address: "XsingleKeyAddr".into(),
        alias: Some("Imported Key".into()),
        confirmed_balance: 500_000,
        unconfirmed_balance: 0,
        total_balance: 500_000,
        utxo_count: 3,
        utxos: vec![
            UtxoDto {
                txid: "aa".repeat(32),
                vout: 0,
                amount: 200_000,
            },
            UtxoDto {
                txid: "bb".repeat(32),
                vout: 1,
                amount: 150_000,
            },
            UtxoDto {
                txid: "cc".repeat(32),
                vout: 2,
                amount: 150_000,
            },
        ],
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"keyHash\""));
    assert!(json.contains("\"publicKey\""));
    assert!(json.contains("\"utxoCount\":3"));
    let back: SingleKeyWalletDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.key_hash, dto.key_hash);
}

// ─── WalletRefDto (tagged enum) ─────────────────────────

#[test]
fn wallet_ref_dto_hd_variant() {
    let dto = WalletRefDto::Hd {
        seed_hash: "ab".repeat(32),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"type\":\"hd\""));
    assert!(json.contains("\"seedHash\""));
    let back: WalletRefDto = serde_json::from_str(&json).unwrap();
    match back {
        WalletRefDto::Hd { seed_hash } => assert_eq!(seed_hash, "ab".repeat(32)),
        _ => panic!("expected Hd variant"),
    }
}

#[test]
fn wallet_ref_dto_single_key_variant() {
    let dto = WalletRefDto::SingleKey {
        key_hash: "cd".repeat(32),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"type\":\"singleKey\""));
    assert!(json.contains("\"keyHash\""));
    let back: WalletRefDto = serde_json::from_str(&json).unwrap();
    match back {
        WalletRefDto::SingleKey { key_hash } => assert_eq!(key_hash, "cd".repeat(32)),
        _ => panic!("expected SingleKey variant"),
    }
}

// ─── QualifiedIdentityDto ────────────────────────────────

#[test]
fn qualified_identity_dto_roundtrip() {
    let dto = QualifiedIdentityDto {
        id: "aa".repeat(32),
        identity_type: IdentityTypeDto::User,
        alias: Some("Alice".into()),
        balance: 1_000_000_000,
        keys: vec![IdentityKeyDto {
            key_id: 0,
            key_type: "ECDSA_SECP256K1".into(),
            purpose: "AUTHENTICATION".into(),
            security_level: "MASTER".into(),
            data: "02abcdef1234".into(),
            is_disabled: false,
            disabled_at: None,
            has_private_key: true,
        }],
        dpns_names: vec![DpnsNameInfoDto {
            name: "alice.dash".into(),
            acquired_at: 1_700_000_000,
        }],
        associated_wallet_hashes: vec!["bb".repeat(32)],
        wallet_index: Some(0),
        top_ups: vec![TopUpEntryDto {
            index: 0,
            amount: 500_000,
        }],
        status: IdentityStatusDto::Active,
        network: NetworkDto::Testnet,
        voter_identity_id: None,
        operator_identity_id: None,
    };
    let json = serde_json::to_string(&dto).unwrap();
    // Verify camelCase and structure
    assert!(json.contains("\"identityType\":\"user\""));
    assert!(json.contains("\"dpnsNames\""));
    assert!(json.contains("\"associatedWalletHashes\""));
    assert!(json.contains("\"walletIndex\":0"));
    assert!(json.contains("\"topUps\""));
    assert!(json.contains("\"voterIdentityId\":null"));
    assert!(json.contains("\"keyType\":\"ECDSA_SECP256K1\""));
    assert!(json.contains("\"hasPrivateKey\":true"));
    let back: QualifiedIdentityDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, dto.id);
    assert_eq!(back.keys.len(), 1);
    assert_eq!(back.dpns_names[0].name, "alice.dash");
}

#[test]
fn identity_type_dto_all_variants() {
    for (variant, expected) in [
        (IdentityTypeDto::User, "\"user\""),
        (IdentityTypeDto::Masternode, "\"masternode\""),
        (IdentityTypeDto::Evonode, "\"evonode\""),
    ] {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, expected);
        let back: IdentityTypeDto = serde_json::from_str(&json).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn identity_status_dto_all_variants() {
    for (variant, expected) in [
        (IdentityStatusDto::Unknown, "\"unknown\""),
        (IdentityStatusDto::PendingCreation, "\"pendingCreation\""),
        (IdentityStatusDto::Active, "\"active\""),
        (IdentityStatusDto::NotFound, "\"notFound\""),
        (IdentityStatusDto::FailedCreation, "\"failedCreation\""),
    ] {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, expected);
        let back: IdentityStatusDto = serde_json::from_str(&json).unwrap();
        assert_eq!(back, variant);
    }
}

// ─── DataContractDto ─────────────────────────────────────

#[test]
fn data_contract_dto_roundtrip() {
    let dto = DataContractDto {
        id: "cc".repeat(32),
        owner_id: "dd".repeat(32),
        alias: Some("DPNS".into()),
        version: 1,
        document_type_names: vec!["domain".into(), "preorder".into()],
        token_count: 0,
        schema_json: serde_json::json!({
            "domain": {
                "type": "object",
                "properties": {
                    "label": { "type": "string" }
                }
            }
        }),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"ownerId\""));
    assert!(json.contains("\"documentTypeNames\""));
    assert!(json.contains("\"tokenCount\":0"));
    assert!(json.contains("\"schemaJson\""));
    let back: DataContractDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.document_type_names.len(), 2);
    assert_eq!(
        back.schema_json["domain"]["properties"]["label"]["type"],
        "string"
    );
}

// ─── DocumentDto ─────────────────────────────────────────

#[test]
fn document_dto_roundtrip() {
    let dto = DocumentDto {
        id: "ee".repeat(32),
        owner_id: "ff".repeat(32),
        document_type: "domain".into(),
        data: serde_json::json!({
            "label": "alice",
            "normalizedLabel": "alice",
            "records": { "identity": "abc123" }
        }),
        revision: 1,
        created_at: Some(1_700_000_000),
        updated_at: None,
        transferred_at: None,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"documentType\":\"domain\""));
    assert!(json.contains("\"createdAt\":1700000000"));
    assert!(json.contains("\"updatedAt\":null"));
    assert!(json.contains("\"transferredAt\":null"));
    let back: DocumentDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.data["label"], "alice");
}

#[test]
fn document_page_dto_roundtrip() {
    let dto = DocumentPageDto {
        documents: vec![
            DocumentPageEntryDto {
                id: "aa".repeat(32),
                document: Some(DocumentDto {
                    id: "aa".repeat(32),
                    owner_id: "bb".repeat(32),
                    document_type: "profile".into(),
                    data: serde_json::json!({}),
                    revision: 1,
                    created_at: None,
                    updated_at: None,
                    transferred_at: None,
                }),
            },
            DocumentPageEntryDto {
                id: "cc".repeat(32),
                document: None,
            },
        ],
        has_more: true,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"hasMore\":true"));
    let back: DocumentPageDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.documents.len(), 2);
    assert!(back.documents[0].document.is_some());
    assert!(back.documents[1].document.is_none());
}

// ─── TokenInfoDto ────────────────────────────────────────

#[test]
fn token_info_dto_roundtrip() {
    let dto = TokenInfoDto {
        token_id: "11".repeat(32),
        token_name: "TestToken".into(),
        data_contract_id: "22".repeat(32),
        token_position: 0,
        description: Some("A test token".into()),
        configuration: TokenConfigurationDto {
            decimals: 8,
            max_supply: Some("21000000000000000".into()),
            is_paused: false,
            keeps_history: true,
            allows_transfers_to_frozen_identities: false,
            base_supply: "1000000000000".into(),
            conventions: serde_json::json!({
                "name": "TestToken",
                "symbol": "TST"
            }),
        },
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"tokenName\":\"TestToken\""));
    assert!(json.contains("\"tokenPosition\":0"));
    assert!(json.contains("\"maxSupply\":\"21000000000000000\""));
    assert!(json.contains("\"isPaused\":false"));
    assert!(json.contains("\"keepsHistory\":true"));
    assert!(json.contains("\"baseSupply\":\"1000000000000\""));
    let back: TokenInfoDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.configuration.decimals, 8);
    assert_eq!(back.configuration.max_supply.unwrap(), "21000000000000000");
}

#[test]
fn identity_token_balance_dto_roundtrip() {
    let dto = IdentityTokenBalanceDto {
        token_id: "11".repeat(32),
        token_alias: "TestToken".into(),
        identity_id: "22".repeat(32),
        balance: "5000000000".into(),
        estimated_unclaimed_rewards: Some("100000".into()),
        data_contract_id: "33".repeat(32),
        token_position: 0,
        decimals: 8,
        available_actions: IdentityTokenAvailableActionsDto {
            can_claim: true,
            can_estimate: true,
            can_mint: false,
            can_burn: true,
            can_freeze: false,
            can_unfreeze: false,
            can_destroy: false,
            can_do_emergency_action: false,
            can_maybe_purchase: true,
            can_set_price: false,
            can_transfer: true,
            can_update_config: false,
        },
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"canClaim\":true"));
    assert!(json.contains("\"canMint\":false"));
    assert!(json.contains("\"canTransfer\":true"));
    assert!(json.contains("\"estimatedUnclaimedRewards\":\"100000\""));
    let back: IdentityTokenBalanceDto = serde_json::from_str(&json).unwrap();
    assert!(back.available_actions.can_claim);
    assert!(!back.available_actions.can_mint);
}

// ─── NetworkDto ──────────────────────────────────────────

#[test]
fn network_dto_all_variants() {
    for (variant, expected) in [
        (NetworkDto::Dash, "\"dash\""),
        (NetworkDto::Testnet, "\"testnet\""),
        (NetworkDto::Devnet, "\"devnet\""),
        (NetworkDto::Regtest, "\"regtest\""),
    ] {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, expected);
        let back: NetworkDto = serde_json::from_str(&json).unwrap();
        assert_eq!(back, variant);
    }
}

// ─── Wallet result DTOs ──────────────────────────────────

#[test]
fn wallet_payment_result_dto_roundtrip() {
    let dto = WalletPaymentResultDto {
        txid: "ab".repeat(32),
        recipients: vec![
            PaymentRecipientDto {
                address: "XrecipAddr1".into(),
                amount: 100_000,
            },
            PaymentRecipientDto {
                address: "XrecipAddr2".into(),
                amount: 200_000,
            },
        ],
        total_amount: 300_000,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"totalAmount\":300000"));
    let back: WalletPaymentResultDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.recipients.len(), 2);
}

// ─── Token operation result DTOs ─────────────────────────

#[test]
fn token_operation_result_dto_roundtrip() {
    let dto = TokenOperationResultDto {
        operation: "mint".into(),
        fee_result: FeeResultDto {
            estimated_fee: 10_000,
            actual_fee: 8_500,
        },
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"operation\":\"mint\""));
    assert!(json.contains("\"feeResult\""));
    let back: TokenOperationResultDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.fee_result.actual_fee, 8_500);
}

// ─── Contract-related DTOs ───────────────────────────────

#[test]
fn contract_description_info_dto_roundtrip() {
    let dto = ContractDescriptionInfoDto {
        data_contract_id: "ab".repeat(32),
        description: "A payment contract".into(),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"dataContractId\""));
    let back: ContractDescriptionInfoDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.description, "A payment contract");
}

#[test]
fn group_action_dto_roundtrip() {
    let dto = GroupActionDto {
        group_position: 0,
        action_id: "action-123".into(),
        action_type: "mint".into(),
        signers_count: 2,
        required_signatures: 3,
        details: serde_json::json!({"amount": "1000"}),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"groupPosition\":0"));
    assert!(json.contains("\"signersCount\":2"));
    assert!(json.contains("\"requiredSignatures\":3"));
    let back: GroupActionDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.details["amount"], "1000");
}

// ─── WalletListDto ───────────────────────────────────────

#[test]
fn wallet_list_dto_roundtrip() {
    let dto = WalletListDto {
        hd_wallets: vec![],
        single_key_wallets: vec![],
        selected: Some(WalletRefDto::Hd {
            seed_hash: "ab".repeat(32),
        }),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"hdWallets\""));
    assert!(json.contains("\"singleKeyWallets\""));
    let back: WalletListDto = serde_json::from_str(&json).unwrap();
    assert!(back.selected.is_some());
}

// ─── IdentitySummaryDto ──────────────────────────────────

#[test]
fn identity_summary_dto_roundtrip() {
    let dto = IdentitySummaryDto {
        id: "aa".repeat(32),
        display_name: "Alice (alice.dash)".into(),
        identity_type: IdentityTypeDto::User,
        balance: 500_000_000,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"displayName\""));
    let back: IdentitySummaryDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.display_name, "Alice (alice.dash)");
}

// ─── TokenPricingDto ─────────────────────────────────────

#[test]
fn token_pricing_dto_with_schedule() {
    let dto = TokenPricingDto {
        token_id: "11".repeat(32),
        pricing_schedule: Some(serde_json::json!({
            "type": "linear",
            "base_price": "100"
        })),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"pricingSchedule\""));
    let back: TokenPricingDto = serde_json::from_str(&json).unwrap();
    assert!(back.pricing_schedule.is_some());
}

#[test]
fn token_pricing_dto_without_schedule() {
    let dto = TokenPricingDto {
        token_id: "11".repeat(32),
        pricing_schedule: None,
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"pricingSchedule\":null"));
    let back: TokenPricingDto = serde_json::from_str(&json).unwrap();
    assert!(back.pricing_schedule.is_none());
}

// ─── DistributionRewardEstimationDto ─────────────────────

#[test]
fn distribution_reward_estimation_dto_roundtrip() {
    let dto = DistributionRewardEstimationDto {
        identity_token: IdentityTokenIdentifierDto {
            identity_id: "aa".repeat(32),
            token_id: "bb".repeat(32),
        },
        estimated_amount: "999999".into(),
        explanation: serde_json::json!({
            "intervals": [{"start": 0, "end": 100, "rate": "10"}]
        }),
    };
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"identityToken\""));
    assert!(json.contains("\"estimatedAmount\":\"999999\""));
    let back: DistributionRewardEstimationDto = serde_json::from_str(&json).unwrap();
    assert_eq!(back.estimated_amount, "999999");
}
