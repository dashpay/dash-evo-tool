# Ed25519 Point Conversion Guide for Dash Evo Tool Integration

## Overview

GroveSTARK requires Ed25519 points in extended Edwards coordinates (X:Y:Z:T) for its STARK proof system. This guide shows how to convert standard compressed Ed25519 points (32 bytes) to the extended format required by GroveSTARK.

## Production API

GroveSTARK provides production-grade conversion functions exposed at the top level of the crate:

```rust
use grovestark::{
    compressed_to_extended,
    populate_witness_with_extended,
    create_witness_with_conversion,
    DecompressError,
};
```

## Key Functions

### 1. Direct Conversion: `compressed_to_extended`

Converts a compressed Ed25519 point (32 bytes) to extended coordinates (4 × 32 bytes).

```rust
pub fn compressed_to_extended(
    compressed: &[u8; 32],
) -> Result<([u8; 32], [u8; 32], [u8; 32], [u8; 32]), DecompressError>
```

**Usage:**
```rust
// Your compressed Ed25519 public key or signature R point
let compressed_point = [/* 32 bytes */];

// Convert to extended coordinates
let (x, y, z, t) = compressed_to_extended(&compressed_point)?;

// Now you have:
// - x: X coordinate (32 bytes)
// - y: Y coordinate (32 bytes)  
// - z: Z coordinate (32 bytes, typically 1 for affine points)
// - t: T coordinate (32 bytes, equals X*Y)
```

### 2. Witness Population: `populate_witness_with_extended`

Automatically converts compressed signature R and public key A to extended coordinates and populates a `PrivateInputs` witness.

```rust
pub fn populate_witness_with_extended(
    witness: &mut PrivateInputs,
    signature_r_compressed: &[u8; 32],
    public_key_compressed: &[u8; 32],
) -> Result<(), DecompressError>
```

**Usage:**
```rust
let mut witness = PrivateInputs::default();

// From your Ed25519 signature (R, s)
let sig_r_compressed = [/* 32 bytes R from signature */];

// Your Ed25519 public key
let pubkey_compressed = [/* 32 bytes compressed public key */];

// Populate witness with automatic conversion
populate_witness_with_extended(&mut witness, &sig_r_compressed, &pubkey_compressed)?;

// witness now contains both compressed and extended forms
```

### 3. Complete Witness Builder: `create_witness_with_conversion`

Creates a complete `PrivateInputs` with automatic Ed25519 point conversion.

```rust
pub fn create_witness_with_conversion(
    document_cbor: Vec<u8>,
    owner_id: Vec<u8>,
    signature_r_compressed: &[u8; 32],
    signature_s: &[u8; 32],
    public_key_compressed: &[u8; 32],
    hash_h: &[u8; 32],
    private_key: &[u8; 32],
    s_windows: Vec<u8>,
    h_windows: Vec<u8>,
) -> Result<PrivateInputs, DecompressError>
```

## Integration Example

Here's how DET should integrate with GroveSTARK:

```rust
use grovestark::{GroveSTARK, PublicInputs, populate_witness_with_extended, PrivateInputs};
use ed25519_dalek::{Signer, SigningKey, Signature};

fn generate_grovestark_proof(
    document: Vec<u8>,
    signing_key: &SigningKey,
    state_root: [u8; 32],
    contract_id: [u8; 32],
) -> Result<grovestark::STARKProof, grovestark::Error> {
    // 1. Create Ed25519 signature
    let message = b"challenge_message";
    let signature: Signature = signing_key.sign(message);
    
    // 2. Extract compressed points
    let sig_bytes = signature.to_bytes();
    let sig_r_compressed: [u8; 32] = sig_bytes[0..32].try_into().unwrap();
    let sig_s: [u8; 32] = sig_bytes[32..64].try_into().unwrap();
    
    let pubkey_compressed = signing_key.verifying_key().to_bytes();
    
    // 3. Create witness with automatic conversion
    let mut witness = PrivateInputs::default();
    witness.document_cbor = document;
    witness.owner_id = vec![/* owner ID */];
    witness.signature_s = sig_s;
    witness.hash_h = [/* SHA-512(R || A || M) mod L */];
    witness.private_key = signing_key.to_bytes();
    
    // Populate extended coordinates automatically
    populate_witness_with_extended(
        &mut witness,
        &sig_r_compressed,
        &pubkey_compressed
    )?;
    
    // 4. Add Merkle paths
    witness.document_merkle_path = vec![/* ... */];
    witness.key_merkle_path = vec![/* ... */];
    
    // 5. Add window decompositions
    witness.s_windows = decompose_scalar_to_windows(&sig_s);
    witness.h_windows = decompose_scalar_to_windows(&witness.hash_h);
    
    // 6. Generate proof
    let public_inputs = PublicInputs {
        state_root,
        contract_id,
        message_hash: [/* hash of message */],
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };
    
    let prover = GroveSTARK::default();
    prover.prove(witness, public_inputs)
}

fn decompose_scalar_to_windows(scalar: &[u8; 32]) -> Vec<u8> {
    // Decompose scalar into 64 4-bit windows
    let mut windows = Vec::with_capacity(64);
    for byte in scalar.iter() {
        windows.push(byte & 0x0F);        // Low nibble
        windows.push((byte >> 4) & 0x0F); // High nibble
    }
    windows
}
```

## Extended Coordinates Format

The extended Edwards coordinates represent a point as (X:Y:Z:T) where:
- **X, Y**: The actual curve point coordinates
- **Z**: The projective scaling factor (usually 1 for affine points)
- **T**: The auxiliary coordinate, equals X*Y

For a compressed point `[y_bytes || sign_bit]`:
1. Y coordinate is extracted from the first 255 bits
2. Sign bit determines the sign of X
3. X is computed from the curve equation: x² = (y² - 1) / (d*y² + 1)
4. Z is set to 1 (affine representation)
5. T is computed as X*Y

## Error Handling

The conversion functions return `DecompressError` which can be:
- `NonCanonicalY`: The Y coordinate is ≥ p (field prime)
- `NoSquareRoot`: No valid X coordinate exists for the given Y

Always handle these errors appropriately:

```rust
match compressed_to_extended(&compressed_point) {
    Ok((x, y, z, t)) => {
        // Use extended coordinates
    }
    Err(DecompressError::NonCanonicalY) => {
        // Handle invalid Y coordinate
    }
    Err(DecompressError::NoSquareRoot) => {
        // Handle point not on curve
    }
}
```

## Performance Notes

- Conversion involves modular arithmetic including square root computation
- The implementation uses constant-time operations where possible
- Conversion is performed once per witness creation, not per proof iteration
- Extended coordinates are cached in the witness for reuse

## Testing

Run the example to verify conversion:

```bash
cargo run --example ed25519_conversion
```

This demonstrates all three conversion methods with test vectors.

## Important Security Considerations

1. **Never use placeholder values in production** - The examples use test values like `[0x01; 32]`
2. **Validate all input points** - Always check that compressed points are valid Ed25519 points
3. **Use proper Ed25519 signatures** - Generate signatures using a secure Ed25519 implementation
4. **Protect private keys** - Never expose private keys in logs or error messages

## Questions or Issues?

If you encounter any issues with the conversion functions, please report them to the GroveSTARK repository with:
1. The compressed point that failed to convert
2. The specific error returned
3. Your Ed25519 library and version