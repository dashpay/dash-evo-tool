use egui::{Button, Color32, Grid, Ui, Vec2};
use rand::Rng;

pub struct U256EntropyGrid {
    random_number: [u8; 32], // Current 256-bit number (32 bytes)
    last_bit_changed: u8,    // Store the last bit position changed
}

impl U256EntropyGrid {
    /// Create a new instance with a random [u8; 32] number
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        let mut random_number = [0u8; 32];
        rng.fill(&mut random_number); // Fill with random bytes

        Self {
            random_number,
            last_bit_changed: 0, // Initialize to 0
        }
    }

    /// Render the UI and allow users to modify bits
    pub fn ui(&mut self, ui: &mut Ui) -> [u8; 32] {
        ui.heading("Select Bits for 256-bit Number");

        // Get the available width to calculate dynamic button sizes
        let available_width = ui.available_width();
        let button_width = available_width / 32.0; // Divide the total width into 32 equal parts
        let button_size = Vec2::new(button_width, button_width); // Square buttons

        // Create a grid with 8 rows and 32 columns (256 bits total)
        Grid::new("entropy_grid")
            .num_columns(32) // Explicitly set 32 columns
            .spacing(Vec2::new(0.0, 0.0)) // Remove spacing between buttons
            .show(ui, |ui| {
                for row in 0..8 {
                    for col in 0..32 {
                        let bit_position = (row * 32 + col) as u8;
                        let byte_index = (bit_position / 8) as usize;
                        let bit_in_byte = (bit_position % 8) as usize;

                        // Determine the bit value in the current number
                        let bit_value = (self.random_number[byte_index] >> bit_in_byte) & 1 == 1;

                        // Set the button color based on the bit value (1 = Black, 0 = White)
                        let color = if bit_value {
                            Color32::BLACK
                        } else {
                            Color32::WHITE
                        };

                        // Create the button with dynamic size and color
                        let button = Button::new("").fill(color).min_size(button_size);

                        // Render the button and handle interactions
                        let response = ui.add(button);

                        // Toggle the bit if clicked or hovered
                        if response.hovered() && self.was_bit_different(bit_position)
                            || response.clicked()
                        {
                            self.toggle_bit(byte_index, bit_in_byte);
                        }
                    }
                    ui.end_row();
                }
            });

        // Display the current random number in hex
        ui.label(format!(
            "Current 256-bit Number: {}",
            hex::encode(self.random_number)
        ));

        self.random_number
    }

    /// Check if the bit at the given position is the same as the last changed bit
    fn was_bit_different(&self, bit_position: u8) -> bool {
        self.last_bit_changed != bit_position
    }

    /// Toggle the bit at the given byte and bit position
    fn toggle_bit(&mut self, byte_index: usize, bit_in_byte: usize) {
        // Toggle the bit using XOR
        self.random_number[byte_index] ^= 1 << bit_in_byte;

        // Update the last changed bit position
        self.last_bit_changed = (byte_index * 8 + bit_in_byte) as u8;
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
