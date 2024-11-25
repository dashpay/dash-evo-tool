use crossbeam_channel::Sender;
use dash_sdk::dpp::dashcore::consensus::Decodable;
use dash_sdk::dpp::dashcore::{Block, InstantLock, Network, Transaction};
use dash_sdk::dpp::prelude::CoreBlockHeight;
use std::error::Error;
use std::io::Cursor;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

#[cfg(not(target_os = "windows"))]
use image::EncodableLayout;
#[cfg(not(target_os = "windows"))]
use zmq::Context;

#[cfg(target_os = "windows")]
use futures::StreamExt;
#[cfg(target_os = "windows")]
use tokio::runtime::Runtime;
#[cfg(target_os = "windows")]
use tokio::time::timeout;
#[cfg(target_os = "windows")]
use zeromq::{Socket, SocketRecv, SubSocket};

pub struct CoreZMQListener {
    should_stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

pub enum ZMQMessage {
    ISLockedTransaction(Transaction, InstantLock),
    ChainLockedBlock(Block),
    ChainLockedLockedTransaction(Transaction, CoreBlockHeight),
}

#[derive(Debug)]
pub enum ZMQConnectionEvent {
    Connected,
    Disconnected,
}

#[cfg(not(target_os = "windows"))]
pub const IS_LOCK_SIG_MSG: &[u8; 12] = b"rawtxlocksig";
#[cfg(not(target_os = "windows"))]
pub const CHAIN_LOCKED_BLOCK_MSG: &[u8; 12] = b"rawchainlock";

#[cfg(target_os = "windows")]
pub const IS_LOCK_SIG_MSG: &str = "rawtxlocksig";
#[cfg(target_os = "windows")]
pub const CHAIN_LOCKED_BLOCK_MSG: &str = "rawchainlock";

impl CoreZMQListener {
    #[cfg(not(target_os = "windows"))]
    pub fn spawn_listener(
        network: Network,
        endpoint: &str,
        sender: mpsc::Sender<(ZMQMessage, Network)>,
        tx_zmq_status: Option<Sender<ZMQConnectionEvent>>,
    ) -> Result<Self, Box<dyn Error>> {
        let should_stop = Arc::new(AtomicBool::new(false));
        let endpoint = endpoint.to_string();
        let should_stop_clone = Arc::clone(&should_stop);
        let sender_clone = sender.clone();

        let handle = thread::spawn(move || {
            // Create the socket inside the thread.
            let context = Context::new();
            let socket = context.socket(zmq::SUB).expect("Failed to create socket");

            // Set heartbeat options
            socket
                .set_heartbeat_ivl(5000)
                .expect("Failed to set heartbeat interval"); // Send a heartbeat every 5000 ms
            socket
                .set_heartbeat_timeout(10000)
                .expect("Failed to set heartbeat timeout"); // Timeout after 10000 ms without response

            let monitor_addr = "inproc://socket-monitor";
            socket
                .monitor(monitor_addr, zmq::SocketEvent::ALL as i32)
                .expect("Failed to monitor socket");

            // Create the PAIR socket for monitoring
            let monitor_socket = context
                .socket(zmq::PAIR)
                .expect("Failed to create monitor socket");
            monitor_socket
                .connect(monitor_addr)
                .expect("Failed to connect monitor socket");

            // Connect to the zmqpubhashtxlock endpoint.
            socket.connect(&endpoint).expect("Failed to connect");

            // Subscribe to the "rawtxlocksig" events.
            socket
                .set_subscribe(IS_LOCK_SIG_MSG)
                .expect("Failed to subscribe to rawtxlocksig");

            // Subscribe to the "rawtxlocksig" events.
            socket
                .set_subscribe(CHAIN_LOCKED_BLOCK_MSG)
                .expect("Failed to subscribe to rawchainlock");

            println!("Subscribed to ZMQ at {}", endpoint);

            let mut items = [
                socket.as_poll_item(zmq::POLLIN),
                monitor_socket.as_poll_item(zmq::POLLIN),
            ];

            while !should_stop_clone.load(Ordering::SeqCst) {
                zmq::poll(&mut items, -1).expect("Failed to poll sockets");

                if items[0].is_readable() {
                    // Handle messages from the SUB socket
                    // Receive the topic part of the message
                    let mut topic_message = zmq::Message::new();

                    // Use non-blocking receive with DONTWAIT.
                    match socket.recv(&mut topic_message, zmq::DONTWAIT) {
                        Ok(_) => {
                            let topic = topic_message.as_str().unwrap_or("");
                            let has_more = socket.get_rcvmore().unwrap_or(false);

                            if has_more {
                                // Receive the data part of the message
                                let mut data_message = zmq::Message::new();
                                if let Err(e) = socket.recv(&mut data_message, 0) {
                                    eprintln!("Error receiving data part: {}", e);
                                    continue;
                                }

                                let data_bytes = data_message.as_bytes();

                                match topic {
                                    "rawchainlock" => {
                                        // println!("Received raw chain locked block:");
                                        // println!("Data (hex): {}", hex::encode(data_bytes));

                                        // Create a cursor over the data_bytes
                                        let mut cursor = Cursor::new(data_bytes);

                                        // Deserialize the LLMQChainLock
                                        match Block::consensus_decode(&mut cursor) {
                                            Ok(block) => {
                                                // Send the ChainLock and Network back to the main thread
                                                if let Err(e) = sender_clone.send((
                                                    ZMQMessage::ChainLockedBlock(block),
                                                    network,
                                                )) {
                                                    eprintln!(
                                                        "Error sending data to main thread: {}",
                                                        e
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "Error deserializing chain locked block: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    "rawtxlocksig" => {
                                        // println!("Received rawtxlocksig for InstantSend:");
                                        // println!("Data (hex): {}", hex::encode(data_bytes));

                                        // Create a cursor over the data_bytes
                                        let mut cursor = Cursor::new(data_bytes);

                                        // Deserialize the transaction
                                        match Transaction::consensus_decode(&mut cursor) {
                                            Ok(tx) => {
                                                // Deserialize the InstantLock from the remaining bytes
                                                match InstantLock::consensus_decode(&mut cursor) {
                                                    Ok(islock) => {
                                                        // Send the Transaction, InstantLock, and Network back to the main thread
                                                        if let Err(e) = sender_clone.send((
                                                            ZMQMessage::ISLockedTransaction(
                                                                tx, islock,
                                                            ),
                                                            network,
                                                        )) {
                                                            eprintln!(
                                                                "Error sending data to main thread: {}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        eprintln!(
                                                            "Error deserializing InstantLock: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("Error deserializing transaction: {}", e);
                                            }
                                        }
                                    }
                                    _ => {
                                        println!("Received unknown topic: {}", topic);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if e == zmq::Error::EAGAIN {
                                // No message received, sleep briefly.
                                thread::sleep(Duration::from_millis(100));
                                continue;
                            } else {
                                eprintln!("Error receiving message: {}", e);
                                break;
                            }
                        }
                    }
                }

                if items[1].is_readable() {
                    let mut event_msg = zmq::Message::new();
                    monitor_socket
                        .recv(&mut event_msg, 0)
                        .expect("Failed to receive event message");

                    let mut addr_msg = zmq::Message::new();
                    monitor_socket
                        .recv(&mut addr_msg, 0)
                        .expect("Failed to receive address message");

                    let data: &[u8] = event_msg.as_ref(); // Explicitly annotate the type
                    if data.len() >= 6 {
                        let event_number = u16::from_le_bytes([data[0], data[1]]);
                        let endpoint = addr_msg.as_str().unwrap_or("");

                        match zmq::SocketEvent::from_raw(event_number) {
                            zmq::SocketEvent::CONNECTED => {
                                if let Some(ref tx) = tx_zmq_status {
                                    println!("ZMQ Socket connected to {}", endpoint);
                                    tx.send(ZMQConnectionEvent::Connected)
                                        .expect("Failed to send connected event");
                                }
                                // Connection is successful
                            }
                            zmq::SocketEvent::DISCONNECTED => {
                                if let Some(ref tx) = tx_zmq_status {
                                    println!("ZMQ Socket disconnected from {}", endpoint);
                                    tx.send(ZMQConnectionEvent::Disconnected)
                                        .expect("Failed to send connected event");
                                }
                                // Connection is lost
                            }
                            // Handle other events as needed
                            _ => {}
                        }
                    } else {
                        println!("Invalid event message received");
                    }
                }
            }

            println!("Listener is stopping.");
            // Clean up socket (optional, as it will be dropped here).
            drop(socket);
        });

        Ok(CoreZMQListener {
            should_stop,
            handle: Some(handle),
        })
    }

    #[cfg(target_os = "windows")]
    pub fn spawn_listener(
        network: Network,
        endpoint: &str,
        sender: mpsc::Sender<(ZMQMessage, Network)>,
        tx_zmq_status: Option<Sender<ZMQConnectionEvent>>,
    ) -> Result<Self, Box<dyn Error>> {
        let should_stop = Arc::new(AtomicBool::new(false));
        let endpoint = endpoint.to_string();
        let should_stop_clone = Arc::clone(&should_stop);
        let sender_clone = sender.clone();

        let handle = thread::spawn(move || {
            // Create the runtime inside the thread.
            let rt = Runtime::new().unwrap();
            rt.block_on(async move {
                // Create the socket inside the async context.
                let mut socket = SubSocket::new();

                // Connect to the endpoint
                socket
                    .connect(&endpoint)
                    .await
                    .expect("Failed to connect");

                // Subscribe to the "rawtxlocksig" events.
                socket
                    .subscribe(IS_LOCK_SIG_MSG)
                    .await
                    .expect("Failed to subscribe to rawtxlocksig");

                // Subscribe to the "rawchainlock" events.
                socket
                    .subscribe(CHAIN_LOCKED_BLOCK_MSG)
                    .await
                    .expect("Failed to subscribe to rawchainlock");

                println!("Subscribed to ZMQ at {}", endpoint);
                while !should_stop_clone.load(Ordering::SeqCst) {
                    match timeout(Duration::from_secs(30), socket.recv()).await {
                        Ok(Ok(msg)) => {
                            // Process the message
                            // Access frames using msg.get(n)
                            if let Some(topic_frame) = msg.get(0) {
                                let topic = String::from_utf8_lossy(topic_frame).to_string();

                                if let Some(data_frame) = msg.get(1) {
                                    let data_bytes = data_frame;

                                    match topic.as_str() {
                                        "rawchainlock" => {
                                            // Deserialize the Block
                                            let mut cursor = Cursor::new(data_bytes);
                                            match Block::consensus_decode(&mut cursor) {
                                                Ok(block) => {
                                                    if let Some(ref tx) = tx_zmq_status {
                                                        // ZMQ refresh socket connected status
                                                        tx.send(ZMQConnectionEvent::Connected)
                                                            .expect("Failed to send connected event");
                                                    }
                                                    if let Err(e) = sender_clone.send((
                                                        ZMQMessage::ChainLockedBlock(block),
                                                        network,
                                                    )) {
                                                        eprintln!(
                                                            "Error sending data to main thread: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                                Err(e) => {
                                                    eprintln!(
                                                        "Error deserializing chain locked block: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        "rawtxlocksig" => {
                                            // Deserialize the Transaction and InstantLock
                                            let mut cursor = Cursor::new(data_bytes);
                                            match Transaction::consensus_decode(&mut cursor) {
                                                Ok(tx) => {
                                                    match InstantLock::consensus_decode(&mut cursor)
                                                    {
                                                        Ok(islock) => {
                                                            if let Some(ref tx) = tx_zmq_status {
                                                                // ZMQ refresh socket connected status
                                                                tx.send(ZMQConnectionEvent::Connected)
                                                                    .expect("Failed to send connected event");
                                                            }
                                                            if let Err(e) = sender_clone.send((
                                                                ZMQMessage::ISLockedTransaction(
                                                                    tx, islock,
                                                                ),
                                                                network,
                                                            )) {
                                                                eprintln!(
                                                                    "Error sending data to main thread: {}",
                                                                    e
                                                                );
                                                            }
                                                        }
                                                        Err(e) => {
                                                            eprintln!(
                                                                "Error deserializing InstantLock: {}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    eprintln!(
                                                        "Error deserializing transaction: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        _ => {
                                            println!("Received unknown topic: {}", topic);
                                        }
                                    }
                                }
                            }
                        },
                        Ok(Err(e)) => {
                            // Handle recv error
                            eprintln!("Error receiving message: {}", e);
                            // Sleep briefly before retrying
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        },
                        Err(_) => {
                            // Timeout occurred, handle disconnection
                            if let Some(ref tx) = tx_zmq_status {
                                tx.send(ZMQConnectionEvent::Disconnected)
                                    .expect("Failed to send connected event");
                            }
                        },
                    }
                }

                println!("Listener is stopping.");
                // The socket will be dropped here
            });
        });

        Ok(CoreZMQListener {
            should_stop,
            handle: Some(handle),
        })
    }

    /// Stops the listener by signaling the thread and waiting for it to finish.
    pub fn stop(&mut self) {
        self.should_stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("Failed to join listener thread");
        }
    }
}
