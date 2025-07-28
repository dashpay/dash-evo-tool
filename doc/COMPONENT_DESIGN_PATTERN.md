# UI Component Design Pattern

## Overview

This document describes the design pattern used for UI components in the Dash Evo Tool project. This pattern provides efficient, maintainable, and user-friendly components that work well with egui's immediate mode GUI paradigm. Components following this pattern implement the `Component` trait defined in `src/ui/components/component_trait.rs`.

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
                .show_action_button(true)
        });
        
        let response = input_component.show(ui);
        // Handle response...
    }
}
```

**Benefits:**
- Reduced memory footprint
- Faster screen initialization
- Components created only when actually displayed

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

**Usage Guidelines:**
- **Static configuration**: Use owned methods during `get_or_insert_with()`
- **Dynamic configuration**: Use `set_*()` methods for runtime changes

### 4. Response-Based Communication with Optional Callbacks
Components primarily communicate state changes through structured response objects, with optional callback support for specialized use cases.

```rust
pub struct MyInputResponse {
    pub response: Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub action_clicked: bool,
    pub parsed_data: Option<ParsedResult>,
}
```

**Primary Pattern - Response Objects:**
- Clear communication contract
- Easy to handle multiple response types
- Stateless and functional approach

**Optional Pattern - Callbacks:**
Components may also support optional callbacks for scenarios requiring immediate response to state changes:

```rust
impl MyComponent {
    // Optional callback for success scenarios
    pub fn on_success(mut self, callback: impl FnMut(&ShowResponse) + 'static) -> Self {
        self.on_success_fn = Some(Box::new(callback));
        self
    }
    
    // Optional callback for error scenarios  
    pub fn on_error(mut self, callback: impl FnMut(&ShowResponse) + 'static) -> Self {
        self.on_error_fn = Some(Box::new(callback));
        self
    }
}
```

**When to Use Callbacks:**
- Immediate response to specific state changes is required
- Complex validation or processing needs to happen during input
- Integration with external systems that require event-driven patterns

**Callback Guidelines:**
- Always make callbacks optional (use `Option<Box<dyn FnMut>>`)
- Provide the full response object to callbacks for maximum flexibility
- Only trigger callbacks when the relevant change occurs
- Maintain response-based communication as the primary pattern

### 5. Immutable Configuration from Data Types
Component behavior is determined by the data types it works with, not manual configuration.

```rust
// ✅ Configuration determined by domain object
let input_component = MyInputComponent::new(domain_object_with_metadata);

// ❌ Manual configuration would be error-prone
// input_component.set_validation_rules(rules);
// input_component.set_format_options(options);
```

**Benefits:**
- Single source of truth for all type-specific behavior
- Type safety prevents inconsistent states
- Domain-specific properties are automatically preserved throughout the component lifecycle
- Clear relationship between data and presentation

## Component Trait System

The `Component` trait system provides a standardized interface for all UI components following this design pattern. Components should implement the appropriate traits from `src/ui/components/component_trait.rs`:

### Core Traits

- **`ComponentResponse`**: Implemented by response types to provide consistent access to basic properties
- **`Component`**: Core trait for all components, defines the main interface
- **`ComponentWithCallbacks`**: Optional trait for components supporting callback functions  
- **`UpdatableComponent`**: Utility trait for components that manage optional values

### Implementing a New Component

Here's a step-by-step guide to implementing a new component:

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

// 3. Define the component struct
pub struct TextInputComponent {
    current_text: String,
    validation_rules: ValidationRules, // From domain object
    label: Option<WidgetText>,
    enabled: bool,
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
            enabled: true,
        }
    }
    
    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        ui.horizontal(|ui| {
            if let Some(label) = &self.label {
                ui.label(label.clone());
            }
            
            let response = ui.add_enabled(
                self.enabled,
                egui::TextEdit::singleline(&mut self.current_text)
            );
            
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
    
    fn is_enabled(&self) -> bool { self.enabled }
    fn set_enabled(&mut self, enabled: bool) -> &mut Self {
        self.enabled = enabled;
        self
    }
}

// 5. Add builder methods following the dual API pattern
impl TextInputComponent {
    // Owned methods for static configuration
    pub fn label<T: Into<WidgetText>>(mut self, label: T) -> Self {
        self.label = Some(label.into());
        self
    }
    
    // Mutable reference methods for dynamic configuration
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
}

impl MyScreen {
    fn render_text_input(&mut self, ui: &mut Ui) {
        let component = self.text_input.get_or_insert_with(|| {
            TextInputComponent::new(validation_rules)
                .label("Enter text:")
        });
        
        component.set_enabled(!operation_in_progress);
        
        let response = component.show(ui);
        
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

## Implementation Guidelines

### Component Structure

```rust
use crate::ui::components::{Component, ComponentResponse};

