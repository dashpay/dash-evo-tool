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
use futures::StreamExt;
use tokio::runtime::Runtime;
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

pub const IS_LOCK_SIG_MSG: &str = "rawtxlocksig";
pub const CHAIN_LOCKED_BLOCK_MSG: &str = "rawchainlock";

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
            // Create the runtime inside the thread.
            let rt = Runtime::new().unwrap();
            rt.block_on(async move {
                // Create the socket inside the async context.
                let mut socket = SubSocket::new();

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
                    // Receive messages
                    match socket.recv().await {
                        Ok(msg) => {
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
                        }
                        Err(e) => {
                            eprintln!("Error receiving message: {}", e);
                            // Sleep briefly before retrying
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
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