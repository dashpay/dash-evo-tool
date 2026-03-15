use chrono::Utc;
use dash_sdk::dpp::dashcore::BlockHash;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::consensus::{deserialize, encode, serialize};
use dash_sdk::dpp::dashcore::network::constants::ServiceFlags;
use dash_sdk::dpp::dashcore::network::message::{NetworkMessage, RawNetworkMessage};
use dash_sdk::dpp::dashcore::network::message_qrinfo::QRInfo;
use dash_sdk::dpp::dashcore::network::message_sml::{GetMnListDiff, MnListDiff};
use dash_sdk::dpp::dashcore::network::{Address, message_network, message_qrinfo};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use sha2::{Digest, Sha256};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use thiserror::Error;

/// Errors from peer-to-peer communication with Dash Core.
#[derive(Debug, Error)]
pub enum P2PError {
    /// TCP connection or socket I/O failed.
    #[error(
        "Could not communicate with Dash Core over the local network. Check that Dash Core is running."
    )]
    Io {
        #[from]
        source: std::io::Error,
    },

    /// Failed to deserialize a P2P protocol message.
    #[error("Received an unreadable message from Dash Core. The node may need to be updated.")]
    Deserialization {
        #[from]
        source: encode::Error,
    },

    /// Timed out waiting for a response from Dash Core.
    #[error("Dash Core did not respond in time. Please retry.")]
    Timeout,

    /// Received an unexpected message type.
    #[error("Received an unexpected response from Dash Core. Please retry.")]
    UnexpectedMessage {
        /// The command name that was received instead of the expected one.
        received: String,
    },

    /// Network type is not supported for P2P connections.
    #[error("This network type does not support direct peer connections.")]
    UnsupportedNetwork,

    /// A protocol-level error (bad checksum, oversized message, unexpected format).
    #[error("Received a malformed message from Dash Core. The node may need to be updated.")]
    ProtocolError { detail: String },
}

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

/// Internal marker for non-fatal (retryable) read errors.
#[derive(Debug)]
enum ReadMessageError {
    Transient,
    Fatal(P2PError),
}

