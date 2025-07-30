# UI Component Design Pattern

## Vision

Imagine a library of ready-to-use widgets where you, as a developer, simply pick what you need.

Need wallet selection? Grab `WalletChooserWidget`. It handles wallet selection, prompts for passwords when needed, validates user choices, and more.

Need password entry? Use `PasswordWidget`. It manages passwords securely, masks input, validates complexity rules, and zeros memory after use.

All widgets follow the same simple pattern: add 2 fields to your screen struct, lazy-load the widget, then bind it to your data with the `update()` method.

## Quick Start: Using Components

In this section, you will see how to use an existing component.

### 1. Add fields to your screen struct
```rust
struct MyScreen {
    amount: Option<Amount>,           // Domain data
    amount_widget: Option<AmountInput>, // UI component
}
```

### 2. Lazily initialize the component

Inside your screen's `show()` method or simiar:

```rust
let amount_widget = self.amount_widget.get_or_insert_with(|| {
    AmountInput::new(amount_type)
        .with_label("Amount:")
});        
```

### 3. Show component and handle updates

After initialization above, use `update()` to bind your screen's field with the component:

```rust
let response = amount_widget.show(ui);
response.inner.update(&mut self.amount);
```

### 4. Use the domain data
When `self.amount.is_some()`, the user has entered a valid amount. Use it for whatever you need.

---

## Implementation Guidelines: Creating New Components

In this screen, you will see generalized guidelines for creating a new component.

### ✅ Component Structure Checklist
- [ ] Struct with private fields only
- [ ] `new()` constructor taking domain configuration
- [ ] Builder methods (`with_label()`, `with_max_amount()`, `with_hint_text()`, etc.)
- [ ] Response struct with `response`, `changed`, `error_message`, and domain-specific data fields

### ✅ Trait Implementation Checklist
- [ ] Implement `Component` trait with `show()` method
- [ ] Implement `ComponentResponse` for response struct

### ✅ Response Pattern
```rust
pub struct MyComponentResponse {
    pub response: Response,
    pub changed: bool,
    pub error_message: Option<String>,
    // Add any component-specific fields as needed
    pub parsed_data: Option<DomainType>,
}

impl ComponentResponse for MyComponentResponse {
    type DomainType = YourType;
    
    fn has_changed(&self) -> bool { self.changed }
    fn is_valid(&self) -> bool { self.error_message.is_none() }
    fn changed_value(&self) -> &Option<Self::DomainType> { &self.parsed_data }
    fn error_message(&self) -> Option<&str> { self.error_message.as_deref() }
}
```

### ✅ Best Practices
- [ ] Use lazy initialization (`Option<Component>`)
- [ ] Use egui's `add_enabled_ui()` for enabled/disabled state
- [ ] Set data to `None` when input changes but is invalid
- [ ] Provide fluent builder API for configuration
- [ ] Keep internal state private
- [ ] **Be self-contained**: Handle validation, error display, hints, and formatting internally (preferably with configurable error display)
- [ ] **Own your UX**: Component should manage its complete user experience

### ❌ Anti-Patterns to Avoid
- Public mutable fields
- Managing enabled state in component
- Eager initialization
- Not clearing invalid data

See `AmountInput` in `src/ui/components/amount_input.rs` for a complete example.
