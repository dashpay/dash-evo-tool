use image::{DynamicImage, GenericImageView};
use sha2::{Digest, Sha256};

/// Maximum allowed size for avatar images (5MB)
const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;

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
/// The DHash algorithm:
/// 1. Convert image to grayscale
/// 2. Resize to 9x8 pixels
/// 3. Compare each pixel with its right neighbor
/// 4. Generate 64-bit hash based on comparisons
pub fn calculate_dhash_fingerprint(image_bytes: &[u8]) -> Result<[u8; 8], String> {
    // Load the image from bytes
    let img =
        image::load_from_memory(image_bytes).map_err(|e| format!("Failed to load image: {}", e))?;

    // Convert to grayscale and resize to 9x8
    let grayscale = img.grayscale();
    let resized = grayscale.resize_exact(9, 8, image::imageops::FilterType::Lanczos3);

    // Calculate the difference hash
    let mut hash = 0u64;
    let mut bit_position = 0;

    for y in 0..8 {
        for x in 0..8 {
            // Get the luminance values of adjacent pixels
            let left_pixel = resized.get_pixel(x, y).0[0];
            let right_pixel = resized.get_pixel(x + 1, y).0[0];

            // Set bit to 1 if left pixel is brighter than right
            if left_pixel > right_pixel {
                hash |= 1 << bit_position;
            }
            bit_position += 1;
        }
    }

    Ok(hash.to_le_bytes())
}

/// DHash calculator for more advanced image processing
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

    /// Calculate DHash from a DynamicImage
    pub fn calculate_from_image(&self, img: &DynamicImage) -> [u8; 8] {
        // Convert to grayscale and resize
        let grayscale = img.grayscale();
        let resized = grayscale.resize_exact(
            self.width as u32,
            self.height as u32,
            image::imageops::FilterType::Lanczos3,
        );

        // Calculate differences and build hash
        let mut hash = 0u64;
        let mut bit_position = 0;

        for y in 0..self.height {
            for x in 0..(self.width - 1) {
                let left_pixel = resized.get_pixel(x as u32, y as u32).0[0];
                let right_pixel = resized.get_pixel((x + 1) as u32, y as u32).0[0];

                if left_pixel > right_pixel {
                    hash |= 1 << bit_position;
                }
                bit_position += 1;
            }
        }

        hash.to_le_bytes()
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
pub async fn fetch_image_bytes(url: &str) -> Result<Vec<u8>, String> {
    // Check URL is valid and uses HTTPS
    if !url.starts_with("https://") {
        return Err("Avatar URL must use HTTPS".to_string());
    }

    // Validate URL length per DIP-0015 (max 2048 characters)
    if url.len() > 2048 {
        return Err("Avatar URL exceeds maximum length of 2048 characters".to_string());
    }

    // Create HTTP client with timeout
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Send GET request
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch image: {}", e))?;

    // Check status code
    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    // Check content type
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type_str = content_type
            .to_str()
            .map_err(|e| format!("Invalid content-type header: {}", e))?;

        if !content_type_str.starts_with("image/") {
            return Err(format!(
                "Invalid content type: expected image/*, got {}",
                content_type_str
            ));
        }
    }

    // Check content length if provided
    if let Some(content_length) = response.headers().get("content-length") {
        let length_str = content_length
            .to_str()
            .map_err(|e| format!("Invalid content-length header: {}", e))?;

        let length: usize = length_str
            .parse()
            .map_err(|e| format!("Failed to parse content-length: {}", e))?;

        if length > MAX_IMAGE_SIZE {
            return Err(format!(
                "Image too large: {} bytes (max {} bytes)",
                length, MAX_IMAGE_SIZE
            ));
        }
    }

    // Download the image bytes
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to download image: {}", e))?;

    // Verify actual size
    if bytes.len() > MAX_IMAGE_SIZE {
        return Err(format!(
            "Image too large: {} bytes (max {} bytes)",
            bytes.len(),
            MAX_IMAGE_SIZE
        ));
    }

    // Try to validate it's actually an image by attempting to load it
    image::load_from_memory(&bytes).map_err(|e| format!("Invalid image data: {}", e))?;

    Ok(bytes.to_vec())
}

/// Process an avatar image: fetch, validate, and calculate hashes
pub async fn process_avatar(url: &str) -> Result<(Vec<u8>, [u8; 32], [u8; 8]), String> {
    // Fetch the image
    let image_bytes = fetch_image_bytes(url).await?;

    // Calculate SHA-256 hash
    let hash = calculate_avatar_hash(&image_bytes);

    // Calculate DHash fingerprint
    let fingerprint = calculate_dhash_fingerprint(&image_bytes)?;

    Ok((image_bytes, hash, fingerprint))
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

    #[test]
    fn test_dhash_with_real_image() {
        // Create a simple test image (3x3 grayscale)
        let pixels = vec![
            0, 50, 100, // Row 1: increasing brightness
            50, 100, 150, // Row 2: increasing brightness
            100, 150, 200, // Row 3: increasing brightness
        ];

        // Create an image from raw pixels
        let img = image::GrayImage::from_raw(3, 3, pixels).unwrap();
        let dynamic_img = DynamicImage::ImageLuma8(img);

        // Calculate DHash
        let calculator = DHashCalculator::new();
        let hash = calculator.calculate_from_image(&dynamic_img);

        // Verify we get an 8-byte hash
        assert_eq!(hash.len(), 8);
    }

    #[tokio::test]
    async fn test_url_validation() {
        // Test non-HTTPS URL
        let result = fetch_image_bytes("http://example.com/image.jpg").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Avatar URL must use HTTPS");

        // Test URL that's too long
        let long_url = format!("https://example.com/{}", "a".repeat(2100));
        let result = fetch_image_bytes(&long_url).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum length"));
    }
}
