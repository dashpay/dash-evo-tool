use egui::{
    Button, Color32, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke,
    Vec2, WidgetText,
};

/// Theme mode enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

/// Detect system theme preference
pub fn detect_system_theme() -> Result<ThemeMode, String> {
    match dark_light::detect().map_err(|e| e.to_string())? {
        dark_light::Mode::Dark => Ok(ThemeMode::Dark),
        dark_light::Mode::Light => Ok(ThemeMode::Light),
        dark_light::Mode::Unspecified => Ok(ThemeMode::Light), // Default to light if unknown
    }
}

/// Resolve the actual theme to use based on preference
pub fn resolve_theme_mode(preference: ThemeMode) -> ThemeMode {
    match preference {
        ThemeMode::System => detect_system_theme().unwrap_or(ThemeMode::Light),
        other => other,
    }
}

/// Dash brand colors according to official guidelines
pub struct DashColors;

#[allow(dead_code)]
impl DashColors {
    /// Primary Dash Blue (#008de4)
    pub const DASH_BLUE: Color32 = Color32::from_rgb(0, 141, 228);

    /// Deep Blue (#012060)
    pub const DEEP_BLUE: Color32 = Color32::from_rgb(1, 32, 96);

    /// Midnight Blue (#0b0f3b)
    pub const MIDNIGHT_BLUE: Color32 = Color32::from_rgb(11, 15, 59);

    /// Black (#111921)
    pub const BLACK: Color32 = Color32::from_rgb(17, 25, 33);

    /// Light Gray - Replaced dark gray with lighter shade
    pub const GRAY: Color32 = Color32::from_rgb(160, 170, 180);

    /// White (#ffffff)
    pub const WHITE: Color32 = Color32::from_rgb(255, 255, 255);

    /// Black Pearl (#001624)
    pub const BLACK_PEARL: Color32 = Color32::from_rgb(0, 22, 36);

    // Semantic colors
    pub const SUCCESS: Color32 = Color32::from_rgb(39, 174, 96);
    pub const WARNING: Color32 = Color32::from_rgb(241, 196, 15);
    pub const ERROR: Color32 = Color32::from_rgb(235, 87, 87);
    pub const INFO: Color32 = Color32::from_rgb(52, 152, 219);
    /// Darker red for danger button hover state
    pub const DANGER_HOVER: Color32 = Color32::from_rgb(200, 0, 0);
    /// Red for danger/destructive action buttons (delete, remove)
    pub const DANGER_RED: Color32 = Color32::from_rgb(200, 60, 60);
    /// Gray fill for disabled/inactive buttons
    pub const BUTTON_DISABLED: Color32 = Color32::from_rgb(100, 100, 100);
    /// Salmon/orange for input validation warnings
    pub const VALIDATION_WARNING: Color32 = Color32::from_rgb(255, 150, 100);
    /// Bright orange for important warnings (e.g., private key exposure, missing identities)
    pub const WARNING_BRIGHT: Color32 = Color32::from_rgb(255, 152, 0);
    /// Purple for Platform address type indicators
    pub const PLATFORM_PURPLE: Color32 = Color32::from_rgb(130, 80, 220);
    /// Blue for primary action buttons (Generate, Save, Import)
    pub const ACTION_BUTTON_BLUE: Color32 = Color32::from_rgb(0, 128, 255);
    /// Gold/amber for text highlighting (e.g., matched hashes in proof logs)
    pub const HIGHLIGHT_GOLD: Color32 = Color32::from_rgb(0x9b, 0x87, 0x0c);
    /// Light pink for very weak password strength
    pub const STRENGTH_WEAK: Color32 = Color32::from_rgb(255, 182, 193);
    /// Light yellow for fair password strength
    pub const STRENGTH_FAIR: Color32 = Color32::from_rgb(255, 224, 130);
    /// Light green for good password strength
    pub const STRENGTH_GOOD: Color32 = Color32::from_rgb(144, 238, 144);
    /// Medium green for strong password strength
    pub const STRENGTH_STRONG: Color32 = Color32::from_rgb(90, 200, 90);

    // Network accent colors
    /// Muted Dash blue for dark mode (20% darker)
    pub const DASH_BLUE_DARK: Color32 = Color32::from_rgb(0, 113, 182);
    /// Testnet orange for light mode
    pub const TESTNET_ORANGE: Color32 = Color32::from_rgb(255, 165, 0);
    /// Muted testnet orange for dark mode
    pub const TESTNET_ORANGE_DARK: Color32 = Color32::from_rgb(204, 132, 0);
    /// Devnet dark red for light mode (matches Color32::DARK_RED)
    pub const DEVNET_RED: Color32 = Color32::from_rgb(139, 0, 0);
    /// Muted devnet red for dark mode
    pub const DEVNET_RED_DARK: Color32 = Color32::from_rgb(111, 0, 0);
    /// Regtest brown for light mode
    pub const REGTEST_BROWN: Color32 = Color32::from_rgb(139, 69, 19);
    /// Muted regtest brown for dark mode
    pub const REGTEST_BROWN_DARK: Color32 = Color32::from_rgb(111, 55, 15);

