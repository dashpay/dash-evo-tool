use chrono::Utc;
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::consensus::{deserialize, serialize};
use dash_sdk::dpp::dashcore::network::constants::ServiceFlags;
use dash_sdk::dpp::dashcore::network::message::{NetworkMessage, RawNetworkMessage};
use dash_sdk::dpp::dashcore::network::message_qrinfo::QRInfo;
use dash_sdk::dpp::dashcore::network::message_sml::{GetMnListDiff, MnListDiff};
use dash_sdk::dpp::dashcore::network::{Address, message_network, message_qrinfo};
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};
use sha2::{Digest, Sha256};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub struct CoreP2PHandler {
    pub network: Network,
    pub port: u16,
    pub stream: TcpStream,
    pub handshake_success: bool,
}

/// Dash P2P header length in bytes
const HEADER_LENGTH: usize = 24;

/// Maximum message payload size (e.g. 0x02000000 bytes)
const MAX_MSG_LENGTH: usize = 0x02000000;

/// Compute double-SHA256 on the given data.
fn double_sha256(data: &[u8]) -> [u8; 32] {
    let hash1 = Sha256::digest(data);
    let hash2 = Sha256::digest(hash1);
    let mut result = [0u8; 32];
    result.copy_from_slice(&hash2);
    result
}

#[derive(Debug)]
enum ReadMessageError {
    Transient,
    Fatal(String),
}

