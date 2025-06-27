use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;

fn main() {
    // Test parsing DAPI addresses and converting to P2P addresses
    let dapi_addresses_str = "https://104.200.24.196:443,https://134.255.182.185:443,https://134.255.182.186:443";
    let network = Network::Dash;
    
    println!("Testing DAPI to P2P address conversion:");
    println!("DAPI addresses: {}", dapi_addresses_str);
    println!("");
    
    let mut p2p_addresses = vec![];
    
    for addr_str in dapi_addresses_str.split(',') {
        let addr_str = addr_str.trim();
        if addr_str.is_empty() {
            continue;
        }
        
        // Parse as URI to extract host
        if let Ok(uri) = addr_str.parse::<dash_sdk::sdk::Uri>() {
            if let Some(host) = uri.host() {
                // Convert DAPI port to P2P port
                let p2p_port = match network {
                    Network::Dash => 9999,
                    Network::Testnet => 19999,
                    _ => continue,
                };
                
                // Try to parse the host as an IP address
                if let Ok(ip_addr) = host.parse::<std::net::IpAddr>() {
                    let p2p_addr = std::net::SocketAddr::new(ip_addr, p2p_port);
                    p2p_addresses.push(p2p_addr);
                    println!("✓ Converted {} -> {}", addr_str, p2p_addr);
                } else {
                    println!("✗ Skipping non-IP host: {} from {}", host, addr_str);
                }
            }
        } else {
            println!("✗ Failed to parse DAPI address as URI: {}", addr_str);
        }
    }
    
    println!("\nTotal P2P addresses: {}", p2p_addresses.len());
    
    // Test peer group selection
    println!("\nTesting peer group selection:");
    let peers_per_group = 3;
    for group_index in 0..3 {
        let start_index = (group_index * peers_per_group) % p2p_addresses.len();
        let mut selected_peers = vec![];
        for i in 0..peers_per_group {
            let index = (start_index + i) % p2p_addresses.len();
            selected_peers.push(p2p_addresses[index]);
        }
        println!("Group {}: {:?}", group_index, selected_peers);
    }
}