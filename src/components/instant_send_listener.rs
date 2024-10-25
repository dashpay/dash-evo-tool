use dash_sdk::dpp::dashcore::consensus::Decodable;
use dash_sdk::dpp::dashcore::{InstantLock, Network, Transaction};
use image::EncodableLayout;
use std::error::Error;
use std::io::Cursor;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;
use zmq::Context;

pub struct InstantSendListener {
    should_stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl InstantSendListener {
    pub fn spawn_listener(
        network: Network,
        endpoint: &str,
        sender: mpsc::Sender<(Transaction, InstantLock, Network)>,
    ) -> Result<Self, Box<dyn Error>> {
        let should_stop = Arc::new(AtomicBool::new(false));
        let endpoint = endpoint.to_string();
        let should_stop_clone = Arc::clone(&should_stop);
        let sender_clone = sender.clone();

        let handle = thread::spawn(move || {
            // Create the socket inside the thread.
            let context = Context::new();
            let socket = context.socket(zmq::SUB).expect("Failed to create socket");

            // Connect to the zmqpubhashtxlock endpoint.
            socket.connect(&endpoint).expect("Failed to connect");

            // Subscribe to the "rawtxlocksig" events.
            socket
                .set_subscribe(b"rawtxlocksig")
                .expect("Failed to subscribe to rawtxlocksig");

            println!("Connected to ZMQ at {}", endpoint);

            while !should_stop_clone.load(Ordering::SeqCst) {
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
                                                    if let Err(e) =
                                                        sender_clone.send((tx, islock, network))
                                                    {
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
                        } else {
                            println!("Received message without data for topic: {}", topic);
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

            println!("Listener is stopping.");
            // Clean up socket (optional, as it will be dropped here).
            drop(socket);
        });

        Ok(InstantSendListener {
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