impl CoreP2PHandler {
    pub fn new(network: Network, use_port: Option<u16>) -> Result<CoreP2PHandler, String> {
        let port = use_port.unwrap_or(match network {
            Network::Dash => 9999,     // Dash Mainnet default
            Network::Testnet => 19999, // Dash Testnet default
            Network::Devnet => 29999,  // Dash Devnet default
            Network::Regtest => 29999, // Dash Regtest default
            _ => return Err(format!("Unsupported network type: {:?}", network)),
        });
        let stream = TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port)
                .parse()
                .map_err(|e| format!("Invalid address: {}", e))?,
            Duration::from_secs(5),
        )
        .map_err(|e| format!("Failed to connect: {}", e))?;
        // Set per-socket timeouts so reads/writes don't block forever
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("set_read_timeout failed: {}", e))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("set_write_timeout failed: {}", e))?;
        println!("Connected to Dash Core at 127.0.0.1:{}", port);
        Ok(CoreP2PHandler {
            network,
            port,
            stream,
            handshake_success: false,
        })
    }

    /// Sends a network message over the provided stream and waits for a response.
    pub fn send_dml_request_message(
        &mut self,
        network_message: NetworkMessage,
    ) -> Result<MnListDiff, String> {
        if !self.handshake_success {
            self.handshake()?;
        }
        let stream = &mut self.stream;
        let raw_message = RawNetworkMessage {
            magic: self.network.magic(),
            payload: network_message,
        };
        let encoded_message = serialize(&raw_message);
        stream
            .write_all(&encoded_message)
            .map_err(|e| format!("Failed to send message: {}", e))?;
        println!("Sent getmnlistdiff message to Dash Core");

        let (mut command, mut payload);
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        loop {
            if start_time.elapsed() > timeout {
                return Err("Timeout waiting for mnlistdiff message".to_string());
            }
            match self.read_message() {
                Ok((c, p)) => {
                    command = c;
                    payload = p;
                }
                Err(ReadMessageError::Transient) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(ReadMessageError::Fatal(e)) => return Err(e),
            }
            if command == "mnlistdiff" {
                println!("Got mnlistdiff message");
                break;
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }

        // let log_file_path = app_user_data_file_path("DML.DAT").expect("should create DML.dat");
        // let mut log_file = match std::fs::File::create(log_file_path) {
        //     Ok(file) => file,
        //     Err(e) => panic!("Failed to create log file: {:?}", e),
        // };
        //
        // log_file.write_all(&payload).expect("expected to write");

        let response_message: RawNetworkMessage = deserialize(&payload).map_err(|e| {
            format!(
                "Failed to deserialize response: {}, payload {}",
                e,
                hex::encode(payload)
            )
        })?;

        match response_message.payload {
            NetworkMessage::MnListDiff(diff) => Ok(diff),
            network_message => Err(format!(
                "Unexpected response type, expected MnListDiff, got {:?}",
                network_message
            )),
        }
    }

    /// Sends a network message over the provided stream and waits for a response.
    pub fn send_qr_info_request_message(
        &mut self,
        network_message: NetworkMessage,
    ) -> Result<QRInfo, String> {
        if !self.handshake_success {
            self.handshake()?;
        }
        let stream = &mut self.stream;
        let raw_message = RawNetworkMessage {
            magic: self.network.magic(),
            payload: network_message,
        };
        let encoded_message = serialize(&raw_message);
        stream
            .write_all(&encoded_message)
            .map_err(|e| format!("Failed to send message: {}", e))?;
        println!("Sent qr info request message to Dash Core");

        let (mut command, mut payload);
        // QRInfo on mainnet can take noticeably longer to prepare.
        // Temporarily increase socket read timeout and our overall wait.
        let (socket_timeout, overall_timeout) = match self.network {
            Network::Dash => (Duration::from_secs(60), Duration::from_secs(60)),
            _ => (Duration::from_secs(15), Duration::from_secs(15)),
        };
        let previous_socket_timeout = self
            .stream
            .read_timeout()
            .map_err(|e| format!("get_read_timeout failed: {}", e))?;
        self.stream
            .set_read_timeout(Some(socket_timeout))
            .map_err(|e| format!("set_read_timeout failed: {}", e))?;
        let start_time = std::time::Instant::now();
        let timeout = overall_timeout;
        loop {
            if start_time.elapsed() > timeout {
                // Restore previous socket timeout before returning
                self.stream
                    .set_read_timeout(previous_socket_timeout)
                    .map_err(|e| format!("restore set_read_timeout failed: {}", e))?;
                return Err("Timeout waiting for qrinfo message".to_string());
            }
            match self.read_message() {
                Ok((c, p)) => {
                    command = c;
                    payload = p;
                }
                Err(ReadMessageError::Transient) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(ReadMessageError::Fatal(e)) => return Err(e),
            }
            if command == "qrinfo" {
                println!("Got qrinfo message");
                // Restore previous socket timeout
                self.stream
                    .set_read_timeout(previous_socket_timeout)
                    .map_err(|e| format!("restore set_read_timeout failed: {}", e))?;
                break;
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }

        // let log_file_path = app_user_data_file_path("QR_INFO.DAT").expect("should create DML.dat");
        // let mut log_file = match std::fs::File::create(log_file_path) {
        //     Ok(file) => file,
        //     Err(e) => panic!("Failed to create log file: {:?}", e),
        // };
        //
        // log_file.write_all(&payload).expect("expected to write");

        let response_message: RawNetworkMessage = deserialize(&payload).map_err(|e| {
            format!(
                "Failed to deserialize response: {}, payload {}",
                e,
                hex::encode(payload)
            )
        })?;

        match response_message.payload {
            NetworkMessage::QRInfo(qr_info) => {
                // let bytes = serialize(&qr_info);
                // let log_file_path = app_user_data_file_path("QR_INFO.DAT").expect("should create DML.dat");
                // let mut log_file = match std::fs::File::create(log_file_path) {
                //     Ok(file) => file,
                //     Err(e) => panic!("Failed to create log file: {:?}", e),
                // };
                //
                // log_file.write_all(&bytes).expect("expected to write");
                Ok(qr_info)
            }
            network_message => Err(format!(
                "Unexpected response type, expected QrInfo, got {:?}",
                network_message
            )),
        }
    }

    // Note: get_dml_diff and get_qr_info are already defined above (lines ~351 and ~364)
    /// Perform the handshake (version/verack exchange) with the peer.
    pub fn handshake(&mut self) -> Result<(), String> {
        let mut rng = StdRng::from_entropy();

        // Build a version message.
        let version_msg = NetworkMessage::Version(message_network::VersionMessage {
            version: 70235,
            services: ServiceFlags::NONE,
            timestamp: Utc::now().timestamp(),
            receiver: Address {
                services: ServiceFlags::BLOOM,
                address: Default::default(),
                port: self.stream.peer_addr().map_err(|e| e.to_string())?.port(),
            },
            sender: Address {
                services: ServiceFlags::NONE,
                address: Default::default(),
                port: self.stream.local_addr().map_err(|e| e.to_string())?.port(),
            },
            nonce: rng.r#gen(),
            user_agent: "/dash-evo-tool:0.9/".to_string(),
            start_height: 0,
            relay: false,
            mn_auth_challenge: rng.r#gen(),
            masternode_connection: false,
        });

        // Wrap it in a raw message.
        let raw_version = RawNetworkMessage {
            magic: self.network.magic(),
            payload: version_msg,
        };
        let encoded_version = serialize(&raw_version);
        self.stream
            .write_all(&encoded_version)
            .map_err(|e| format!("Failed to send version: {}", e))?;
        println!("Sent version message");

        thread::sleep(Duration::from_millis(50));

        // Read and process incoming messages until handshake is complete.
        self.run_handshake_loop()?;
        self.handshake_success = true;
        Ok(())
    }

    fn read_message(&mut self) -> Result<(String, Vec<u8>), ReadMessageError> {
        let mut header_buf = [0u8; HEADER_LENGTH];
        // Read the header.
        self.stream
            .read_exact(&mut header_buf)
            .map_err(|e| match e.kind() {
                ErrorKind::WouldBlock | ErrorKind::TimedOut => ReadMessageError::Transient,
                _ => ReadMessageError::Fatal(format!("Error reading header: {}", e)),
            })?;

        // If the first 4 bytes don't match our network magic, shift until we do.
        const MAX_SYNC_ATTEMPTS: usize = 1024; // Prevent reading more than 1KB looking for magic
        let mut sync_attempts = 0;
        while u32::from_le_bytes(header_buf[0..4].try_into().unwrap()) != self.network.magic() {
            sync_attempts += 1;
            if sync_attempts > MAX_SYNC_ATTEMPTS {
                return Err(ReadMessageError::Fatal(
                    "Failed to find network magic in stream".to_string(),
                ));
            }
            // Shift left by one byte.
            for i in 0..HEADER_LENGTH - 1 {
                header_buf[i] = header_buf[i + 1];
            }
            // Read one more byte.
            let mut one_byte = [0u8; 1];
            self.stream
                .read_exact(&mut one_byte)
                .map_err(|e| match e.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => ReadMessageError::Transient,
                    _ => {
                        ReadMessageError::Fatal(format!("Error reading while syncing magic: {}", e))
                    }
                })?;
            header_buf[HEADER_LENGTH - 1] = one_byte[0];
        }

        // Extract the command.
        let command_bytes = &header_buf[4..16];
        let command = String::from_utf8_lossy(command_bytes)
            .trim_matches('\0')
            .to_string();

        // Payload length (little-endian u32)
        let payload_len_u32 = u32::from_le_bytes(header_buf[16..20].try_into().unwrap());
        if payload_len_u32 > MAX_MSG_LENGTH as u32 {
            return Err(ReadMessageError::Fatal(format!(
                "Payload length {} exceeds maximum",
                payload_len_u32
            )));
        }
        let payload_len = payload_len_u32 as usize;

        // Expected checksum.
        let expected_checksum = &header_buf[20..24];

        // Read the payload.
        let mut payload_buf = vec![0u8; payload_len];
        self.stream
            .read_exact(&mut payload_buf)
            .map_err(|e| match e.kind() {
                ErrorKind::WouldBlock | ErrorKind::TimedOut => ReadMessageError::Transient,
                _ => ReadMessageError::Fatal(format!("Error reading payload: {}", e)),
            })?;

        // Compute and verify checksum.
        let computed_checksum = &double_sha256(&payload_buf)[0..4];
        if computed_checksum != expected_checksum {
            return Err(ReadMessageError::Fatal(format!(
                "Checksum mismatch for {}: computed {:x?}, expected {:x?}, payload is {:x?}",
                command, computed_checksum, expected_checksum, payload_buf
            )));
        }
        let mut total_buf = header_buf.to_vec();
        total_buf.append(&mut payload_buf);
        Ok((command, total_buf))
    }

    /// The handshake loop: read messages until we complete the version/verack exchange.
    fn run_handshake_loop(&mut self) -> Result<(), String> {
        // Expect a version message from the peer, with a timeout.
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        let (command, payload) = loop {
            if start_time.elapsed() > timeout {
                return Err("Timeout waiting for version message".to_string());
            }
            match self.read_message() {
                Ok(res) => break res,
                Err(ReadMessageError::Transient) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(ReadMessageError::Fatal(e)) => return Err(e),
            }
        };
        if command != "version" {
            return Err(format!("Expected version message, got {}", command));
        }
        // Deserialize the version message payload.
        let raw: RawNetworkMessage = deserialize(&payload)
            .map_err(|e| format!("Failed to deserialize version payload: {}", e))?;
        match raw.payload {
            NetworkMessage::Version(peer_version) => {
                println!("Received peer version: {:?}", peer_version);
            }
            _ => {
                return Err("Deserialized message was not a version message".to_string());
            }
        }

        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        loop {
            if start_time.elapsed() > timeout {
                return Err("Timeout waiting for verack message".to_string());
            }
            let (command, _) = match self.read_message() {
                Ok(res) => res,
                Err(ReadMessageError::Transient) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(ReadMessageError::Fatal(e)) => return Err(e),
            };
            if command == "verack" {
                println!("Got verack message");
                break;
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }

        // Send verack.
        let verack_msg = NetworkMessage::Verack;
        let raw_verack = RawNetworkMessage {
            magic: self.network.magic(),
            payload: verack_msg,
        };
        let encoded_verack = serialize(&raw_verack);
        self.stream
            .write_all(&encoded_verack)
            .map_err(|e| format!("Failed to send verack: {}", e))?;

        println!("Sent verack message");
        Ok(())
    }

    /// Sends a `GetMnListDiff` request after completing the handshake.
    pub fn get_dml_diff(
        &mut self,
        base_block_hash: BlockHash,
        block_hash: BlockHash,
    ) -> Result<MnListDiff, String> {
        let get_mnlist_diff_msg = NetworkMessage::GetMnListD(GetMnListDiff {
            base_block_hash,
            block_hash,
        });
        self.send_dml_request_message(get_mnlist_diff_msg)
    }

    /// Sends a `GetMnListDiff` request after completing the handshake.
    pub fn get_qr_info(
        &mut self,
        known_block_hashes: Vec<BlockHash>,
        block_request_hash: BlockHash,
    ) -> Result<QRInfo, String> {
        let get_mnlist_diff_msg = NetworkMessage::GetQRInfo(message_qrinfo::GetQRInfo {
            base_block_hashes: known_block_hashes,
            block_request_hash,
            extra_share: true,
        });
        self.send_qr_info_request_message(get_mnlist_diff_msg)
    }
}
