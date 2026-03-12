## TaskError variant messages

error-generic = { $message }
error-identity-save = Could not save identity changes. Check available disk space and retry.
error-core-wallet-not-configured = Core wallet not configured for this wallet. Go to the Wallets screen and refresh to auto-detect the Core wallet association.
error-must-retry = { $message }
error-duplicate-identity-public-key = This public key is already registered on the platform. Try a different key.
error-duplicate-identity-public-key-id = This key is already registered on the platform. Try a different key.
error-identity-public-key-contract-bounds-conflict = This key conflicts with an existing key bound to contract { $contractId }. Use a different key or purpose.
error-identity-not-found-locally = This identity could not be found in your local wallet. Try refreshing your identities list.
error-identity-update-transition = Could not build the key update transaction. Please retry.
error-internal-send = Internal update failed. Please retry the operation.

## SDK error messages (from sdk_error_user_message)

error-sdk-broadcast-rejected = The platform rejected this request. Please check your input and try again.
error-sdk-timeout = The operation did not complete within { $seconds } seconds. Please retry — it often succeeds on the second attempt.
error-sdk-stale-node = The server you connected to is behind. Please retry — the app will pick a different server automatically.
error-sdk-dapi-client = Could not connect to the Dash network. Please retry in a few moments.
error-sdk-no-available-addresses = All Dash network servers are temporarily unreachable. Please wait a minute and retry.
error-sdk-cancelled = The operation was cancelled.
error-sdk-already-exists = This object already exists on the platform.
error-sdk-nonce-overflow = This identity has reached its maximum number of operations. Please try again later.
error-sdk-identity-nonce-not-found = The platform has not indexed this identity yet. Please retry in a few moments.
error-sdk-unknown = An unexpected error occurred: { $error }. Please try again later.
