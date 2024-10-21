use dash_sdk::dashcore_rpc::dashcore::Txid;
use std::error::Error;
use std::time::Duration;
use tokio::time::sleep;
use zmq::{Context, Message, Socket};

pub struct InstantSendListener {
    socket: Socket,
}

impl InstantSendListener {
    /// Create a new InstantSend listener connected to the given ZMQ endpoint.
    pub fn new(endpoint: &str) -> Result<Self, Box<dyn Error>> {
        let context = Context::new();
        let socket = context.socket(zmq::SUB)?;

        // Connect to the zmqpubhashtxlock endpoint.
        socket.connect(endpoint)?;
        // Subscribe to the "hashtxlock" events.
        socket.set_subscribe(b"hashtxlock")?;

        println!("Connected to ZMQ at {}", endpoint);
        Ok(Self { socket })
    }

    /// Start listening for InstantSend locks.
    pub async fn listen_for_locks(&self, expected_txid: Txid) -> Result<(), Box<dyn Error>> {
        println!("Listening for InstantSend locks...");

        loop {
            // Wait to receive a message from the ZMQ socket.
            let mut message = Message::new();
            self.socket.recv(&mut message, 0)?;

            // Convert the message into a string and check if it matches the expected TXID.
            let received_txid = message.to_str()?.trim();

            if let Ok(txid) = Txid::from_hex(received_txid) {
                if txid == expected_txid {
                    println!("Transaction {} is InstantSend locked!", txid);
                    break;
                }
            }

            // Throttle the loop to avoid busy waiting.
            sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }
}
