use crate::app_dir::{app_user_data_dir_path, create_app_user_data_directory_if_not_exists};
use crate::cpu_compatibility::check_cpu_compatibility;
use std::env;

mod app;
mod app_dir;
mod backend_task;
mod components;
mod config;
mod context;
mod context_provider;
mod cpu_compatibility;
mod database;
mod logging;
mod model;
mod sdk_wrapper;
mod ui;
mod utils;

include!(concat!(env!("OUT_DIR"), "/version.rs"));

fn main() -> eframe::Result<()> {
    create_app_user_data_directory_if_not_exists()
        .expect("Failed to create app user_data directory");
    let app_data_dir =
        app_user_data_dir_path().expect("Failed to get app user_data directory path");
    println!("running v{}", VERSION);
    check_cpu_compatibility();
    // Initialize the Tokio runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(40)
        .enable_all()
        .build()
        .expect("multi-threading runtime cannot be initialized");

    // Run the native application
    runtime.block_on(async {
        let native_options = eframe::NativeOptions {
            persist_window: true, // Persist window size and position
            centered: true,       // Center window on startup if not maximized
            persistence_path: Some(app_data_dir.join("app.ron")),
            ..Default::default()
        };
        eframe::run_native(
            &format!("Dash Evo Tool v{}", VERSION),
            native_options,
            Box::new(|_cc| Ok(Box::new(app::AppState::new()))),
        )
    })
}
extern crate serde;
extern crate serde_json;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug)]
struct BitcoinBlock {
    hash: String,
    height: u64,
    chain: String,
    total: u64,
    fees: u64,
    size: u64,
    vsize: u64,
    ver: u64,
    time: String,
    received_time: String,
    relayed_by: String,
    bits: u64,
    nonce: u64,
    n_tx: u64,
    prev_block: String,
    mrkl_root: String,
    txids: Vec<String>,
    depth: u64,
    prev_block_url: String,
    tx_url: String,
    next_txids: String,
}

fn main() {
    let data = r#"
    {
        "hash": "00000000000000000000cfbeaeb0b5f18dffc118597310d3f87096a2a204b512",
        "height": 877599,
        "chain": "BTC.main",
        "total": 53802306540,
        "fees": 3790551,
        "size": 1443191,
        "vsize": 996671,
        "ver": 537223168,
        "time": "2025-01-03T06:06:44Z",
        "received_time": "2025-01-03T06:07:57.04Z",
        "relayed_by": "104.223.60.58:18336",
        "bits": 386043996,
        "nonce": 779452439,
        "n_tx": 1469,
        "prev_block": "00000000000000000001dc4780b5419dd828c2f2fecfc18a2a8c7387ca960206",
        "mrkl_root": "d0d926d9443a788c713c010a70002ef5ed5e5addde647797bf9056823aeba579",
        "txids": [
            "de6b6cfb392130af7d58f03fba7ff39c011b63b243582adc0c481d44ade94278",
            "384b3bb0a3d92b6e0af22f1fb8c498f323ce48b5971462c1dc2b70e905b6c5b3",
            "9e9771296f8a05fd44e2e1af9884e0b7eb43123ef1110c553e165dafe8f81a04",
            "a36372f70edfee188bd008666d5c149933509597c58c16fbc2fdbc3ecec6573e",
            "6df58358a9d09960747b1a288bda712e69cf273f3ad4c97fc9ae7ddfa07741be",
            "5d2b759752687fe9d852e28f5bd8ce75cd20ce101a784ac893a6c8635153006f",
            "3973d77b5d8d25124cf79ad393210bddbabd7b677f3f3843ac6fdf6055fd1007",
            "33100c82dc10aea34f40193edff4528d34fbe3fbe237d4b007875f2e94359c3b",
            "5bf620573f55da2e63f5f9a54c377156be58f62deaacc591b773bad43772ea45",
            "ef34802b1cf8c6f68974c6e627e9fe683eca8c510b539ffa816b08f510c3c948",
            "af4a2ebbbadc1421a10550086a9fc8a5241ead6156cee5005de58aff81c69c7f",
            "1c08cb2510c76b96d915e70144c5b053b0edd53f5c527157383beb99e9c6bad2",
            "cceae37a7eb5d1c3f462102e5393d85473266713217be55f8407c2824f56f8eb",
            "aa9ba02e13533608d8cb2bc827fccc78a96fab697e19ab04b3dd5853f26c08ad",
            "3819eef3eefce6da6ab7ba74dba30e98a213a9d6a83f1062993c41e00a57f095",
            "faabe0ac04060d05605d6184f0baecc950ae67eba565cd2f4921966e27957dc5",
            "8c5131c9d252b27e14f6c9e423a3b2eb88f3b4df6d16ba21c17db8b5975998d7",
            "5e57b5a2b2ecf3d9d4213cbf4f8a8a7410e729067852492c3cff3aacd641c9aa",
            "90e39e4eff3cf65f32d95cca9b8c63f7eb6039ac94c52d3ad492e9615e50ba37",
            "7fdcd66ff616ae238c73f8281786c3ec7a721264ed3fddc60b3eadd252d6d97c"
        ],
        "depth": 10862,
        "prev_block_url": "https://api.blockcypher.com/v1/btc/main/blocks/00000000000000000001dc4780b5419dd828c2f2fecfc18a2a8c7387ca960206",
        "tx_url": "https://api.blockcypher.com/v1/btc/main/txs/",
        "next_txids": "https://api.blockcypher.com/v1/btc/main/blocks/00000000000000000000cfbeaeb0b5f18dffc118597310d3f87096a2a204b512?txstart=20&limit=20"
    }
    "#;

    let block: BitcoinBlock = serde_json::from_str(data).unwrap();
    println!("{:#?}", block);
}