#[derive(Clone)]  // Note: May need custom Clone implementation if using callbacks
pub struct MyInputComponent {
    // Private fields for internal state
    internal_state: String,
    domain_data: DomainType,
    config_field: Option<Value>,
    // Optional callback functions
    on_success_fn: Option<Box<dyn FnMut(&ShowResponse)>>,
    on_error_fn: Option<Box<dyn FnMut(&ShowResponse)>>,
    // ... other private fields
}

#[derive(Clone)]
pub struct MyInputResponse {
    pub response: Response,
    pub changed: bool,
    pub data: Option<ProcessedData>,
    pub error_message: Option<String>,
    // ... other response fields
}

impl ComponentResponse for MyInputResponse {
    fn has_changed(&self) -> bool { self.changed }
    fn is_valid(&self) -> bool { self.error_message.is_none() }
    fn error_message(&self) -> Option<&str> { self.error_message.as_deref() }
}

pub type ShowResponse = InnerResponse<MyInputResponse>;
```

### Required Methods

```rust
impl Component for MyInputComponent {
    type DomainType = DomainType;
    type Response = MyInputResponse;
    
    // Constructor from domain object
    fn new(domain_object: Self::DomainType) -> Self { /* ... */ }
    
    // Main render method that handles both responses and callbacks
    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let result = self.show_internal(ui);
        
        // Trigger callbacks when relevant changes occur
        if result.inner.changed {
            if result.inner.data.is_some() {
                if let Some(on_success_fn) = &mut self.on_success_fn {
                    on_success_fn(&result);
                }
            }
            
            if result.inner.error_message.is_some() {
                if let Some(on_error_fn) = &mut self.on_error_fn {
                    on_error_fn(&result);
                }
            }
        }
        
        result
    }
    
    fn is_enabled(&self) -> bool { self.enabled }
    fn set_enabled(&mut self, enabled: bool) -> &mut Self {
        self.enabled = enabled;
        self
    }
}

// Additional configuration methods
impl MyInputComponent {
    // Owned configuration methods
    pub fn config_option(mut self, value: T) -> Self { /* ... */ }
    
    // Mutable reference configuration methods
    pub fn set_config_option(&mut self, value: T) -> &mut Self { /* ... */ }
    
