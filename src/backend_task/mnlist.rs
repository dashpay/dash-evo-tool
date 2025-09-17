use crate::backend_task::BackendTaskSuccessResult;
use crate::components::core_p2p_handler::CoreP2PHandler;
use crate::context::AppContext;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::bls_sig_utils::BLSSignature;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::network::message_qrinfo::QRInfo;
use dash_sdk::dpp::dashcore::network::message_sml::MnListDiff;
use dash_sdk::dpp::dashcore::{BlockHash, Network};

#[derive(Debug, Clone, PartialEq)]
pub enum MnListTask {
    FetchEndDmlDiff {
        base_block_height: u32,
        base_block_hash: BlockHash,
        block_height: u32,
        block_hash: BlockHash,
        validate_quorums: bool,
    },
    FetchEndQrInfo {
        base_block_hash: BlockHash,
        block_hash: BlockHash,
    },
    FetchEndQrInfoWithDmls {
        base_block_hash: BlockHash,
        block_hash: BlockHash,
    },
    FetchChainLocks {
        base_block_height: u32,
        block_height: u32,
    },
    /// Fetch a sequence of MNListDiffs for validation purposes
    /// Each tuple is (base_height, base_hash, height, hash)
    FetchDiffsChain {
        chain: Vec<(u32, BlockHash, u32, BlockHash)>,
    },
}

pub async fn run_mnlist_task(
    app: &AppContext,
    task: MnListTask,
) -> Result<BackendTaskSuccessResult, String> {
    match task {
        MnListTask::FetchEndDmlDiff {
            base_block_height,
            base_block_hash,
            block_height,
            block_hash,
            validate_quorums: _,
        } => {
            let network = app.network;
            let mut p2p = CoreP2PHandler::new(network, None)?;
            let diff = p2p.get_dml_diff(base_block_hash, block_hash)?;
            Ok(BackendTaskSuccessResult::MnListFetchedDiff {
                base_height: base_block_height,
                height: block_height,
                diff,
            })
        }
        MnListTask::FetchEndQrInfo {
            base_block_hash,
            block_hash,
        } => {
            let network = app.network;
            let mut p2p = CoreP2PHandler::new(network, None)?;
            let qr_info = p2p.get_qr_info(vec![base_block_hash], block_hash)?;
            Ok(BackendTaskSuccessResult::MnListFetchedQrInfo { qr_info })
        }
        MnListTask::FetchEndQrInfoWithDmls {
            base_block_hash,
            block_hash,
        } => {
            // For now, fetch QRInfo; UI can integrate included diffs from QRInfo
            let network = app.network;
            let mut p2p = CoreP2PHandler::new(network, None)?;
            let qr_info = p2p.get_qr_info(vec![base_block_hash], block_hash)?;
            Ok(BackendTaskSuccessResult::MnListFetchedQrInfo { qr_info })
        }
        MnListTask::FetchChainLocks {
            base_block_height,
            block_height,
        } => {
            let client = app.core_client.read().unwrap();
            // Determine the range (replicate UI logic approximately)
            let loaded_list_height = match app.network {
                Network::Dash => 2_227_096,
                Network::Testnet => 1_296_600,
                _ => 0,
            };
            let max_blocks = 2000u32;
            let start_height = if base_block_height < loaded_list_height {
                block_height.saturating_sub(max_blocks)
            } else {
                base_block_height
            };
            let end_height = start_height.saturating_add(max_blocks).min(block_height);

            let mut out: Vec<((u32, BlockHash), Option<BLSSignature>)> = Vec::new();
            for h in start_height..end_height {
                if let Ok(bh2) = client.get_block_hash(h) {
                    // Convert RPC hash to DPP hash
                    let bh = BlockHash::from_byte_array(bh2.to_byte_array());
                    // Get block and extract coinbase best_cl_signature
                    if let Ok(block) = client.get_block(&bh2) {
                        let sig_opt = block
                            .coinbase()
                            .and_then(|cb| cb.special_transaction_payload.as_ref())
                            .and_then(|pl| pl.clone().to_coinbase_payload().ok())
                            .and_then(|cp| cp.best_cl_signature)
                            .map(|sig| sig.to_bytes().into());
                        out.push(((h, bh), sig_opt));
                    } else {
                        out.push(((h, bh), None));
                    }
                }
            }
            Ok(BackendTaskSuccessResult::MnListChainLockSigs { entries: out })
        }
        MnListTask::FetchDiffsChain { chain } => {
            let network = app.network;
            let mut p2p = CoreP2PHandler::new(network, None)?;
            let mut items = Vec::with_capacity(chain.len());
            for (base_h, base_hash, h, hash) in chain {
                let diff = p2p.get_dml_diff(base_hash, hash)?;
                items.push(((base_h, h), diff));
            }
            Ok(BackendTaskSuccessResult::MnListFetchedDiffs { items })
        }
    }
}
