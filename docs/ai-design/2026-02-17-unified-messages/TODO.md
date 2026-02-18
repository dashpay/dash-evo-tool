# Unified messages TODO

1. fn display_message right now in many cases only sets some screen-scoped field to indicate error state:
    ```
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if let MessageType::Error = message_type {
            self.transfer_tokens_status = TransferTokensStatus::Error;
        }
    }
    ```

    Check if we have any good way to unify that. It doesn't sound like something that should be in display_message - rather on_message() or message_handler()
