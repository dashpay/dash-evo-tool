# UI Component Design Pattern

## Overview

This document describes the design pattern used for UI components in the Dash Evo Tool project, exemplified by the `AmountInput` component. This pattern provides efficient, maintainable, and user-friendly components that work well with egui's immediate mode GUI paradigm.

## Core Principles

### 1. Self-Contained State Management
Components manage their own internal state and expose only what's necessary through responses.

```rust
pub struct AmountInput {
    // Private internal state
    amount_str: String,
    decimal_places: u8,
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
    amount_input: Option<AmountInput>,  // Lazy initialization
}

impl MyScreen {
    fn show(&mut self, ui: &mut Ui) {
        // Create component only when first needed
        let amount_input = self.amount_input.get_or_insert_with(|| {
            AmountInput::new(Amount::dash(0))
                .label("Amount:")
                .max_amount(Some(1000000))
                .show_max_button(true)
        });
        
        let response = amount_input.show(ui);
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
impl AmountInput {
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
pub struct AmountInputResponse {
    pub response: Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub max_clicked: bool,
    pub parsed_amount: Option<Amount>,
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
// ✅ Decimal places determined by Amount object
let amount_input = AmountInput::new(Amount::new(0, token_decimals));

// ❌ Manual configuration would be error-prone
// amount_input.set_decimal_places(token_decimals);
```

**Benefits:**
- Single source of truth
- Type safety prevents inconsistent states
- Clear relationship between data and presentation

## Implementation Guidelines

### Component Structure

```rust
#[derive(Clone)]  // Note: May need custom Clone implementation if using callbacks
pub struct MyComponent {
    // Private fields for internal state
    internal_state: String,
    config_field: Option<Value>,
    // Optional callback functions
    on_success_fn: Option<Box<dyn FnMut(&ShowResponse)>>,
    on_error_fn: Option<Box<dyn FnMut(&ShowResponse)>>,
    // ... other private fields
}

pub struct MyComponentResponse {
    pub response: Response,
    pub changed: bool,
    pub data: Option<ProcessedData>,
    pub error_message: Option<String>,
    // ... other response fields
}

pub type ShowResponse = InnerResponse<MyComponentResponse>;
```

### Required Methods

```rust
impl MyComponent {
    // Constructor from domain object
    pub fn new(domain_object: DomainType) -> Self { /* ... */ }
    
    // Owned configuration methods
    pub fn config_option(mut self, value: T) -> Self { /* ... */ }
    
    // Mutable reference configuration methods
    pub fn set_config_option(&mut self, value: T) -> &mut Self { /* ... */ }
    
    // Optional callback configuration
    pub fn on_success(mut self, callback: impl FnMut(&ShowResponse) + 'static) -> Self { /* ... */ }
    pub fn on_error(mut self, callback: impl FnMut(&ShowResponse) + 'static) -> Self { /* ... */ }
    
    // Main render method that handles both responses and callbacks
    pub fn show(&mut self, ui: &mut Ui) -> InnerResponse<MyComponentResponse> {
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
    
    // Internal rendering logic
    fn show_internal(&mut self, ui: &mut Ui) -> InnerResponse<MyComponentResponse> { /* ... */ }
}
```

### Screen Integration Pattern

```rust
pub struct MyScreen {
    my_component: Option<MyComponent>,
    // ... other screen state
}

impl MyScreen {
    fn render_my_component(&mut self, ui: &mut Ui) {
        // Static configuration during lazy initialization
        let component = self.my_component.get_or_insert_with(|| {
            MyComponent::new(domain_object)
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

## When to Recreate Components

Recreate components when:
- Core configuration changes (e.g., decimal places for amounts)
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
        MyComponent::new(new_domain_object)
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
        let component = MyComponent::new(domain_object);
        
        // Test initial state
        assert_eq!(component.some_property(), expected_value);
    }
    
    #[test]
    fn test_configuration_methods() {
        let component = MyComponent::new(test_domain_object)
            .config_option(test_value);
            
        // Test configuration was applied
        assert_eq!(component.internal_config, test_value);
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
- ❌ Manual configuration of derived properties (e.g., decimal places)
- ❌ Callback-only APIs without response objects (prefer hybrid approach)
- ❌ Required callbacks (always make them optional)
- ❌ Eager initialization of all components (use lazy initialization)
- ❌ Mixing static and dynamic configuration inappropriately
- ❌ Complex callback chains (keep callbacks simple and focused)

## Example: AmountInput Implementation

The `AmountInput` component exemplifies this pattern:

- **Self-contained**: Manages its own string state and validation
- **Lazy initialization**: Created only when first displayed
- **Dual API**: Both `label()` and `set_label()` methods
- **Hybrid communication**: Returns `AmountInputResponse` with optional callbacks
- **Type-driven**: Decimal places determined by `Amount` object
- **Optional callbacks**: Supports `on_success()` and `on_error()` for immediate response

This pattern has proven effective for creating maintainable, performant UI components that integrate well with egui's immediate mode paradigm while providing excellent developer experience and flexible communication options.
