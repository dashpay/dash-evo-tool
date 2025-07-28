# UI Component Design Pattern

## Overview

This document describes the design pattern used for UI components in the Dash Evo Tool project. This pattern provides efficient, maintainable, and user-friendly components that work well with egui's immediate mode GUI paradigm. Components following this pattern implement the `Component` trait defined in `src/ui/components/component_trait.rs`.

For a complete example implementation, see `AmountInput` in `src/ui/components/amount_input.rs`.

## Core Principles

### 1. Self-Contained State Management
Components manage their own internal state and expose only what's necessary through responses.

```rust
pub struct MyInputComponent {
    // Private internal state
    internal_value: String,
    domain_config: DomainType,
    label: Option<WidgetText>,
    // ... other private fields
}
```

**Benefits:**
- Encapsulation prevents misuse of internal state
- Component is responsible for its own consistency
- Easier to reason about component behavior

### 2. Lazy Initialization Pattern
Components are created only when first needed, reducing memory usage and initialization overhead.

```rust
struct MyScreen {
    input_component: Option<MyInputComponent>,  // Lazy initialization
}

impl MyScreen {
    fn show(&mut self, ui: &mut Ui) {
        // Create component only when first needed
        let input_component = self.input_component.get_or_insert_with(|| {
            MyInputComponent::new(domain_object)
                .label("Input:")
                .max_value(Some(1000000))
        });
        
        // Use egui's native enabled state for dynamic control
        let response = ui.add_enabled_ui(!operation_in_progress, |ui| {
            input_component.show(ui)
        }).inner;
        // Handle response...
    }
}
```

### 3. Dual Builder API Pattern
Provide both owned (consuming) and mutable reference versions of configuration methods.

```rust
impl MyInputComponent {
    // Owned version - for initial configuration
    pub fn label<T: Into<WidgetText>>(mut self, label: T) -> Self {
        self.label = Some(label.into());
        self
    }
    
    // Mutable reference version - for dynamic updates
    pub fn set_label<T: Into<WidgetText>>(&mut self, label: T) -> &mut Self {
        self.label = Some(label.into());
        self
    }
}
```

### 4. Response-Based Communication
Components communicate state changes through structured response objects.

```rust
pub struct MyInputResponse {
    pub response: Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub parsed_data: Option<ParsedResult>,
}
```

### 5. Type-Driven Configuration
Component behavior is determined by the data types it works with, not manual configuration.

```rust
// ✅ Configuration determined by domain object
let input_component = MyInputComponent::new(domain_object_with_metadata);
```

### 6. egui Native Enabled State
Components use egui's built-in enabled/disabled state rather than managing their own enabled field.

```rust
// ✅ Use egui's add_enabled_ui for controlling enabled state
let response = ui.add_enabled_ui(!operation_in_progress, |ui| {
    component.show(ui)
}).inner;

// ❌ Don't manage enabled state in components
// component.set_enabled(false); // REMOVED from trait
```

## Component Trait System

### Core Traits

- **`ComponentResponse`**: Implemented by response types to provide consistent access to basic properties
- **`Component`**: Core trait for all components, defines the main interface
- **`ComponentWithCallbacks`**: Optional trait for components supporting callback functions  
- **`UpdatableComponent`**: Utility trait for components that manage optional values

### Implementing a New Component