impl CoreP2PHandler {
    pub fn new(network: Network, use_port: Option<u16>) -> Result<CoreP2PHandler, P2PError> {
        let port = use_port.unwrap_or(match network {
            Network::Dash => 9999,
            Network::Testnet => 19999,
            Network::Devnet => 29999,
            Network::Regtest => 29999,
            _ => return Err(P2PError::UnsupportedNetwork),
        });
        let stream = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}")
                .parse()
                .map_err(|_| P2PError::Io {
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "invalid socket address",
                    ),
                })?,
            Duration::from_secs(5),
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        tracing::info!("Connected to Dash Core at 127.0.0.1:{}", port);
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
    ) -> Result<MnListDiff, P2PError> {
        if !self.handshake_success {
            self.handshake()?;
        }
        let stream = &mut self.stream;
        let raw_message = RawNetworkMessage {
            magic: self.network.magic(),
            payload: network_message,
        };
        let encoded_message = serialize(&raw_message);
        stream.write_all(&encoded_message)?;
        tracing::debug!("Sent getmnlistdiff message to Dash Core");

        let (mut command, mut payload);
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        loop {
            if start_time.elapsed() > timeout {
                return Err(P2PError::Timeout);
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
                tracing::debug!("Got mnlistdiff message");
                break;
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }

        let response_message: RawNetworkMessage = deserialize(&payload)?;

        match response_message.payload {
            NetworkMessage::MnListDiff(diff) => Ok(diff),
            msg => Err(P2PError::UnexpectedMessage {
                received: format!("{msg:?}"),
            }),
        }
    }

    /// Sends a network message over the provided stream and waits for a response.
    pub fn send_qr_info_request_message(
        &mut self,
        network_message: NetworkMessage,
    ) -> Result<QRInfo, P2PError> {
        if !self.handshake_success {
            self.handshake()?;
        }
        let stream = &mut self.stream;
        let raw_message = RawNetworkMessage {
            magic: self.network.magic(),
            payload: network_message,
        };
        let encoded_message = serialize(&raw_message);
        stream.write_all(&encoded_message)?;
        tracing::debug!("Sent qr info request message to Dash Core");

        let (mut command, mut payload);
        // QRInfo on mainnet can take noticeably longer to prepare.
        // Temporarily increase socket read timeout and our overall wait.
        let (socket_timeout, overall_timeout) = match self.network {
            Network::Dash => (Duration::from_secs(60), Duration::from_secs(60)),
            _ => (Duration::from_secs(15), Duration::from_secs(15)),
        };
        let previous_socket_timeout = self.stream.read_timeout()?;
        self.stream.set_read_timeout(Some(socket_timeout))?;
        let start_time = std::time::Instant::now();
        let timeout = overall_timeout;
        loop {
            if start_time.elapsed() > timeout {
                self.stream.set_read_timeout(previous_socket_timeout)?;
                return Err(P2PError::Timeout);
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
                tracing::debug!("Got qrinfo message");
                self.stream.set_read_timeout(previous_socket_timeout)?;
                break;
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }

        let response_message: RawNetworkMessage = deserialize(&payload)?;

        match response_message.payload {
            NetworkMessage::QRInfo(qr_info) => Ok(qr_info),
            msg => Err(P2PError::UnexpectedMessage {
                received: format!("{msg:?}"),
            }),
        }
    }

    // Note: get_dml_diff and get_qr_info are already defined above (lines ~351 and ~364)
    /// Perform the handshake (version/verack exchange) with the peer.
    pub fn handshake(&mut self) -> Result<(), P2PError> {
        let mut rng = StdRng::from_os_rng();

        // Build a version message.
        let version_msg = NetworkMessage::Version(message_network::VersionMessage {
            version: 70235,
            services: ServiceFlags::NONE,
            timestamp: Utc::now().timestamp(),
            receiver: Address {
                services: ServiceFlags::BLOOM,
                address: Default::default(),
                port: self.stream.peer_addr()?.port(),
            },
            sender: Address {
                services: ServiceFlags::NONE,
                address: Default::default(),
                port: self.stream.local_addr()?.port(),
            },
            nonce: rng.random(),
            user_agent: "/dash-evo-tool:0.9/".to_string(),
            start_height: 0,
            relay: false,
            mn_auth_challenge: rng.random(),
            masternode_connection: false,
        });

        // Wrap it in a raw message.
        let raw_version = RawNetworkMessage {
            magic: self.network.magic(),
            payload: version_msg,
        };
        let encoded_version = serialize(&raw_version);
        self.stream.write_all(&encoded_version)?;
        tracing::debug!("Sent version message");

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
                _ => ReadMessageError::Fatal(P2PError::Io { source: e }),
            })?;

        // If the first 4 bytes don't match our network magic, shift until we do.
        const MAX_SYNC_ATTEMPTS: usize = 1024; // Prevent reading more than 1KB looking for magic
        let mut sync_attempts = 0;
        while u32::from_le_bytes(header_buf[0..4].try_into().unwrap()) != self.network.magic() {
            sync_attempts += 1;
            if sync_attempts > MAX_SYNC_ATTEMPTS {
                return Err(ReadMessageError::Fatal(P2PError::ProtocolError {
                    detail: "failed to find network magic in stream".to_string(),
                }));
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
                    _ => ReadMessageError::Fatal(P2PError::Io { source: e }),
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
            return Err(ReadMessageError::Fatal(P2PError::ProtocolError {
                detail: format!("payload length {payload_len_u32} exceeds maximum"),
            }));
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
                _ => ReadMessageError::Fatal(P2PError::Io { source: e }),
            })?;

        // Compute and verify checksum.
        let computed_checksum = &double_sha256(&payload_buf)[0..4];
        if computed_checksum != expected_checksum {
            return Err(ReadMessageError::Fatal(P2PError::ProtocolError {
                detail: format!(
                    "checksum mismatch for {command}: computed {computed_checksum:x?}, expected {expected_checksum:x?}"
                ),
            }));
        }
        let mut total_buf = header_buf.to_vec();
        total_buf.append(&mut payload_buf);
        Ok((command, total_buf))
    }

    /// The handshake loop: read messages until we complete the version/verack exchange.
    fn run_handshake_loop(&mut self) -> Result<(), P2PError> {
        // Expect a version message from the peer, with a timeout.
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        let (command, payload) = loop {
            if start_time.elapsed() > timeout {
                return Err(P2PError::Timeout);
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
            return Err(P2PError::UnexpectedMessage { received: command });
        }
        // Deserialize the version message payload.
        let raw: RawNetworkMessage = deserialize(&payload)?;
        match raw.payload {
            NetworkMessage::Version(peer_version) => {
                tracing::debug!("Received peer version: {:?}", peer_version);
            }
            msg => {
                return Err(P2PError::UnexpectedMessage {
                    received: format!("{msg:?}"),
                });
            }
        }

        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        loop {
            if start_time.elapsed() > timeout {
                return Err(P2PError::Timeout);
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
                tracing::debug!("Got verack message");
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
        self.stream.write_all(&encoded_verack)?;

        tracing::debug!("Sent verack message");
        Ok(())
    }

    /// Sends a `GetMnListDiff` request after completing the handshake.
    pub fn get_dml_diff(
        &mut self,
        base_block_hash: BlockHash,
        block_hash: BlockHash,
    ) -> Result<MnListDiff, P2PError> {
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
    ) -> Result<QRInfo, P2PError> {
        let get_mnlist_diff_msg = NetworkMessage::GetQRInfo(message_qrinfo::GetQRInfo {
            base_block_hashes: known_block_hashes,
            block_request_hash,
            extra_share: true,
        });
        self.send_qr_info_request_message(get_mnlist_diff_msg)
    }
}
