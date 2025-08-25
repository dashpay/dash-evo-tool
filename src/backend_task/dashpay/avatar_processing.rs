use sha2::{Digest, Sha256};

/// Calculate SHA-256 hash of image bytes
pub fn calculate_avatar_hash(image_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(image_bytes);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Calculate DHash (Difference Hash) perceptual fingerprint of an image
///
/// This is a simplified implementation that works with raw pixel data.
/// In production, you'd use an image processing library to properly decode
/// and resize the image.
///
/// The DHash algorithm:
/// 1. Convert image to grayscale
/// 2. Resize to 9x8 pixels
/// 3. Compare each pixel with its right neighbor
/// 4. Generate 64-bit hash based on comparisons
pub fn calculate_dhash_fingerprint(image_bytes: &[u8]) -> Result<[u8; 8], String> {
    // This is a placeholder implementation
    // In production, you would:
    // 1. Use an image library to decode the image (JPEG, PNG, etc.)
    // 2. Convert to grayscale
    // 3. Resize to 9x8 pixels
    // 4. Calculate the difference hash

    // For now, we'll create a simple hash based on the image bytes
    // This maintains the correct format but doesn't implement the actual DHash algorithm

    let mut hasher = Sha256::new();
    hasher.update(image_bytes);
    hasher.update(b"dhash_placeholder");
    let result = hasher.finalize();

    let mut fingerprint = [0u8; 8];
    fingerprint.copy_from_slice(&result[..8]);

    Ok(fingerprint)
}

/// Simplified DHash implementation for demonstration
/// This would need a proper image processing library in production
pub struct DHashCalculator {
    width: usize,
    height: usize,
}

impl Default for DHashCalculator {
    fn default() -> Self {
        Self {
            width: 9,
            height: 8,
        }
    }
}

impl DHashCalculator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert RGB pixels to grayscale
    #[allow(dead_code)]
    fn to_grayscale(&self, rgb: &[u8]) -> Vec<u8> {
        let mut grayscale = Vec::new();
        for chunk in rgb.chunks(3) {
            if chunk.len() == 3 {
                // Standard grayscale conversion: 0.299*R + 0.587*G + 0.114*B
                let gray = (0.299 * chunk[0] as f32
                    + 0.587 * chunk[1] as f32
                    + 0.114 * chunk[2] as f32) as u8;
                grayscale.push(gray);
            }
        }
        grayscale
    }

    /// Simple box filter resize (nearest neighbor)
    fn resize(&self, pixels: &[u8], orig_width: usize, orig_height: usize) -> Vec<u8> {
        let mut resized = Vec::with_capacity(self.width * self.height);

        for y in 0..self.height {
            for x in 0..self.width {
                let orig_x = (x * orig_width) / self.width;
                let orig_y = (y * orig_height) / self.height;
                let idx = orig_y * orig_width + orig_x;

                if idx < pixels.len() {
                    resized.push(pixels[idx]);
                } else {
                    resized.push(0);
                }
            }
        }

        resized
    }

    /// Calculate the DHash from grayscale pixels
    pub fn calculate(&self, grayscale_pixels: &[u8], width: usize, height: usize) -> [u8; 8] {
        // Resize to 9x8
        let resized = self.resize(grayscale_pixels, width, height);

        // Calculate differences and build hash
        let mut hash = 0u64;
        let mut bit_position = 0;

        for y in 0..self.height {
            for x in 0..self.width - 1 {
                let idx = y * self.width + x;
                if idx + 1 < resized.len() {
                    // Set bit to 1 if left pixel is brighter than right
                    if resized[idx] > resized[idx + 1] {
                        hash |= 1 << bit_position;
                    }
                    bit_position += 1;
                }
            }
        }

        hash.to_le_bytes()
    }
}

/// Calculate Hamming distance between two perceptual hashes
/// Used to determine similarity between images
pub fn hamming_distance(hash1: &[u8; 8], hash2: &[u8; 8]) -> u32 {
    let mut distance = 0u32;

    for i in 0..8 {
        let xor = hash1[i] ^ hash2[i];
        distance += xor.count_ones();
    }

    distance
}

/// Check if two images are similar based on their perceptual hashes
/// Returns true if Hamming distance is below threshold (typically 10-15)
pub fn are_images_similar(hash1: &[u8; 8], hash2: &[u8; 8], threshold: u32) -> bool {
    hamming_distance(hash1, hash2) <= threshold
}

/// Fetch image from URL and return bytes
/// This is a placeholder - in production you'd use reqwest or similar
pub async fn fetch_image_bytes(url: &str) -> Result<Vec<u8>, String> {
    // Check URL is valid and uses HTTPS
    if !url.starts_with("https://") {
        return Err("Avatar URL must use HTTPS".to_string());
    }

    // Validate URL length per DIP-0015 (max 2048 characters)
    if url.len() > 2048 {
        return Err("Avatar URL exceeds maximum length of 2048 characters".to_string());
    }

    // In production, you would:
    // 1. Use reqwest or similar to fetch the image
    // 2. Validate content-type is an image
    // 3. Limit download size (e.g., max 5MB)
    // 4. Return the raw bytes

    // Placeholder for now
    Err("Image fetching not yet implemented - requires HTTP client".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_avatar_hash() {
        let test_data = b"test image data";
        let hash = calculate_avatar_hash(test_data);
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_dhash_fingerprint() {
        let test_data = b"test image data";
        let fingerprint = calculate_dhash_fingerprint(test_data).unwrap();
        assert_eq!(fingerprint.len(), 8);
    }

    #[test]
    fn test_hamming_distance() {
        let hash1 = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let hash2 = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(hamming_distance(&hash1, &hash2), 64);

        let hash3 = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(hamming_distance(&hash1, &hash3), 0);
    }

    #[test]
    fn test_image_similarity() {
        let hash1 = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let hash2 = [0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]; // 1 bit different

        assert!(are_images_similar(&hash1, &hash2, 10));
        assert!(!are_images_similar(&hash1, &hash2, 0));
    }
}
