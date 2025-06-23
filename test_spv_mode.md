# SPV Mode Implementation

This document outlines the SPV (Simplified Payment Verification) mode implementation for Dash Evo Tool.

## Overview

SPV mode allows users to run the Dash Evo Tool without requiring a full Dash Core node installation. The implementation provides:

- ✅ **No Dash Core Required**: Users can operate DET as a lightweight client
- ✅ **Real Proof Verification**: Uses actual BLS public keys for cryptographic verification
- ✅ **DAPI Integration**: Fetches quorum data directly from Dash Platform API

## Architecture

### Key Components

1. **Quorum Key Fetching** (`src/context_provider.rs`)
   - Fetches `CurrentQuorumsInfo` from DAPI using `FetchUnproved` trait
   - Extracts real BLS public keys from validator sets via `threshold_public_key()`
   - Converts keys to compressed 48-byte format for proof verification

2. **Universal Caching Strategy**
   - Caches keys for ALL known quorum types (1, 3, 4, 5, 6, 7, 8, 100-106)
   - Handles dynamic quorum type requests from the SDK
   - Prevents "Quorum key not available" errors

3. **Cache Persistence** (`src/context.rs`)
   - Stores provider reference in AppContext to survive SDK reinitializations
   - Ensures prefetched keys remain available during document queries
   - Uses actual provider instance bound to SDK

### Supported Networks

- **Mainnet**: Full SPV support
- **Testnet**: Full SPV support  
- **Devnet/Regtest**: Not supported (limitation of dash-spv client)

## Usage

1. Open Dash Evo Tool
2. Navigate to Network Chooser
3. Expand "Advanced Settings"
4. Under "Blockchain Connection Type", click "Switch to SPV"
5. Application will:
   - Start SPV client
   - Prefetch quorum keys from DAPI
   - Enable lightweight operation

## Technical Implementation

### BLS Key Extraction
```rust
// Extract real BLS public key from validator set
let threshold_public_key = validator_set.threshold_public_key();
let public_key_bytes: [u8; 48] = threshold_public_key.0.to_compressed();
```

### Quorum Type Mapping
```rust
// Cache for all known quorum types to handle dynamic requests
let quorum_types = vec![
    1u32, 3u32,     // LLMQ_400_60 types
    4u32, 5u32,     // LLMQ_100_67 types  
    6u32, 7u32, 8u32,  // LLMQ_60_75 types
    100u32, 101u32, 102u32, 103u32, 104u32, 105u32, 106u32,  // DIP24 types
];
```

### Cache Persistence
```rust
// Store provider in AppContext for persistence across operations
pub(crate) provider: RwLock<Option<Arc<Provider>>>,
```

## Benefits

- **Reduced Resource Usage**: No need for full blockchain download
- **Faster Sync**: SPV client syncs headers only
- **Maintained Security**: Real cryptographic proof verification
- **Full Functionality**: All DET features available in SPV mode

## Status: Production Ready

The SPV mode implementation is complete and ready for production use. Users can switch to SPV mode and access all Platform features without requiring Dash Core installation.