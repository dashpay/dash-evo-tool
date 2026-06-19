//! Shared no-leak assertion for secret-path tests.
//!
//! Promoted from the private `assert_no_leak` in
//! `model/qualified_identity/encrypted_key_storage.rs::tests` so the seam,
//! sidecar, QI-blob, and `ClosedSingleKey`-Debug leak cases share one
//! implementation rather than copy-pasting it.
//!
//! The decimal-array check is load-bearing: a `#[derive(Debug)]` on `[u8; N]`
//! leaks the `[160, 167, …]` decimal form, and finding `6a2818cd` leaked
//! exactly that. Hex alone would falsely pass against that bug.

/// Assert `rendered` exposes `secret` in NONE of the forms a sink could leak
/// it: lowercase hex and the `[160, 167, …]` decimal-array form. Works for any
/// secret length (32-byte keys, 64-byte seeds).
pub(crate) fn assert_no_leak_bytes(rendered: &str, secret: &[u8], context: &str) {
    let hex = hex::encode(secret);
    let decimal_array = format!(
        "[{}]",
        secret
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(
        !rendered.contains(&hex),
        "{context} leaked the raw secret (hex): {rendered}"
    );
    assert!(
        !rendered.contains(&decimal_array),
        "{context} leaked the raw secret (byte array): {rendered}"
    );
}

/// A recognizable 32-byte secret. A full 32-byte collision with unrelated
/// bytes is astronomically improbable, so finding it anywhere in a rendering
/// means the raw key bytes leaked.
pub(crate) fn distinctive_secret_32() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = 0xA0 ^ (i as u8).wrapping_mul(7);
    }
    bytes
}

/// A recognizable 64-byte secret, the seed-length analogue of
/// [`distinctive_secret_32`].
pub(crate) fn distinctive_secret_64() -> [u8; 64] {
    let mut bytes = [0u8; 64];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = 0xC0 ^ (i as u8).wrapping_mul(5);
    }
    bytes
}