    // Icon tint colors for nav panels
    /// Icon tint when selected/active
    pub const ICON_SELECTED: Color32 = Color32::WHITE;
    /// Cornflower blue tint for selected wallet panel icons
    pub const ICON_SELECTED_BLUE: Color32 = Color32::from_rgb(100, 149, 237);
    /// Gray tint for unselected icons in dark mode
    pub const ICON_UNSELECTED_DARK: Color32 = Color32::from_rgb(180, 180, 180);
    /// Gray tint for unselected icons in light mode
    pub const ICON_UNSELECTED_LIGHT: Color32 = Color32::from_rgb(160, 160, 160);
    /// Gray tint for unselected wallet panel icons
    pub const ICON_UNSELECTED: Color32 = Color32::from_rgb(169, 169, 169);

    // Entropy grid colors
    /// Off squares in entropy grid (dark mode)
    pub const ENTROPY_OFF_DARK: Color32 = Color32::from_rgb(80, 80, 80);

    // UI Colors - Light mode
    pub const BACKGROUND: Color32 = Color32::from_rgb(240, 242, 247);
    pub const BACKGROUND_DARK: Color32 = Color32::from_rgb(230, 235, 245);
    pub const SURFACE: Color32 = Color32::WHITE;
    pub const INPUT_BACKGROUND: Color32 = Color32::from_rgb(248, 250, 252);
    pub const BORDER: Color32 = Color32::from_rgb(226, 232, 240);
    pub const BORDER_LIGHT: Color32 = Color32::from_rgb(240, 245, 251);
    pub const TEXT_PRIMARY: Color32 = Self::BLACK;
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(100, 120, 140);
    pub const TEXT_ON_PRIMARY: Color32 = Self::WHITE;

    // Dark mode UI colors
    pub const DARK_BACKGROUND: Color32 = Color32::from_rgb(18, 18, 18);
    pub const DARK_BACKGROUND_ELEVATED: Color32 = Color32::from_rgb(28, 28, 28);
    pub const DARK_SURFACE: Color32 = Color32::from_rgb(32, 32, 32);
    pub const DARK_INPUT_BACKGROUND: Color32 = Color32::from_rgb(40, 40, 40);
    pub const DARK_BORDER: Color32 = Color32::from_rgb(60, 60, 60);
    pub const DARK_BORDER_LIGHT: Color32 = Color32::from_rgb(50, 50, 50);
    pub const DARK_TEXT_PRIMARY: Color32 = Color32::from_rgb(240, 240, 240);
    pub const DARK_TEXT_SECONDARY: Color32 = Color32::from_rgb(160, 160, 160);
    pub const DARK_TEXT_ON_PRIMARY: Color32 = Self::WHITE;

    // Gradient colors for modern effects
    pub const GRADIENT_START: Color32 = Color32::from_rgb(0, 141, 228); // Dash Blue
    pub const GRADIENT_END: Color32 = Color32::from_rgb(1, 32, 96); // Deep Blue
    pub const GRADIENT_ACCENT: Color32 = Color32::from_rgb(52, 152, 219); // Info blue
    pub const GRADIENT_PURPLE: Color32 = Color32::from_rgb(142, 68, 173); // Purple accent
    pub const GRADIENT_PINK: Color32 = Color32::from_rgb(231, 76, 60); // Pink accent
    pub const GRADIENT_TEAL: Color32 = Color32::from_rgb(26, 188, 156); // Teal accent

    // Interactive states - Light mode
    pub const HOVER: Color32 = Color32::from_rgb(200, 220, 250);
    pub const PRESSED: Color32 = Color32::from_rgb(180, 200, 240);
    pub const SELECTED: Color32 = Color32::from_rgb(190, 210, 245);
    pub const DISABLED: Color32 = Color32::from_rgb(189, 195, 199);

    // Interactive states - Dark mode
    pub const DARK_HOVER: Color32 = Color32::from_rgb(45, 45, 55);
    pub const DARK_PRESSED: Color32 = Color32::from_rgb(55, 55, 65);
    pub const DARK_SELECTED: Color32 = Color32::from_rgb(50, 70, 100);
    pub const DARK_DISABLED: Color32 = Color32::from_rgb(80, 80, 80);

