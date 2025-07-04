use std::ops::Deref;

pub use aes_gcm::aead::heapless::Vec as HeaplessVec;
use dash_sdk::dpp::bls_signatures::vsss_rs::elliptic_curve::bigint::Zero;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};
/// Secret (eg. a password) used in KMS operations.
///
/// Maximum size is 127 bytes, which is the maximum size of a `heapless::Vec<u8>`.
/// The memory is locked to prevent it from being swapped out, ensuring that sensitive data
/// remains in RAM and is not written to disk.
///The [`Zeroizing` type is used to ensure that the data is zeroized when it goes out of scope,
/// preventing sensitive data from lingering in memory after use.
pub struct Secret {
    data: HeaplessVec<u8, 4096>,
    guard: region::LockGuard,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretError {
    #[error("Provided secret is too large, maximum size is 4096 bytes")]
    TooLarge,
}
impl Secret {
    /// Creates a new `Secret` from a vector of bytes.
    pub fn new<T: AsRef<[u8]> + Zeroize>(mut from: T) -> Result<Self, SecretError> {
        let data = HeaplessVec::from_slice(from.as_ref()).map_err(|_| SecretError::TooLarge)?;
        from.zeroize();
        // Lock the memory to prevent it from being swapped out
        let guard =
            region::lock(data.as_ptr(), data.capacity()).expect("Failed to lock memory for Secret");

        Ok(Self { data, guard })
    }
}

impl AsMut<HeaplessVec<u8, 4096>> for Secret {
    /// Returns a mutable reference to the underlying data as a `heapless::Vec<u8, 4096>`.
    ///
    /// This is useful when using aes_gcm with `heapless` feature enabled.
    fn as_mut(&mut self) -> &mut HeaplessVec<u8, 4096> {
        // Return a mutable reference to the underlying data
        &mut self.data
    }
}

impl AsRef<[u8]> for Secret {
    fn as_ref(&self) -> &[u8] {
        // Return a reference to the underlying data
        &self.data
    }
}

impl AsRef<[u8; 32]> for Secret {
    /// Returns a reference to the underlying data as a fixed-size array of 32 bytes.
    /// This is useful for cases where the secret is expected to be exactly 32 bytes long,
    /// such as when dealing with cryptographic keys.
    ///
    /// # Panics
    ///
    /// Panics if the data is not exactly 32 bytes long.
    fn as_ref(&self) -> &[u8; 32] {
        // Ensure the data is exactly 32 bytes long
        if self.data.len() != 32 {
            panic!("Secret data must be exactly 32 bytes long");
        }
        // Convert the heapless Vec to a slice of 32 bytes
        unsafe { &*(self.data.as_ptr() as *const [u8; 32]) }
    }
}

impl Deref for Secret {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        // Deref to the underlying data
        &self.data
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        // Create a new Secret with the same data
        let cloned_data = self.data.clone();
        let guard = region::lock(cloned_data.as_ptr(), cloned_data.len())
            .expect("Failed to lock memory for cloned Secret");

        Self {
            data: cloned_data,
            guard,
        }
    }
}

impl Zeroize for Secret {
    fn zeroize(&mut self) {
        // Zeroize the data to prevent sensitive data from lingering in memory
        self.data.zeroize();
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Zeroize the data to prevent sensitive data from lingering in memory
        self.zeroize();
    }
}