    // Optional callback configuration
    pub fn on_success(mut self, callback: impl FnMut(&ShowResponse) + 'static) -> Self { /* ... */ }
    pub fn on_error(mut self, callback: impl FnMut(&ShowResponse) + 'static) -> Self { /* ... */ }
    
    // Internal rendering logic
    fn show_internal(&mut self, ui: &mut Ui) -> InnerResponse<MyInputResponse> { /* ... */ }
}
```

### Screen Integration Pattern

```rust
pub struct MyScreen {
    my_component: Option<MyInputComponent>,
    // ... other screen state
}

impl MyScreen {
    fn render_my_component(&mut self, ui: &mut Ui) {
        // Static configuration during lazy initialization
        let component = self.my_component.get_or_insert_with(|| {
            MyInputComponent::new(domain_object)
                .static_config("value")
                .another_static_config(true)
                // Optional: Configure callbacks for immediate response
                .on_success(|response| {
                    println!("Success: {:?}", response.inner.data);
                })
                .on_error(|response| {
                    eprintln!("Error: {:?}", response.inner.error_message);
                })
        });
        
        // Dynamic configuration for runtime changes
        let response = component
            .set_dynamic_config(runtime_value)
            .show(ui);
            
        // Primary pattern: Handle response data
        if let Some(data) = response.inner.data {
            self.handle_data_change(data);
        }
        
        if let Some(error) = &response.inner.error_message {
            ui.colored_label(egui::Color32::DARK_RED, error);
        }
        
        // Note: Callbacks were already triggered during show() if configured
    }
}
```

## Critical: Handling Invalid Input States

When using input components, it's crucial to properly handle transitions between valid and invalid states. **Failure to do this correctly will cause your screen to retain stale data when the user enters invalid input.**

### The Problem

Consider this incorrect pattern that leads to bugs:

```rust
// ❌ INCORRECT - Only updates on valid input, retains stale data on invalid input
let response = input_component.show(ui);
if let Some(parsed_data) = response.inner.parsed_data {
    self.current_data = Some(parsed_data); // Only updates when valid
}
// BUG: If user enters valid data then invalid data, 
// self.current_data still holds the old valid value!
```

### The Solution

Always handle input state changes to prevent retaining stale data. For components that implement the `UpdatableComponent` trait, you can use the helper method:

```rust
// ✅ CORRECT - Use the helper method for Option<T> fields
let response = input_component.show(ui);
if response.inner.update(&mut self.current_data) {
    println!("Data changed: {:?}", self.current_data);
}

// ✅ CORRECT - For non-Option fields, use a temporary variable
let response = input_component.show(ui);
let mut temp_data: Option<DataType> = Some(self.data.clone());
if response.inner.update(&mut temp_data) {
    if let Some(data) = temp_data {
        self.data = data;
    } else {
        // No valid data - set to appropriate default
        self.data = DataType::default(); // or appropriate default
    }
}
```

You can also handle it manually if you need custom logic:

```rust
// ✅ CORRECT - Manual handling for custom logic
let response = input_component.show(ui);
if response.inner.changed {
    if response.inner.error_message.is_none() {
        // Input is valid - update our data (could be None for empty input)
        self.current_data = response.inner.parsed_data;
    } else {
        // Input is invalid - clear our data to prevent using stale values
        self.current_data = None;
    }
}
```

### Pattern for Different Component Types

This pattern applies to all input components. For components with helper methods:

```rust
// For Option<T> fields - simplest case
let response = input_component.show(ui);
if response.inner.update(&mut self.data_option) {
    // Data was updated (or cleared if invalid)
    // Type-specific properties are automatically preserved by the component
}

// For non-Option fields, use a temporary variable
let response = input_component.show(ui);
let mut temp_data = Some(self.data.clone());
if response.inner.update(&mut temp_data) {
    if let Some(data) = temp_data {
        self.data = data; // Type-specific properties automatically preserved
    } else {
        // Set appropriate default for invalid/empty input
        self.data = DataType::default();
    }
}

// For components without helper methods with validation
if response.inner.changed {
    if response.inner.error_message.is_none() {
        // Input is valid - use the parsed data
        self.component_data = response.inner.parsed_data;
    } else {
        // Input is invalid - clear to prevent stale data
        self.component_data = None; // or appropriate default/reset value
    }
}

// Always show errors to user
if let Some(error) = &response.inner.error_message {
    ui.colored_label(egui::Color32::RED, error);
}
```

### Why This Matters

- **Data Integrity**: Prevents operations with invalid/stale data
- **User Experience**: Clear feedback when input becomes invalid  
- **Business Logic**: Ensures validation rules are properly enforced
- **Debugging**: Eliminates confusing state where UI shows errors but data is still "valid"
```

## When to Recreate Components

Recreate components when:
- Core configuration changes (e.g., data type structure changes)
- The component type fundamentally changes
- You need to reset all internal state

```rust
// Check if recreation is needed
let needs_recreation = self.component
    .as_ref()
    .map(|comp| comp.core_property() != new_core_value)
    .unwrap_or(true);
    
if needs_recreation {
    self.component = Some(
        MyInputComponent::new(new_domain_object)
            .with_static_config()
    );
}
```

## Testing Guidelines

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_component_initialization() {
        let domain_object = DomainType::new(test_data);
        let component = MyInputComponent::new(domain_object);
        
        // Test initial state
        assert_eq!(component.some_property(), expected_value);
    }
    
    #[test]
    fn test_configuration_methods() {
        let component = MyInputComponent::new(test_domain_object)
            .config_option(test_value);
            
        // Test configuration was applied
        assert_eq!(component.internal_config, test_value);
    }
    
    #[test]
    fn test_component_trait_implementation() {
        let mut component = MyInputComponent::new(test_domain_object);
        
        // Test Component trait methods
        assert!(component.is_enabled());
        component.set_enabled(false);
        assert!(!component.is_enabled());
    }
}
```

## Benefits of This Pattern

1. **Performance**: Lazy initialization and efficient state management
2. **Maintainability**: Clear separation of concerns and encapsulation
3. **Flexibility**: Dual API supports both static and dynamic configuration
4. **Type Safety**: Configuration driven by domain objects prevents errors
5. **Testability**: Components can be tested in isolation
6. **Consistency**: Standardized pattern across all components
7. **Communication Options**: Both response-based and callback patterns available
8. **Progressive Enhancement**: Start with response handling, add callbacks when needed

## Anti-Patterns to Avoid

- ❌ Public mutable fields (breaks encapsulation)
- ❌ Manual configuration of derived properties (derive from domain objects instead)
- ❌ Callback-only APIs without response objects (prefer hybrid approach)
- ❌ Required callbacks (always make them optional)
- ❌ Eager initialization of all components (use lazy initialization)
- ❌ Mixing static and dynamic configuration inappropriately
- ❌ Complex callback chains (keep callbacks simple and focused)
- ❌ Not implementing the Component trait when following this pattern

## Example: AmountInput Implementation

The `AmountInput` component exemplifies this pattern and implements the `Component` trait:

- **Self-contained**: Manages its own string state and validation
- **Lazy initialization**: Created only when first displayed
- **Dual API**: Both `label()` and `set_label()` methods
- **Hybrid communication**: Returns `AmountInputResponse` with optional callbacks
- **Type-driven**: Decimal places determined by `Amount` object
- **Optional callbacks**: Supports `on_success()` and `on_error()` for immediate response
- **Trait implementation**: Implements `Component`, `ComponentWithCallbacks`, and `UpdatableComponent`

This pattern and trait system has proven effective for creating maintainable, performant UI components that integrate well with egui's immediate mode paradigm while providing excellent developer experience and flexible communication options.