    // Glass morphism colors (non-const functions)
    pub fn surface_elevated(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgba_unmultiplied(40, 40, 40, 240)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 250)
        }
    }

    pub fn glass_white(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgba_unmultiplied(60, 60, 60, 180)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 180)
        }
    }

    pub fn glass_blue(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgba_unmultiplied(0, 141, 228, 60)
        } else {
            Color32::from_rgba_unmultiplied(0, 141, 228, 40)
        }
    }

    pub fn glass_border(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgba_unmultiplied(100, 100, 100, 80)
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 60)
        }
    }

    // Animated gradient colors
    pub fn gradient_animated(time: f32) -> Color32 {
        let t = (time.sin() + 1.0) / 2.0;
        let r = (0.0 * (1.0 - t) + 142.0 * t) as u8;
        let g = (141.0 * (1.0 - t) + 68.0 * t) as u8;
        let b = (228.0 * (1.0 - t) + 173.0 * t) as u8;
        Color32::from_rgb(r, g, b)
    }

    pub fn pastel_gradient(index: usize) -> Color32 {
        match index % 6 {
            0 => Color32::from_rgb(255, 182, 193), // Light Pink
            1 => Color32::from_rgb(255, 218, 185), // Peach
            2 => Color32::from_rgb(255, 255, 224), // Light Yellow
            3 => Color32::from_rgb(193, 255, 193), // Light Green
            4 => Color32::from_rgb(224, 255, 255), // Light Cyan
            5 => Color32::from_rgb(230, 230, 250), // Lavender
            _ => Color32::from_rgb(255, 192, 203), // Pink
        }
    }

    // Theme-aware color getters
    pub fn background(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_BACKGROUND
        } else {
            Self::BACKGROUND
        }
    }

    pub fn surface(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_SURFACE
        } else {
            Self::SURFACE
        }
    }

    pub fn input_background(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_INPUT_BACKGROUND
        } else {
            Self::INPUT_BACKGROUND
        }
    }

    pub fn border(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_BORDER
        } else {
            Self::BORDER
        }
    }

    pub fn border_light(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_BORDER_LIGHT
        } else {
            Self::BORDER_LIGHT
        }
    }

    pub fn text_primary(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_TEXT_PRIMARY
        } else {
            Self::TEXT_PRIMARY
        }
    }

    pub fn text_secondary(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_TEXT_SECONDARY
        } else {
            Self::TEXT_SECONDARY
        }
    }

    pub fn hover(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_HOVER
        } else {
            Self::HOVER
        }
    }

    pub fn pressed(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_PRESSED
        } else {
            Self::PRESSED
        }
    }

    pub fn selected(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_SELECTED
        } else {
            Self::SELECTED
        }
    }

    pub fn disabled(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_DISABLED
        } else {
            Self::DISABLED
        }
    }

    // Semantic colors that adapt to theme
    pub fn error_color(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgb(255, 100, 100) // Lighter red for dark mode
        } else {
            Color32::DARK_RED
        }
    }

    pub fn success_color(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgb(80, 160, 80) // Darker muted green for dark mode
        } else {
            Color32::DARK_GREEN
        }
    }

    pub fn warning_color(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgb(255, 200, 100) // Lighter orange for dark mode
        } else {
            Color32::from_rgb(255, 140, 0) // Dark orange
        }
    }

    /// Magenta-red for sync/connection error state (distinct from disconnected red).
    pub fn sync_error_color(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgb(230, 80, 180)
        } else {
            Color32::from_rgb(200, 50, 150)
        }
    }

    pub fn info_color(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgb(100, 180, 255) // Lighter blue for dark mode
        } else {
            Self::DEEP_BLUE
        }
    }

    /// Returns the foreground (text/border) color for a message severity level.
    pub fn message_color(message_type: crate::ui::MessageType, dark_mode: bool) -> Color32 {
        match message_type {
            crate::ui::MessageType::Error => Self::error_color(dark_mode),
            crate::ui::MessageType::Warning => Self::warning_color(dark_mode),
            crate::ui::MessageType::Success => Self::success_color(dark_mode),
            crate::ui::MessageType::Info => Self::info_color(dark_mode),
        }
    }

    /// Returns the tinted background color for a message severity level.
    pub fn message_background_color(
        message_type: crate::ui::MessageType,
        dark_mode: bool,
    ) -> Color32 {
        let alpha = if dark_mode { 30 } else { 20 };
        match message_type {
            crate::ui::MessageType::Error => {
                let c = if dark_mode {
                    (255, 100, 100)
                } else {
                    (235, 87, 87)
                };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
            crate::ui::MessageType::Warning => {
                let c = if dark_mode {
                    (255, 200, 100)
                } else {
                    (241, 196, 15)
                };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
            crate::ui::MessageType::Success => {
                let c = if dark_mode {
                    (80, 200, 120)
                } else {
                    (39, 174, 96)
                };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
            crate::ui::MessageType::Info => {
                let c = if dark_mode {
                    (100, 180, 255)
                } else {
                    (52, 152, 219)
                };
                Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha)
            }
        }
    }

    pub fn muted_color(dark_mode: bool) -> Color32 {
        if dark_mode {
            Color32::from_rgb(150, 150, 150) // Lighter gray for dark mode
        } else {
            Color32::GRAY
        }
    }

    // Modal/popup overlay colors

    /// Semi-transparent black overlay behind modals/popups
    pub fn modal_overlay() -> Color32 {
        Color32::from_rgba_unmultiplied(0, 0, 0, 120)
    }

    /// Shadow color for popup/dialog frames
    pub fn popup_shadow() -> Color32 {
        Color32::from_rgba_unmultiplied(0, 0, 0, 100)
    }

    /// Subtle border glow for popup/dialog windows
    pub fn popup_border_glow() -> Color32 {
        Color32::from_rgba_unmultiplied(255, 255, 255, 30)
    }

    // Secondary button colors (theme-aware)
    /// Secondary button colors — must contrast against dialog window_fill
    /// Light: darker than BACKGROUND (240,242,247) so buttons are visible on dialogs
    /// Dark: lighter than DARK_BACKGROUND (18,18,18) for the same reason
    pub const SECONDARY_BUTTON_FILL_LIGHT: Color32 = Color32::from_rgb(218, 222, 230);
    pub const SECONDARY_BUTTON_FILL_DARK: Color32 = Color32::from_rgb(50, 52, 58);
    pub const SECONDARY_BUTTON_TEXT_LIGHT: Color32 = Self::BLACK; // (17, 25, 33)
    pub const SECONDARY_BUTTON_TEXT_DARK: Color32 = Self::DARK_TEXT_PRIMARY; // (240, 240, 240)
    pub const SECONDARY_BUTTON_STROKE_LIGHT: Color32 = Color32::from_rgb(195, 200, 212);
    pub const SECONDARY_BUTTON_STROKE_DARK: Color32 = Color32::from_rgb(72, 75, 82);

    /// Popup fill color adapting to dark/light mode
    pub fn popup_fill(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_INPUT_BACKGROUND // rgb(40, 40, 40)
        } else {
            Self::WHITE
        }
    }

    /// Subtle stripe color for alternating table rows in dark mode
    pub fn stripe_dark() -> Color32 {
        Color32::from_rgba_unmultiplied(255, 255, 255, 10)
    }

    /// Subtle stripe color for alternating table rows in light mode
    pub fn stripe_light() -> Color32 {
        Color32::from_rgba_unmultiplied(0, 100, 200, 10)
    }

    /// Stripe color adapting to dark/light mode
    pub fn stripe(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::stripe_dark()
        } else {
            Self::stripe_light()
        }
    }

    /// Fill color for unselected toggle/segmented buttons
    pub fn unselected_fill(dark_mode: bool) -> Color32 {
        if dark_mode {
            Self::DARK_BORDER // rgb(60, 60, 60)
        } else {
            Color32::from_rgb(220, 220, 220)
        }
    }

    // Network accent color helpers

    /// Returns the accent color for a given network, adapting to dark/light mode
    pub fn network_accent(
        network: dash_sdk::dashcore_rpc::dashcore::Network,
        dark_mode: bool,
    ) -> Color32 {
        match network {
            dash_sdk::dashcore_rpc::dashcore::Network::Dash => {
                if dark_mode {
                    Self::DASH_BLUE_DARK
                } else {
                    Self::DASH_BLUE
                }
            }
            dash_sdk::dashcore_rpc::dashcore::Network::Testnet => {
                if dark_mode {
                    Self::TESTNET_ORANGE_DARK
                } else {
                    Self::TESTNET_ORANGE
                }
            }
            dash_sdk::dashcore_rpc::dashcore::Network::Devnet => {
                if dark_mode {
                    Self::DEVNET_RED_DARK
                } else {
                    Self::DEVNET_RED
                }
            }
            dash_sdk::dashcore_rpc::dashcore::Network::Regtest => {
                if dark_mode {
                    Self::REGTEST_BROWN_DARK
                } else {
                    Self::REGTEST_BROWN
                }
            }
            _ => {
                if dark_mode {
                    Self::DASH_BLUE_DARK
                } else {
                    Self::DASH_BLUE
                }
            }
        }
    }

    /// Returns the network label color (used in left panel, always light-mode tones)
    pub fn network_label_color(network: dash_sdk::dashcore_rpc::dashcore::Network) -> Color32 {
        match network {
            dash_sdk::dashcore_rpc::dashcore::Network::Testnet => Self::TESTNET_ORANGE,
            dash_sdk::dashcore_rpc::dashcore::Network::Devnet => Self::DEVNET_RED,
            dash_sdk::dashcore_rpc::dashcore::Network::Regtest => Self::REGTEST_BROWN,
            _ => Self::DASH_BLUE,
        }
    }

    /// Icon tint color based on selection state and dark mode
    pub fn icon_tint(selected: bool, dark_mode: bool) -> Color32 {
        if selected {
            Self::ICON_SELECTED
        } else if dark_mode {
            Self::ICON_UNSELECTED_DARK
        } else {
            Self::ICON_UNSELECTED_LIGHT
        }
    }
}

