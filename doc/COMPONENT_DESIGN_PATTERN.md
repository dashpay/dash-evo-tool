# UI Component Design Pattern

## Vision

Imagine a library of ready-to-use widgets where you, as a developer, simply pick what you need.

Need wallet selection? Grab `WalletChooserWidget`. It handles wallet selection, prompts for passwords when needed, validates user choices, and more.

Need password entry? Use `PasswordWidget`. It manages passwords securely, masks input, validates complexity rules, and zeros memory after use.

All widgets follow the same simple pattern: add 2 fields to your screen struct, lazy-load the widget, then bind it to your data with the `update()` method.

## Quick Start: Using Components

### 1. Add fields to your screen struct
```rust
struct MyScreen {
    amount: Option<Amount>,           // Domain data
    amount_widget: Option<AmountInput>, // UI component
}
```

### 2. Lazily initialize the component
```rust
fn show(&mut self, ui: &mut Ui) {
    let amount_widget = self.amount_widget.get_or_insert_with(|| {
        AmountInput::new(amount_type)
            .with_label("Amount:")
    });
}
```

### 3. Show component and handle updates
```rust
let response = amount_widget.show(ui);
response.inner.update(&mut self.amount);
```

### 4. Use the domain data
When `self.amount.is_some()`, the user has entered a valid amount. Use it for whatever you need.

---

## Implementation Guidelines: Creating New Components

### ✅ Component Structure Checklist
- [ ] Struct with private fields only
- [ ] `new()` constructor taking domain configuration
- [ ] Builder methods (`with_label()`, `with_max_value()`, etc.)
- [ ] Response struct with `changed`, `error_message`, `parsed_data` fields

### ✅ Trait Implementation Checklist
- [ ] Implement `Component` trait with `show()` method
- [ ] Implement `ComponentResponse` for response struct
- [ ] Add `UpdatableComponent` if managing `Option<T>` data

### ✅ Response Pattern
```rust
pub struct MyComponentResponse {
    pub response: Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub parsed_data: Option<DomainType>,
}

impl ComponentResponse for MyComponentResponse {
    type DomainType = YourType;
    
    fn has_changed(&self) -> bool { self.changed }
    fn is_valid(&self) -> bool { self.error_message.is_none() }
    fn changed(&self) -> Option<Self::DomainType> { self.parsed_data.clone() }
    fn error_message(&self) -> Option<&str> { self.error_message.as_deref() }
}
```

### ✅ Best Practices
- [ ] Use lazy initialization (`Option<Component>`)
- [ ] Use egui's `add_enabled_ui()` for enabled/disabled state
- [ ] Clear invalid data when input changes but is invalid
- [ ] Provide fluent builder API for configuration
- [ ] Keep internal state private
- [ ] **Be self-contained**: Handle validation, error display, hints, and formatting internally
- [ ] **Own your UX**: Component should manage its complete user experience

### ❌ Anti-Patterns to Avoid
- Public mutable fields
- Managing enabled state in component
- Eager initialization
- Not clearing invalid data

See `AmountInput` in `src/ui/components/amount_input.rs` for a complete example.
