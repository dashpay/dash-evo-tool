use egui::{Color32, Grid, Ui, Vec2};
use rand::Rng;

pub struct U256EntropyGrid {
    random_number: [u8; 32],   // Current 256-bit number (32 bytes)
    previous_number: [u8; 32], // Previous frame's state of the 256-bit number
}

impl U256EntropyGrid {
    /// Create a new instance with a random [u8; 32] number
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        let mut random_number = [0u8; 32];
        rng.fill(&mut random_number); // Fill with random bytes

        Self {
            random_number,
            previous_number: random_number, // Initialize previous_number to the same value
        }
    }

    /// Render the UI and allow users to modify bits
    pub fn ui(&mut self, ui: &mut Ui) -> [u8; 32] {
        ui.heading("Select Bits for 256-bit Number");

        // Create a grid with 8 rows and 32 columns (256 bits total)
        Grid::new("entropy_grid").show(ui, |ui| {
            for row in 0..8 {
                for col in 0..32 {
                    let bit_position = row * 32 + col;
                    let byte_index = bit_position / 8;
                    let bit_in_byte = bit_position % 8;

                    // Determine the bit value in the current number
                    let bit_value = (self.random_number[byte_index] >> bit_in_byte) & 1 == 1;

                    // Set the button color based on the bit value (1 = Black, 0 = White)
                    let color = if bit_value {
                        Color32::BLACK
                    } else {
                        Color32::WHITE
                    };

                    // Define the button size and allocate the rect
                    let button_size = Vec2::new(8.0, 8.0);
                    let button_rect = ui.allocate_space(button_size).1;

                    // Interact with the button to detect hover or click
                    let response = ui.interact(
                        button_rect,
                        ui.id().with(bit_position),
                        egui::Sense::hover(),
                    );

                    // If the bit was different in the previous state, toggle it
                    let was_different = self.was_bit_different(byte_index, bit_in_byte);
                    if response.hovered() && was_different {
                        self.toggle_bit(byte_index, bit_in_byte);
                    }

                    // Render the button with the appropriate color
                    ui.painter().rect_filled(button_rect, 0.0, color);
                }
                ui.end_row();
            }
        });

        // Display the current and previous random numbers in hex
        ui.label(format!(
            "Current 256-bit Number: {}",
            hex::encode(self.random_number)
        ));
        ui.label(format!(
            "Previous 256-bit Number: {}",
            hex::encode(self.previous_number)
        ));

        // Update the previous_number for the next frame
        self.previous_number = self.random_number;

        self.random_number
    }

    /// Check if a bit differs between the previous and current numbers
    fn was_bit_different(&self, byte_index: usize, bit_in_byte: usize) -> bool {
        let current_bit = (self.random_number[byte_index] >> bit_in_byte) & 1;
        let previous_bit = (self.previous_number[byte_index] >> bit_in_byte) & 1;
        current_bit != previous_bit
    }

    /// Toggle the bit at the given byte and bit position
    fn toggle_bit(&mut self, byte_index: usize, bit_in_byte: usize) {
        self.previous_number = self.random_number;
        self.random_number[byte_index] ^= 1 << bit_in_byte;
    }

    /// Generate a new random number and XOR it with the current number
    pub fn random_number_with_user_input(&self) -> [u8; 32] {
        let mut rng = rand::thread_rng();
        let mut new_random_number = [0u8; 32];
        rng.fill(&mut new_random_number); // Generate a new random number

        // XOR the new random number with the existing one
        let mut result = [0u8; 32];
        for i in 0..32 {
            result[i] = self.random_number[i] ^ new_random_number[i];
        }
        result
    }
}