```rust
use crate::ui::components::{Component, ComponentResponse};
use egui::{InnerResponse, Response, Ui, WidgetText};

// 1. Define the response struct
#[derive(Clone)]
pub struct TextInputResponse {
    pub response: Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub validated_text: Option<String>,
}

// 2. Implement ComponentResponse for the response
impl ComponentResponse for TextInputResponse {
    fn has_changed(&self) -> bool { self.changed }
    fn is_valid(&self) -> bool { self.error_message.is_none() }
    fn error_message(&self) -> Option<&str> { self.error_message.as_deref() }
}

// 3. Define the component struct (no enabled field needed)
pub struct TextInputComponent {
    current_text: String,
    validation_rules: ValidationRules,
    label: Option<WidgetText>,
}

// 4. Implement the Component trait
impl Component for TextInputComponent {
    type DomainType = ValidationRules;
    type Response = TextInputResponse;
    
    fn new(domain_object: Self::DomainType) -> Self {
        Self {
            current_text: String::new(),
            validation_rules: domain_object,
            label: None,
        }
    }
    
    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        ui.horizontal(|ui| {
            if let Some(label) = &self.label {
                ui.label(label.clone());
            }
            
            let response = ui.add(egui::TextEdit::singleline(&mut self.current_text));
            let changed = response.changed();
            let (error_message, validated_text) = if changed {
                self.validate_text()
            } else {
                (None, None)
            };
            
            TextInputResponse {
                response,
                changed,
                error_message,
                validated_text,
            }
        })
    }
}

// 5. Add builder methods following the dual API pattern
impl TextInputComponent {
    pub fn label<T: Into<WidgetText>>(mut self, label: T) -> Self {
        self.label = Some(label.into());
        self
    }
    
    pub fn set_label<T: Into<WidgetText>>(&mut self, label: T) -> &mut Self {
        self.label = Some(label.into());
        self
    }
    
    fn validate_text(&self) -> (Option<String>, Option<String>) {
        // Validation logic based on self.validation_rules
        // Return (error_message, validated_text)
    }
}
```

### Usage in Screens

```rust
struct MyScreen {
    text_input: Option<TextInputComponent>,
    current_text: Option<String>,
    operation_in_progress: bool,
}

impl MyScreen {
    fn render_text_input(&mut self, ui: &mut Ui) {
        let component = self.text_input.get_or_insert_with(|| {
            TextInputComponent::new(validation_rules)
                .label("Enter text:")
        });
        
        // Use egui's enabled state instead of component methods
        let response = ui.add_enabled_ui(!self.operation_in_progress, |ui| {
            component.show(ui)
        }).inner;
        
        // Handle state changes correctly
        if response.inner.has_changed() {
            if response.inner.is_valid() {
                self.current_text = response.inner.validated_text;
            } else {
                self.current_text = None; // Clear on invalid input
            }
        }
        
        // Show errors
        if let Some(error) = response.inner.error_message() {
            ui.colored_label(egui::Color32::RED, error);
        }
    }
}
```

## Critical: Handling Invalid Input States

Always handle input state changes to prevent retaining stale data:

```rust
// ✅ CORRECT - Use the helper method for Option<T> fields
let response = input_component.show(ui);
if response.inner.update(&mut self.current_data) {
    println!("Data changed: {:?}", self.current_data);
}

// ✅ CORRECT - Manual handling for custom logic
let response = input_component.show(ui);
if response.inner.changed {
    if response.inner.error_message.is_none() {
        self.current_data = response.inner.parsed_data;
    } else {
        self.current_data = None; // Clear invalid data
    }
}
```

## When to Recreate Components

Recreate components when:
- Core configuration changes (e.g., data type structure changes)
- The component type fundamentally changes
- You need to reset all internal state

## Benefits of This Pattern

1. **Performance**: Lazy initialization and efficient state management
2. **Maintainability**: Clear separation of concerns and encapsulation
3. **Flexibility**: Dual API supports both static and dynamic configuration
4. **Type Safety**: Configuration driven by domain objects prevents errors
5. **Testability**: Components can be tested in isolation
6. **Consistency**: Standardized pattern across all components
7. **egui Integration**: Uses framework's native enabled/disabled state

## Anti-Patterns to Avoid

- ❌ Public mutable fields (breaks encapsulation)
- ❌ Manual configuration of derived properties (derive from domain objects instead)
- ❌ Eager initialization of all components (use lazy initialization)
- ❌ Managing enabled state in components (use egui's `add_enabled_ui` instead)
- ❌ Not implementing the Component trait when following this pattern

## Example: AmountInput Implementation

The `AmountInput` component in `src/ui/components/amount_input.rs` exemplifies this pattern:

- **Self-contained**: Manages its own string state and validation
- **Lazy initialization**: Created only when first displayed  
- **Dual API**: Both `label()` and `set_label()` methods
- **Response-based**: Returns `AmountInputResponse` with validation results
- **Type-driven**: Decimal places determined by `Amount` object
- **egui Integration**: Uses egui's native enabled state (no internal enabled field)
- **Trait implementation**: Implements `Component`, `ComponentWithCallbacks`, and `UpdatableComponent`

Refer to the AmountInput implementation for a complete, working example of this pattern.