/// Typography scale and font configuration
pub struct Typography;

#[allow(dead_code)]
impl Typography {
    pub const SCALE_XS: f32 = 12.0;
    pub const SCALE_SM: f32 = 14.0;
    pub const SCALE_BASE: f32 = 16.0;
    pub const SCALE_LG: f32 = 18.0;
    pub const SCALE_XL: f32 = 20.0;
    pub const SCALE_XXL: f32 = 24.0;
    pub const SCALE_XXXL: f32 = 30.0;
    pub const SCALE_DISPLAY: f32 = 36.0;

    pub fn heading_xlarge() -> FontId {
        FontId::new(Self::SCALE_DISPLAY, FontFamily::Proportional)
    }

    pub fn heading_large() -> FontId {
        FontId::new(Self::SCALE_XXXL, FontFamily::Proportional)
    }

    pub fn heading_medium() -> FontId {
        FontId::new(Self::SCALE_XXL, FontFamily::Proportional)
    }

    pub fn heading_small() -> FontId {
        FontId::new(Self::SCALE_XL, FontFamily::Proportional)
    }

    pub fn body_large() -> FontId {
        FontId::new(Self::SCALE_LG, FontFamily::Proportional)
    }

    pub fn body() -> FontId {
        FontId::new(Self::SCALE_BASE, FontFamily::Proportional)
    }

