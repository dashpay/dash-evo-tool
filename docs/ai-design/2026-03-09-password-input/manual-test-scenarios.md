# Manual Test Scenarios: Secret Type + PasswordInput Component

**Date**: 2026-03-09
**Feature**: Password masking with hold-to-reveal eye icon (Issues #705, #707)
**Tester prerequisites**: A running Dash Evo Tool instance with at least one HD wallet and one single-key wallet loaded. Access to Testnet or a local Devnet. A known-valid WIF private key and hex private key for identity import tests.

---

## Group 1: PasswordInput Core Behavior

### TC-1.1 Default masking on password fields

**Preconditions**: Application is running. Navigate to any screen with a password field (e.g., Wallets > click a locked HD wallet to trigger unlock popup).

**Steps**:
1. Observe the password input field before typing anything.
2. Type the characters `testpass123`.
3. Observe the field contents while typing and after.

**Expected results**:
- Step 1: The field is empty and shows hint text (e.g., "Enter password") in a muted color. An eye icon with a diagonal slash is visible inside the field, right-aligned.
- Step 2-3: Each typed character is immediately displayed as a bullet/dot character. The plaintext is never visible at any point during typing. The eye icon remains in the "closed" state (circle with slash).

---

### TC-1.2 Hold-to-reveal shows plaintext while held

**Preconditions**: A password field contains typed text (e.g., continue from TC-1.1).

**Steps**:
1. Move the mouse pointer over the eye icon inside the password field.
2. Observe the cursor and tooltip.
3. Press and hold the primary mouse button on the eye icon.
4. While holding, observe the field contents and the eye icon.
5. Release the mouse button.
6. Observe the field contents and eye icon after release.

**Expected results**:
- Step 1-2: The cursor changes to a pointing hand. The eye icon color changes to Dash blue (#008de4). A tooltip "Hold to reveal" appears.
- Step 3-4: The field contents switch from dots to plaintext (the actual typed password is visible). The eye icon changes to "open" state (circle with filled pupil, no slash). The icon remains Dash blue.
- Step 5-6: The field contents immediately switch back to dots/bullets. The eye icon returns to "closed" state (circle with slash). The color returns to muted gray (assuming the pointer moved off the icon).

---

### TC-1.3 Dragging pointer off eye icon re-masks immediately

**Preconditions**: A password field contains typed text.

**Steps**:
1. Press and hold the primary mouse button on the eye icon (text reveals).
2. While still holding the mouse button, drag the pointer outside the eye icon area (e.g., move left into the text area or away from the field entirely).
3. Observe the field contents while the pointer is outside the icon area but the button is still held.
4. Release the mouse button.

**Expected results**:
- Step 1: Text is revealed in plaintext while pointer is on the icon.
- Step 2-3: As soon as the pointer leaves the eye icon rectangle, the text re-masks to dots immediately, even though the mouse button is still held down. The eye icon returns to closed state.
- Step 4: Field remains masked. No change on button release.

---

### TC-1.4 Eye icon non-functional on empty field

**Preconditions**: Navigate to any screen with a PasswordInput field. The field is empty.

**Steps**:
1. Observe that the eye icon is present even when the field is empty.
2. Press and hold the eye icon.
3. Release.

**Expected results**:
- Step 1: Eye icon is displayed in muted color. Hint text is visible.
- Step 2: No crash or error. The field visually switches to "revealing" mode (open eye icon) but there is nothing to show since the field is empty.
- Step 3: Returns to closed/masked state.

---

### TC-1.5 Error message display below input

**Preconditions**: Navigate to a wallet unlock popup (click a locked wallet, or attempt an operation requiring unlock). Type an incorrect password.

**Steps**:
1. Enter an incorrect password in the unlock popup.
2. Click "Unlock" (or equivalent submit action).
3. Observe the area below the password input after the backend returns an error.
4. Verify the eye icon still functions while an error is displayed.

**Expected results**:
- Step 2-3: An error message (e.g., "Incorrect password") appears below the input field in a warning/error color. The password field retains its content (dots visible).
- Step 4: Pressing and holding the eye icon still reveals the typed (incorrect) password. The error message remains visible during reveal. The eye icon color and behavior are unchanged by the error state.

---

## Group 2: Wallet Unlock Flow

### TC-2.1 HD wallet unlock popup uses PasswordInput

**Preconditions**: At least one locked HD wallet exists.

**Steps**:
1. Navigate to the Wallets screen.
2. Click on a locked HD wallet to trigger any operation that requires unlock (e.g., viewing private keys, sending funds).
3. Observe the unlock popup that appears.
4. Verify the password field is a PasswordInput (masked with eye icon).
5. Type the correct wallet password.
6. Hold the eye icon to verify the password is correct.
7. Release the eye icon.
8. Click "Unlock".

**Expected results**:
- Step 3-4: The popup contains a password field with dot masking and an eye icon. There is no separate "Show password" checkbox (the old pattern has been removed).
- Step 5: Characters appear as dots.
- Step 6: Plaintext password is visible while the eye is held.
- Step 7: Password re-masks to dots.
- Step 8: Wallet unlocks successfully. The popup closes.

---

### TC-2.2 Single-key wallet password entry

**Preconditions**: A single-key wallet exists on the Wallets screen.

**Steps**:
1. Navigate to the Wallets screen.
2. Locate the single-key wallet section.
3. Observe the password input field for the single-key wallet.
4. Type a password and verify it is masked.
5. Use the hold-to-reveal eye icon to verify the password.

**Expected results**:
- Step 3: The password field uses PasswordInput with dot masking and an eye icon. No "Show password" checkbox is present.
- Step 4-5: Masking and hold-to-reveal behave identically to TC-1.2.

---

## Group 3: Previously Unmasked Fields (Security Fixes)

### TC-3.1 Add New Wallet screen masks password

**Preconditions**: Application is running.

**Steps**:
1. Navigate to the Add New Wallet screen (e.g., Wallets > Add Wallet).
2. Locate the password field.
3. Type a password.
4. Observe whether the field is masked.
5. Use the eye icon to verify hold-to-reveal works.

**Expected results**:
- Step 2: The field has an eye icon and shows hint text (e.g., "Optional password").
- Step 3-4: The password is displayed as dots, NOT plaintext. (Previously this field was unmasked -- this is a security fix.)
- Step 5: Hold-to-reveal works as specified in TC-1.2.

---

### TC-3.2 Import Mnemonic screen masks password and private key

**Preconditions**: Application is running.

**Steps**:
1. Navigate to the Import Mnemonic / Import Wallet screen.
2. Locate the password field.
3. Type a password and verify it is masked with dots.
4. Locate any private key field (if applicable on this screen).
5. Paste or type a private key value and verify it is masked.
6. Use the eye icon on each field independently to verify reveal works.

**Expected results**:
- Step 2-3: Password field is masked. (Previously unmasked -- security fix.)
- Step 4-5: Private key field is masked with dots and uses a monospace font. (Previously unmasked -- security fix.)
- Step 6: Each field has its own independent eye icon. Holding one reveals only that field, not the other.

---

### TC-3.3 Add Existing Identity screen masks all private key fields

**Preconditions**: Application is running.

**Steps**:
1. Navigate to the Add Existing Identity screen (Identities section).
2. Locate the Voting Private Key, Owner Private Key, and Payout Address Private Key fields.
3. Type or paste a WIF key into the Voting Private Key field.
4. Observe that it is masked.
5. Use the eye icon to reveal and verify the key text.
6. Scroll down to the dynamic key list section.
7. Add 2-3 additional private key entries using the "Add" button.
8. Type values into each dynamic key field.
9. Verify each dynamic key field is independently masked with its own eye icon.

**Expected results**:
- Step 2: All three static key fields use PasswordInput with eye icons. (Previously unmasked -- security fix.)
- Step 3-4: The typed/pasted WIF key is shown as dots.
- Step 5: Holding the eye reveals the key in monospace font.
- Step 7-8: Each newly added key field is a PasswordInput with its own eye icon.
- Step 9: Holding the eye on one dynamic key field reveals only that field. Other fields remain masked.

---

### TC-3.4 Network Chooser screen masks RPC password

**Preconditions**: Application is running with access to network settings.

**Steps**:
1. Navigate to the Network Chooser / Network Configuration screen.
2. Locate the Dashmate / Core RPC password field.
3. Type or paste an RPC password.
4. Observe masking behavior.
5. Use the eye icon to verify the password.

**Expected results**:
- Step 2: The RPC password field uses PasswordInput with an eye icon and hint text "Core RPC password". (Previously unmasked -- security fix.)
- Step 3-4: The password is displayed as dots.
- Step 5: Hold-to-reveal works correctly.

---

## Group 4: Private Key Display (WIF in Popups and Info Screens)

### TC-4.1 Private key dialog in Wallets screen

**Preconditions**: An unlocked HD wallet with at least one derived address.

**Steps**:
1. Navigate to the Wallets screen.
2. Select an unlocked HD wallet.
3. Trigger the "Show Private Key" action for a specific address (e.g., via context menu or a key icon button).
4. Observe the private key dialog/popup that appears.
5. Look for the WIF private key display.
6. Close the dialog.

**Expected results**:
- Step 4-5: The WIF private key is displayed. If there is a copy button, it copies the WIF value. The display uses the Secret type internally (no sensitive data in Debug output if logged).
- Step 6: After closing the dialog, the WIF value should be cleared from the dialog state. Reopening the dialog for the same address should re-derive the key, not reuse stale data.

---

### TC-4.2 Key Info screen private key input and WIF display

**Preconditions**: An identity with at least one key exists.

**Steps**:
1. Navigate to the Key Info screen for an identity key.
2. Locate the private key input field.
3. Verify it uses PasswordInput with masking and an eye icon.
4. Type or paste a valid private key (hex format, 64 characters).
5. Hold the eye icon to verify.
6. If a WIF display is shown after key validation, verify it uses monospace font and the Secret type.

**Expected results**:
- Step 2-3: Private key input is a PasswordInput with monospace font, masked by default.
- Step 4: Characters are shown as dots.
- Step 5: Plaintext is revealed in monospace while holding the eye icon.
- Step 6: WIF values are displayed using the Secret wrapper. If a copy button is present, it copies the WIF correctly.

---

### TC-4.3 Add Key screen private key input

**Preconditions**: An identity exists where keys can be added.

**Steps**:
1. Navigate to the Add Key screen for an identity.
2. Locate the private key input field.
3. Type or paste a WIF private key (51-52 characters).
4. Verify masking and hold-to-reveal.

**Expected results**:
- Step 2: The field uses PasswordInput with eye icon and monospace font.
- Step 3-4: Key is masked as dots. Hold-to-reveal shows the WIF in monospace. Release re-masks.

---

### TC-4.4 Asset Lock screens password masking

**Preconditions**: Navigate to an asset lock detail screen or create asset lock screen that requires wallet unlock.

**Steps**:
1. Trigger the wallet password field on the Asset Lock Detail screen or Create Asset Lock screen.
2. Type a password.
3. Verify masking with eye icon.
4. On the Asset Lock Detail screen, if a private key WIF is displayed, verify it uses the Secret type.

**Expected results**:
- Step 2-3: Password field is masked with hold-to-reveal eye icon. No "Show password" checkbox.
- Step 4: WIF display, if present, is wrapped in Secret (monospace, proper handling).

---

## Group 5: Secret Type Behavior

### TC-5.1 Debug output does not leak secrets

**Preconditions**: Application is running with logging enabled (debug level).

**Steps**:
1. Perform any operation that involves a Secret value (e.g., unlock a wallet, view a private key).
2. Check the application log output (stdout/stderr or log file).
3. Search for the actual password or key text in the logs.
4. Search for "Secret(***)" in the logs to confirm redaction is present.

**Expected results**:
- Step 2-3: The actual secret value (password text, WIF key, hex key) does NOT appear anywhere in the log output.
- Step 4: If the Secret type's Debug representation is logged, it shows `Secret(***)` instead of the actual value.

---

### TC-5.2 Password field clears after failed unlock attempt

**Preconditions**: A locked wallet exists.

**Steps**:
1. Trigger the unlock popup for a locked wallet.
2. Enter an incorrect password.
3. Submit the unlock attempt.
4. Observe whether the password field is cleared after the error is shown, or if the user can retry with the same text.
5. If the field is not auto-cleared, close the popup and reopen it.
6. Verify the field is empty on reopen.

**Expected results**:
- Step 3-4: The behavior depends on the screen implementation. The field may retain text for retry or may be cleared. Either is acceptable as long as the error message is shown.
- Step 5-6: When the popup is closed and reopened, the password field MUST be empty. The Secret backing the previous input should have been zeroized.

---

### TC-5.3 Private key fields clear on screen navigation

**Preconditions**: Navigate to the Add Existing Identity screen.

**Steps**:
1. Type private keys into the Voting, Owner, and Payout Address fields.
2. Navigate away from the screen (e.g., go to Wallets, then back).
3. Check whether the private key fields are empty or still contain the previous values.

**Expected results**:
- Step 3: Behavior depends on whether the screen instance is recreated on navigation. If the screen persists (root screen), the fields may retain values. If the screen is a modal that is popped from the stack, the fields and their Secret-wrapped values should be dropped and zeroized.

---

## Group 6: Theme and Visual Consistency

### TC-6.1 Eye icon appearance in light mode

**Preconditions**: Application is set to light mode.

**Steps**:
1. Navigate to any screen with a PasswordInput field.
2. Observe the eye icon color in default state (not hovered).
3. Hover over the eye icon and observe color change.
4. Hold the eye icon and observe the "open eye" state.

**Expected results**:
- Step 2: Eye icon is drawn in a muted/secondary text color (visible but not prominent).
- Step 3: Eye icon color changes to Dash blue (#008de4).
- Step 4: Open eye (circle + filled pupil) in Dash blue. Text is revealed in plaintext.

---

### TC-6.2 Eye icon appearance in dark mode

**Preconditions**: Application is set to dark mode.

**Steps**:
1. Repeat all steps from TC-6.1 in dark mode.

**Expected results**:
- Same behavior as TC-6.1 but with dark mode secondary text color for the default icon state. Dash blue hover/active color remains the same. The icon must have sufficient contrast against the dark input background.

---

### TC-6.3 Monospace font for private key fields

**Preconditions**: Navigate to any screen with a private key PasswordInput (e.g., Add Existing Identity, Key Info).

**Steps**:
1. Type a hex string or WIF key into the private key field.
2. Hold the eye icon to reveal the text.
3. Compare the font to a regular password field (e.g., wallet unlock).

**Expected results**:
- Step 2-3: The revealed text in a private key field uses a monospace font. A regular password field uses the default proportional font. The difference should be visually obvious (equal character widths in monospace).

---

## Group 7: Edge Cases

### TC-7.1 Very long password handling

**Preconditions**: Any PasswordInput field.

**Steps**:
1. Type or paste a very long string (200+ characters) into the password field.
2. Observe that the field scrolls or truncates visually but accepts all characters.
3. Hold the eye icon to reveal.
4. Verify all characters are present.

**Expected results**:
- Step 2: The field accepts the full string. Dots may overflow the visible area (horizontal scrolling within the field).
- Step 3-4: Revealing shows the full plaintext (may require scrolling within the field). No truncation or data loss.

---

### TC-7.2 Paste into masked field

**Preconditions**: Copy a known string to the clipboard (e.g., "pastedSecret123").

**Steps**:
1. Click into any PasswordInput field to focus it.
2. Paste from clipboard (Ctrl+V / Cmd+V).
3. Observe that dots appear (not plaintext).
4. Hold the eye to reveal and confirm the pasted value matches the clipboard content.

**Expected results**:
- Step 2-3: Pasted text is immediately masked as dots.
- Step 4: The revealed text matches the original clipboard content exactly.

---

### TC-7.3 Multiple PasswordInput fields on one screen

**Preconditions**: Navigate to the Add Existing Identity screen (which has 3+ PasswordInput fields plus dynamic key list).

**Steps**:
1. Type different values into each of the three static key fields (Voting, Owner, Payout).
2. Add two dynamic key entries and type values into them.
3. Hold the eye icon on the Voting key field -- verify only that field reveals.
4. Release. Hold the eye on a dynamic key field -- verify only that field reveals.
5. Verify all other fields remain masked during each reveal.

**Expected results**:
- Step 3: Only the Voting key field shows plaintext. Owner, Payout, and dynamic fields show dots.
- Step 4: Only the targeted dynamic field reveals. All others remain masked.
- Step 5: Each eye icon operates independently. No cross-field reveal.

---

### TC-7.4 Rapid click on eye icon (not hold)

**Preconditions**: A password field with text entered.

**Steps**:
1. Quickly click (press and release immediately) the eye icon.
2. Observe whether the password flashes briefly or stays masked.

**Expected results**:
- Step 1-2: A very brief click may cause a single-frame flash of plaintext (since the reveal state is based on pointer-down). This is acceptable -- the text re-masks immediately on release. The field should NOT toggle to a persistent revealed state. After the click, the field must be fully masked.
