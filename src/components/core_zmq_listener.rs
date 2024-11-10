use dash_sdk::dpp::dashcore::consensus::Decodable;
use dash_sdk::dpp::dashcore::{Block, InstantLock, Network, Transaction};
use dash_sdk::dpp::prelude::CoreBlockHeight;
use image::EncodableLayout;
use std::error::Error;
use std::io::Cursor;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;
use crossbeam_channel::Sender;
use zmq::Context;

pub struct CoreZMQListener {
    should_stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

pub enum ZMQMessage {
    ISLockedTransaction(Transaction, InstantLock),
    ChainLockedBlock(Block),
    ChainLockedLockedTransaction(Transaction, CoreBlockHeight),
}

pub enum ZMQConnectionEvent {
    Connected,
    Disconnected,
}

pub const IS_LOCK_SIG_MSG: &[u8; 12] = b"rawtxlocksig";
pub const CHAIN_LOCKED_BLOCK_MSG: &[u8; 12] = b"rawchainlock";

impl CoreZMQListener {
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
            socket.set_heartbeat_ivl(5000).expect("Failed to set heartbeat interval");      // Send a heartbeat every 5000 ms
            socket.set_heartbeat_timeout(10000).expect("Failed to set heartbeat timeout");  // Timeout after 10000 ms without response

            let monitor_addr = "inproc://socket-monitor";
            socket.monitor(monitor_addr, zmq::SocketEvent::ALL as i32)
                .expect("Failed to monitor socket");

            // Create the PAIR socket for monitoring
            let monitor_socket = context.socket(zmq::PAIR).expect("Failed to create monitor socket");
            monitor_socket.connect(monitor_addr).expect("Failed to connect monitor socket");

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
                                        println!("Received raw chain locked block:");
                                        println!("Data (hex): {}", hex::encode(data_bytes));

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
                                        println!("Received rawtxlocksig for InstantSend:");
                                        println!("Data (hex): {}", hex::encode(data_bytes));

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
                                                            ZMQMessage::ISLockedTransaction(tx, islock),
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
                    monitor_socket.recv(&mut event_msg, 0).expect("Failed to receive event message");

                    let mut addr_msg = zmq::Message::new();
                    monitor_socket.recv(&mut addr_msg, 0).expect("Failed to receive address message");

                    let data = event_msg.as_ref();
                    if data.len() >= 6 {
                        let event_number = u16::from_le_bytes([data[0], data[1]]);
                        let endpoint = addr_msg.as_str().unwrap_or("");

                        match zmq::SocketEvent::from_raw(event_number) {
                            zmq::SocketEvent::CONNECTED => {
                                println!("Socket connected to {}", endpoint);
                                if let Some(ref tx) = tx_zmq_status {
                                    tx.send(ZMQConnectionEvent::Connected).expect("Failed to send connected event");
                                }
                                // Connection is successful
                            }
                            zmq::SocketEvent::DISCONNECTED => {
                                println!("Socket disconnected from {}", endpoint);
                                if let Some(ref tx) = tx_zmq_status {
                                    tx.send(ZMQConnectionEvent::Disconnected).expect("Failed to send connected event");
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

    /// Stops the listener by signaling the thread and waiting for it to finish.
    pub fn stop(&mut self) {
        self.should_stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("Failed to join listener thread");
        }
    }

    pub fn handle_monitor_event(&mut self, monitor_socket: &zmq::Socket) {
        let mut event_msg = zmq::Message::new();
        monitor_socket.recv(&mut event_msg, 0).expect("Failed to receive event message");

        let mut addr_msg = zmq::Message::new();
        monitor_socket.recv(&mut addr_msg, 0).expect("Failed to receive address message");

        let data = event_msg.as_ref();
        if data.len() >= 6 {
            let event_number = u16::from_be_bytes([data[0], data[1]]);
            let event_value = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
            let endpoint = addr_msg.as_str().unwrap_or("");

            match zmq::SocketEvent::from_raw(event_number) {
                zmq::SocketEvent::CONNECTED => {
                    println!("Socket connected to {}", endpoint);
                    // Connection is successful
                }
                zmq::SocketEvent::DISCONNECTED => {
                    println!("Socket disconnected from {}", endpoint);
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