    pub fn body_small() -> FontId {
        FontId::new(Self::SCALE_SM, FontFamily::Proportional)
    }

    pub fn caption() -> FontId {
        FontId::new(Self::SCALE_XS, FontFamily::Proportional)
    }

    pub fn monospace() -> FontId {
        FontId::new(Self::SCALE_BASE, FontFamily::Monospace)
    }

    pub fn button() -> FontId {
        FontId::new(Self::SCALE_BASE, FontFamily::Proportional)
    }
}

/// Spacing constants for consistent layout
pub struct Spacing;

#[allow(dead_code)]
impl Spacing {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 16.0;
    pub const LG: f32 = 24.0;
    pub const XL: f32 = 32.0;
    pub const XXL: f32 = 48.0;
    pub const XXXL: f32 = 64.0;

    // For egui Margin which expects i8
    pub const MD_I8: i8 = 16;
    pub const SM_I8: i8 = 8;

    pub const BUTTON_PADDING: Vec2 = Vec2::new(24.0, 12.0);
    pub const BUTTON_PADDING_SMALL: Vec2 = Vec2::new(16.0, 8.0);
    pub const BUTTON_PADDING_LARGE: Vec2 = Vec2::new(32.0, 16.0);

    pub const CARD_PADDING: f32 = 20.0;
    pub const SECTION_SPACING: f32 = 32.0;
    pub const FORM_SPACING: Vec2 = Vec2::new(16.0, 8.0);
}

/// Border radius and shape constants
pub struct Shape;

#[allow(dead_code)]
impl Shape {
    pub const RADIUS_NONE: u8 = 0;
    pub const RADIUS_SM: u8 = 6;
    pub const RADIUS_MD: u8 = 12;
    pub const RADIUS_LG: u8 = 16;
    pub const RADIUS_XL: u8 = 20;
    pub const RADIUS_FULL: u8 = 255;

    pub const BORDER_WIDTH: f32 = 1.0;
    pub const BORDER_WIDTH_THICK: f32 = 2.0;
}

/// Modern shadow definitions for depth and visual appeal
pub struct Shadow;

#[allow(dead_code)]
impl Shadow {
    pub fn small() -> egui::Shadow {
        egui::Shadow {
            offset: [0, 2],
            blur: 4,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 0, 0, 8),
        }
    }

    pub fn medium() -> egui::Shadow {
        egui::Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 0, 0, 12),
        }
    }

    pub fn large() -> egui::Shadow {
        egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 0, 0, 15),
        }
    }

    /// Modern elevated shadow for cards and panels
    pub fn elevated() -> egui::Shadow {
        egui::Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 0, 0, 18),
        }
    }

    /// Subtle inner shadow for glass morphism
    pub fn inner() -> egui::Shadow {
        egui::Shadow {
            offset: [0, 1],
            blur: 2,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(255, 255, 255, 25),
        }
    }

    /// Glow effect for primary elements
    pub fn glow() -> egui::Shadow {
        egui::Shadow {
            offset: [0, 0],
            blur: 20,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 141, 228, 30),
        }
    }
}

/// Component style definitions
pub struct ComponentStyles;

#[allow(dead_code)]
impl ComponentStyles {
    /// Standard minimum size for dialog buttons (width × height)
    pub const DIALOG_BUTTON_MIN_SIZE: Vec2 = Vec2::new(96.0, 36.0);

    pub fn primary_button_fill() -> Color32 {
        DashColors::DASH_BLUE
    }

    pub fn primary_button_text() -> Color32 {
        DashColors::WHITE
    }

    pub fn primary_button_stroke() -> Stroke {
        Stroke::new(1.0, DashColors::DASH_BLUE)
    }

    pub fn secondary_button_fill(dark_mode: bool) -> Color32 {
        if dark_mode {
            DashColors::SECONDARY_BUTTON_FILL_DARK
        } else {
            DashColors::SECONDARY_BUTTON_FILL_LIGHT
        }
    }

    pub fn secondary_button_text(dark_mode: bool) -> Color32 {
        if dark_mode {
            DashColors::SECONDARY_BUTTON_TEXT_DARK
        } else {
            DashColors::SECONDARY_BUTTON_TEXT_LIGHT
        }
    }

