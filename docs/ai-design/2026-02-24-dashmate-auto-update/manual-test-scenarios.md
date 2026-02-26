# Manual Test Scenarios: Dashmate Auto Update (PR #641)

## Overview

PR #641 adds an "Auto Update" button next to the local network (Regtest) password field in the Network Chooser screen. When clicked, it reads the RPC password from `~/.dashmate/config.json` (using the `local_seed` config name) and auto-fills the password field, saving it to the application config.

---

## Scenario 1: Auto Update button is visible on the Local Network tab

### Preconditions

- Dash Evo Tool is built and running.
- The Network Chooser screen is accessible.

### Steps

1. Open the application.
2. Navigate to the Network Chooser screen.
3. Select the **Local Network** (Regtest) tab.
4. Locate the RPC password text field.

### Expected Results

- The password text field is visible.
- A **"Save"** button is displayed next to the text field.
- An **"Auto Update"** button is displayed next to the Save button.
- Both buttons are clearly labeled and not overlapping.

---

## Scenario 2: Successful auto-fill with dashmate installed and configured

### Preconditions

- Dashmate is installed on the machine.
- `~/.dashmate/config.json` exists and contains a valid `local_seed` configuration with an RPC password at the JSON path: `configs.local_seed.core.rpc.users.dashmate.password`.
- The password field is either empty or contains a different value.

### Steps

1. Open the application and navigate to Network Chooser > Local Network.
2. Note the current value in the password text field.
3. Click the **"Auto Update"** button.

### Expected Results

- The password text field is immediately populated with the password value from `~/.dashmate/config.json`.
- The password is automatically saved to the application config (no need to click Save separately).
- No error messages are displayed.

### Verification

- Close and reopen the application.
- Navigate back to Network Chooser > Local Network.
- Confirm the password field still contains the auto-filled value (persistence check).

---

## Scenario 3: Auto Update when dashmate is not installed

### Preconditions

- Dashmate is **not** installed, meaning `~/.dashmate/config.json` does not exist.
- The password field may contain any value (empty or pre-filled).

### Steps

1. Open the application and navigate to Network Chooser > Local Network.
2. Note the current value in the password text field.
3. Click the **"Auto Update"** button.

### Expected Results

- The password field retains its previous value (unchanged).
- An error is logged (check application logs / terminal output for a message like: `Auto update failed: Failed to read /home/<user>/.dashmate/config.json: No such file or directory`).
- **Note:** Currently there is no visible UI error feedback to the user -- the error only appears in `tracing::error!` logs. Verify whether this is acceptable UX or if a user-visible message should be shown.

---

## Scenario 4: Auto Update when dashmate config exists but has no local_seed entry

### Preconditions

- `~/.dashmate/config.json` exists but does not contain a `local_seed` key under `configs`, or the `local_seed` entry does not have the expected `core.rpc.users.dashmate.password` path.

### Steps

1. Open the application and navigate to Network Chooser > Local Network.
2. Click the **"Auto Update"** button.

### Expected Results

- The password field retains its previous value (unchanged).
- An error is logged: `Auto update failed: Password not found in dashmate config 'local_seed'`.
- No crash or panic occurs.

---

## Scenario 5: Auto Update when dashmate config contains malformed JSON

### Preconditions

- `~/.dashmate/config.json` exists but contains invalid JSON (e.g., a truncated file or syntax error).

### Steps

1. Open the application and navigate to Network Chooser > Local Network.
2. Click the **"Auto Update"** button.

### Expected Results

- The password field retains its previous value (unchanged).
- An error is logged: `Auto update failed: Failed to parse dashmate config: ...`.
- No crash or panic occurs.

---

## Scenario 6: Manual Save button still works independently

### Preconditions

- The application is running and on the Local Network tab.

### Steps

1. Navigate to Network Chooser > Local Network.
2. Clear the password field and type a custom password, e.g., `my_custom_password`.
3. Click the **"Save"** button (not Auto Update).
4. Close and reopen the application.
5. Navigate back to Network Chooser > Local Network.

### Expected Results

- After step 3, the password is saved to the application config.
- After step 5, the password field shows `my_custom_password`, confirming persistence.

---

## Scenario 7: Auto Update overwrites a previously saved manual password

### Preconditions

- Dashmate is installed with a valid `local_seed` config containing a known RPC password (e.g., `dashmate_password_123`).
- A different password was previously saved manually (e.g., `old_manual_password`).

### Steps

1. Navigate to Network Chooser > Local Network.
2. Confirm the field shows `old_manual_password`.
3. Click **"Auto Update"**.

### Expected Results

- The password field now shows `dashmate_password_123`.
- The new password is automatically persisted.
- Closing and reopening the application confirms the dashmate password is retained.

---

## Scenario 8: Auto Update button is not present on non-Local networks

### Preconditions

- The application is running.

### Steps

1. Navigate to the Network Chooser screen.
2. Select the **Mainnet** tab.
3. Look for an "Auto Update" button in the network configuration area.
4. Repeat for **Testnet** and **Devnet** tabs.

### Expected Results

- The "Auto Update" button is **not** displayed on Mainnet, Testnet, or Devnet tabs.
- The button only appears on the Local Network (Regtest) tab where the dashmate password field is shown.

---

## Edge Cases

### E1: Config save failure

- If `Config::load()` or `config.save()` fails after Auto Update reads the password successfully, the password field will be updated in the UI but may not persist across restarts. This mirrors the existing Save button behavior.

### E2: Empty password in dashmate config

- If the dashmate config contains an empty string for the password value (`"password": ""`), the Auto Update should still fill the field with an empty string and save it. Verify the field is cleared and the empty value is persisted.

### E3: Very long password in dashmate config

- If the dashmate config contains an unusually long password string, verify the text field displays it correctly without UI layout issues.

### E4: File permission issues

- If `~/.dashmate/config.json` exists but is not readable by the current user, verify the error is handled gracefully (logged, no crash, password field unchanged).

### E5: Concurrent dashmate config changes

- If the dashmate config is being written to by dashmate while the user clicks Auto Update, the read may fail or return partial data. Verify no crash occurs (the JSON parse error path should handle this).