    pub fn secondary_button_stroke(dark_mode: bool) -> Stroke {
        if dark_mode {
            Stroke::new(1.0, DashColors::SECONDARY_BUTTON_STROKE_DARK)
        } else {
            Stroke::new(1.0, DashColors::SECONDARY_BUTTON_STROKE_LIGHT)
        }
    }

    pub fn danger_button_fill() -> Color32 {
        DashColors::ERROR
    }

    pub fn danger_button_text() -> Color32 {
        DashColors::WHITE
    }

    pub fn input_stroke() -> Stroke {
        Stroke::new(1.0, DashColors::BORDER)
    }

    pub fn input_stroke_focused() -> Stroke {
        Stroke::new(2.0, DashColors::DASH_BLUE)
    }

    pub fn input_stroke_error() -> Stroke {
        Stroke::new(2.0, DashColors::ERROR)
    }

    /// Returns a fully styled primary (action) button with Dash Blue fill and white text.
    ///
    /// Accepts any label type (`&str`, `String`, `RichText`, `WidgetText`).
    /// When a `RichText` is passed, its existing formatting (e.g. font size) is
    /// preserved and only `strong` + text color are applied on top.
    pub fn primary_button(label: impl Into<WidgetText>) -> Button<'static> {
        let text = match label.into() {
            WidgetText::RichText(rt) => rt
                .as_ref()
                .clone()
                .strong()
                .color(Self::primary_button_text()),
            // INTENTIONAL(CMT-010): LayoutJob/Galley variants not used by any callsite
            other => RichText::new(other.text().to_string())
                .strong()
                .color(Self::primary_button_text()),
        };
        Button::new(text)
            .fill(Self::primary_button_fill())
            .stroke(Self::primary_button_stroke())
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
            .min_size(Self::DIALOG_BUTTON_MIN_SIZE)
    }

    /// Returns a fully styled secondary (cancel/close) button with theme-aware colors.
    ///
    /// Accepts any label type (`&str`, `String`, `RichText`, `WidgetText`).
    pub fn secondary_button(label: impl Into<WidgetText>, dark_mode: bool) -> Button<'static> {
        let text = match label.into() {
            WidgetText::RichText(rt) => rt
                .as_ref()
                .clone()
                .strong()
                .color(Self::secondary_button_text(dark_mode)),
            // INTENTIONAL(CMT-010): LayoutJob/Galley variants not used by any callsite
            other => RichText::new(other.text().to_string())
                .strong()
                .color(Self::secondary_button_text(dark_mode)),
        };
        Button::new(text)
            .fill(Self::secondary_button_fill(dark_mode))
            .stroke(Self::secondary_button_stroke(dark_mode))
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
            .min_size(Self::DIALOG_BUTTON_MIN_SIZE)
    }

    /// Returns a fully styled danger (destructive action) button with red fill and white text.
    ///
    /// Accepts any label type (`&str`, `String`, `RichText`, `WidgetText`).
    pub fn danger_button(label: impl Into<WidgetText>) -> Button<'static> {
        let text = match label.into() {
            WidgetText::RichText(rt) => rt
                .as_ref()
                .clone()
                .strong()
                .color(Self::danger_button_text()),
            // INTENTIONAL(CMT-010): LayoutJob/Galley variants not used by any callsite
            other => RichText::new(other.text().to_string())
                .strong()
                .color(Self::danger_button_text()),
        };
        Button::new(text)
            .fill(Self::danger_button_fill())
            .stroke(egui::Stroke::NONE)
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
            .min_size(Self::DIALOG_BUTTON_MIN_SIZE)
    }

    /// Add a primary button to the UI with pointer cursor on hover.
    pub fn add_primary_button(ui: &mut egui::Ui, label: impl Into<WidgetText>) -> egui::Response {
        ui.add(Self::primary_button(label))
            .on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Add a primary button (conditionally enabled) with pointer cursor on hover.
    ///
    /// When disabled, uses distinct greyed-out fill and text so the button
    /// visually reads as inactive (egui's default disabled visuals are bypassed
    /// by the explicit fill/text styling in `primary_button()`).
    pub fn add_primary_button_enabled(
        ui: &mut egui::Ui,
        enabled: bool,
        label: impl Into<WidgetText>,
    ) -> egui::Response {
        let button = if enabled {
            Self::primary_button(label)
        } else {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            let text = match label.into() {
                WidgetText::RichText(rt) => rt
                    .as_ref()
                    .clone()
                    .strong()
                    .color(DashColors::disabled(dark_mode)),
                // INTENTIONAL(CMT-010): LayoutJob/Galley variants not used by any callsite
                other => RichText::new(other.text().to_string())
                    .strong()
                    .color(DashColors::disabled(dark_mode)),
            };
            Button::new(text)
                .fill(DashColors::BUTTON_DISABLED)
                .stroke(egui::Stroke::NONE)
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                .min_size(Self::DIALOG_BUTTON_MIN_SIZE)
        };
        ui.add_enabled(enabled, button)
            .on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Add a secondary button to the UI with pointer cursor on hover.
    pub fn add_secondary_button(
        ui: &mut egui::Ui,
        label: impl Into<WidgetText>,
        dark_mode: bool,
    ) -> egui::Response {
        ui.add(Self::secondary_button(label, dark_mode))
            .on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Add a danger button to the UI with pointer cursor on hover.
    pub fn add_danger_button(ui: &mut egui::Ui, label: impl Into<WidgetText>) -> egui::Response {
        ui.add(Self::danger_button(label))
            .on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Add any custom-styled button to the UI with pointer cursor on hover.
    ///
    /// Use this for buttons that don't fit the primary/secondary/danger/toolbar helpers.
    pub fn add_button(ui: &mut egui::Ui, button: Button<'_>) -> egui::Response {
        ui.add(button).on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Height for toolbar buttons in the top panel.
    const TOOLBAR_BUTTON_HEIGHT: f32 = 30.0;

    /// Returns a styled toolbar button with white text on the given accent fill.
    ///
    /// Used for top-panel action buttons (Register Name, Refresh, Documents, etc.)
    /// whose fill color depends on the active network.
    pub fn toolbar_button(label: impl Into<WidgetText>, fill: egui::Color32) -> Button<'static> {
        let text = match label.into() {
            WidgetText::RichText(rt) => rt.as_ref().clone().color(DashColors::WHITE),
            // INTENTIONAL(CMT-010): LayoutJob/Galley variants not used by any callsite
            other => RichText::new(other.text().to_string()).color(DashColors::WHITE),
        };
        Button::new(text)
            .fill(fill)
            .frame(true)
            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
            .stroke(egui::Stroke::NONE)
            .min_size(Vec2::new(100.0, Self::TOOLBAR_BUTTON_HEIGHT))
    }

    /// Add a toolbar button to the UI with pointer cursor on hover.
    pub fn add_toolbar_button(
        ui: &mut egui::Ui,
        label: impl Into<WidgetText>,
        fill: egui::Color32,
    ) -> egui::Response {
        ui.add(Self::toolbar_button(label, fill))
            .on_hover_cursor(CursorIcon::PointingHand)
    }
}

/// Extension methods for [`egui::Response`] that enforce project-wide UI behavior policies.
///
/// This is the general-purpose extension point for `egui::Response`. Current scope:
/// cursor icons (tooltip methods below). Future methods may enforce hover effects,
/// accessibility attributes, interaction patterns, or other UX conventions.
///
/// Use these instead of raw `.on_hover_text()` to ensure consistent behavior.
pub trait ResponseExt {
    /// Informational tooltip with `Help` (?) cursor. For non-interactive elements
    /// that show explanatory text on hover (status labels, setting descriptions).
    fn info_tooltip(self, text: impl Into<egui::WidgetText>) -> Self;

    /// Clickable tooltip with `PointingHand` cursor. For interactive elements
    /// that show a tooltip and respond to clicks (custom clickable labels, links).
    fn clickable_tooltip(self, text: impl Into<egui::WidgetText>) -> Self;

    /// Disabled tooltip with `NotAllowed` cursor. For disabled elements
    /// that explain why the action is unavailable.
    fn disabled_tooltip(self, text: impl Into<egui::WidgetText>) -> Self;
}

impl ResponseExt for egui::Response {
    fn info_tooltip(self, text: impl Into<egui::WidgetText>) -> Self {
        self.on_hover_text(text).on_hover_cursor(CursorIcon::Help)
    }

    fn clickable_tooltip(self, text: impl Into<egui::WidgetText>) -> Self {
        self.on_hover_text(text)
            .on_hover_cursor(CursorIcon::PointingHand)
    }

    fn disabled_tooltip(self, text: impl Into<egui::WidgetText>) -> Self {
        self.on_disabled_hover_text(text)
            .on_hover_cursor(CursorIcon::NotAllowed)
    }
}

/// Configure fonts for the application
pub fn configure_fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();

    // Load Noto Sans font for better international support
    fonts.font_data.insert(
        "NotoSans".to_owned(),
        FontData::from_static(include_bytes!(
            "../../assets/Fonts/Noto_Sans/NotoSans-VariableFont.ttf"
        ))
        .into(),
    );

    // Add NotoSans to the proportional font family (used for UI text)
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "NotoSans".to_owned());

    fonts
}

/// Apply the modern Dash theme to the egui context
pub fn apply_theme(ctx: &egui::Context, theme_mode: ThemeMode) {
    // Resolve the actual theme to use
    let resolved_theme = resolve_theme_mode(theme_mode);
    let dark_mode = resolved_theme == ThemeMode::Dark;

    // Start with appropriate base mode
    let mut visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    // Override ALL background-related properties with our custom colors
    visuals.window_fill = DashColors::background(dark_mode);
    visuals.panel_fill = DashColors::background(dark_mode);
    visuals.extreme_bg_color = DashColors::input_background(dark_mode);
    visuals.faint_bg_color = DashColors::background(dark_mode);
    visuals.code_bg_color = if dark_mode {
        Color32::from_rgb(30, 30, 30)
    } else {
        Color32::from_rgb(245, 245, 245)
    };

    // Set dark mode flag correctly
    visuals.dark_mode = dark_mode;

    // Apply the custom visuals first
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();

    // Configure modern visuals with gradients and glass effects
    // Override all background colors again to ensure they stick
    style.visuals.window_fill = DashColors::background(dark_mode);
    style.visuals.panel_fill = DashColors::background(dark_mode);
    style.visuals.extreme_bg_color = DashColors::input_background(dark_mode);
    style.visuals.faint_bg_color = DashColors::background(dark_mode);
    style.visuals.dark_mode = dark_mode;
    style.visuals.window_stroke = Stroke::new(1.0, DashColors::border(dark_mode));
    // Note: window_rounding is not available in this egui version
    style.visuals.window_shadow = Shadow::elevated();

    // Modern widget styling with solid backgrounds for buttons
    style.visuals.widgets.inactive.bg_fill = DashColors::background(dark_mode);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, DashColors::border(dark_mode));
    style.visuals.widgets.inactive.fg_stroke.color = DashColors::text_primary(dark_mode);
    style.visuals.widgets.inactive.weak_bg_fill = DashColors::background(dark_mode);
    style.visuals.widgets.inactive.expansion = 0.0;

    // Hover state with highlighted background
    style.visuals.widgets.hovered.bg_fill = DashColors::hover(dark_mode);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, DashColors::DASH_BLUE);
    style.visuals.widgets.hovered.fg_stroke.color = DashColors::DASH_BLUE;
    style.visuals.widgets.hovered.weak_bg_fill = DashColors::hover(dark_mode);
    style.visuals.widgets.hovered.expansion = 2.0;

    // Active state with enhanced feedback
    style.visuals.widgets.active.bg_fill = DashColors::GRADIENT_START;
    style.visuals.widgets.active.bg_stroke = Stroke::new(2.0, DashColors::GRADIENT_END);
    style.visuals.widgets.active.fg_stroke.color = DashColors::WHITE;
    style.visuals.widgets.active.weak_bg_fill = DashColors::GRADIENT_START;
    style.visuals.widgets.active.expansion = 1.0;

    // Text input fields - ensure appropriate background with contrasting text
    // Note: TextEdit uses extreme_bg_color by default, but we also set noninteractive for consistency
    style.visuals.widgets.noninteractive.bg_fill = DashColors::input_background(dark_mode);
    style.visuals.widgets.noninteractive.bg_stroke =
        Stroke::new(1.0, DashColors::border(dark_mode));
    style.visuals.widgets.noninteractive.weak_bg_fill = DashColors::input_background(dark_mode);
    style.visuals.widgets.noninteractive.fg_stroke.color = DashColors::text_primary(dark_mode);

    // Open state is also used for focused text inputs
    style.visuals.widgets.open.bg_fill = DashColors::input_background(dark_mode);
    style.visuals.widgets.open.weak_bg_fill = DashColors::input_background(dark_mode);
    style.visuals.widgets.open.bg_stroke = Stroke::new(2.0, DashColors::DASH_BLUE);
    style.visuals.widgets.open.fg_stroke.color = DashColors::text_primary(dark_mode);

    // Specific text input colors
    style.visuals.text_cursor.stroke = Stroke::new(1.0, DashColors::text_primary(dark_mode));

    // Text colors - ensure contrasting text on all elements
    style.visuals.override_text_color = Some(DashColors::text_primary(dark_mode));

    // Text selection
    style.visuals.selection.bg_fill = DashColors::selected(dark_mode);
    style.visuals.selection.stroke = Stroke::new(1.0, DashColors::DASH_BLUE);

    // Hyperlinks
    style.visuals.hyperlink_color = DashColors::DASH_BLUE;

    // Code styling - use appropriate background for better contrast
    style.visuals.code_bg_color = if dark_mode {
        Color32::from_rgb(30, 30, 30)
    } else {
        Color32::from_rgb(245, 245, 245)
    };

    // Note: extreme_bg_color is already set to INPUT_BACKGROUND above for TextEdit widgets

    // Enhance dropdowns and menus
    style.visuals.popup_shadow = Shadow::medium();

    // Apply improved spacing
    style.spacing.item_spacing = Vec2::new(Spacing::SM, Spacing::SM);
    style.spacing.button_padding = Vec2::new(16.0, 8.0);
    style.spacing.menu_margin = egui::Margin::same(4);
    style.spacing.indent = Spacing::MD;
    style.spacing.icon_width = 14.0; // Reduced from 18.0
    style.spacing.icon_width_inner = 12.0; // Reduced from 16.0
    style.spacing.icon_spacing = 4.0; // Reduced from 6.0

    // Final override of all background colors to ensure they are definitely set
    style.visuals.window_fill = DashColors::background(dark_mode);
    style.visuals.panel_fill = DashColors::background(dark_mode);
    // Don't override extreme_bg_color here - it should remain as input_background for TextEdit widgets
    style.visuals.faint_bg_color = DashColors::background(dark_mode);

    ctx.set_style(style);
    ctx.set_fonts(configure_fonts());
}
