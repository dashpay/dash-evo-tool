# Phase 8: DashPay Screens — Design

## 8.1 DashPay Social/Payments UX Design (Run 87)

### Complete DashPay Functionality Inventory

**13 files, 4 subscreens, 9 distinct screen types, ~85 user actions, 23 backend IPC commands (all implemented)**

### Screen Details

**ProfileScreen (~25 actions):**
- Identity selector, load profile from DB/Platform
- View mode: avatar (async load, center-crop, cached), display name, DPNS username, identity ID, bio
- Click avatar -> popup with larger image + copy URL
- Edit mode: display name (25 char), bio (140 char), avatar URL (500 char, http/https validation)
- Real-time character count with color coding, real-time validation
- Unsaved changes detection with discard confirmation
- Fee estimation, identity balance check, wallet unlock, create vs update profile

**ContactsList (~20 actions):**
- 2 tabs: My Contacts / Requests (with pending count badge)
- Identity selector, search bar, filter dropdown (7 options), sort dropdown (4 options)
- Contact cards: avatar, name (fallback chain), @username, bio, account reference
- Per-contact actions: View Profile, Pay (dev only), Hide/Unhide
- Empty states, DB load on init, Platform fetch on refresh

**ContactRequests (~15 actions):**
- 2 tabs: Incoming / Outgoing
- Incoming: avatar, name, account label, timestamp, Accept/Reject with confirmation + wallet unlock
- MissingEncryptionKey error -> "Add Encryption Key" button
- Outgoing: To name, identity ID, status (Pending), "Cannot be cancelled"
- Name resolution from local DB + async Platform fetch

**AddContactScreen (~10 actions):**
- Identity selector, auto key selection (AUTH, CRITICAL/HIGH)
- Username or Identity ID input with validation
- Relationship Label (optional, 100 char max)
- Structured errors: MissingEncryptionKey/DecryptionKey, InvalidUsername, UsernameResolutionFailed
- Retry button, success screen

**ContactDetailsScreen (~8 actions):**
- Profile section, Send Payment button (dev only)
- Private Contact Info: nickname, note, is_hidden with edit/save/cancel
- Payment History section (per-contact)
- Auto-fetch from Platform

**ContactInfoEditorScreen (~6 actions):**
- Nickname, Note (multiline), Hide checkbox with warning
- Accepted Account Indices with Parse button
- Wallet unlock, Save/Cancel

**ContactProfileViewerScreen (~8 actions):**
- Public profile (avatar, name, ID, message)
- Avatar verification (hash, fingerprint)
- Refresh/Pay buttons, embedded Private Contact Info

**ProfileSearchScreen (~6 actions):**
- DPNS username prefix search, results cards, View Profile / Add Contact actions

**SendPaymentScreen (~8 actions):**
- From identity + wallet balance, To contact, Amount input + Max, Memo (100 char)
- Wallet unlock, success with tx ID

**PaymentHistory (~5 actions):**
- Identity selector, payment records (avatar, direction, name, amount, memo, tx ID, timestamp)

**QRCodeGeneratorScreen (~6 actions):**
- Identity selector, account index (advanced), validity hours (1-720, advanced)
- Wallet unlock, QR image, collapsible text data, copy

**QRScannerScreen (~5 actions):**
- Identity selector, QR data paste, Parse, parsed details, Add Contact

### Backend Status
All 23 IPC commands fully implemented (14 async + 9 direct DB).

### UX Improvements Over egui
- Modern card-based layout, tabbed navigation
- Inline editing with react-hook-form + zod validation
- Toast notifications, debounced search
- Better empty states, confirmation dialogs, wallet unlock as overlay

---

## 8.4 DashPay Screens Functionality Parity Audit (Run 173)

### Summary
**Grade: A-** — All 12 DashPay screens implemented with excellent feature parity. 529 DashPay-specific tests (66 store + 18 DashPayScreen + 60 ContactsList + 34 ContactDetails + 35 ContactProfileViewer + 40 ContactInfoEditor + 45 SendPayment + 33 PaymentHistory + 37 ProfileSearch + 37 AddContact + 32 QRCodeGenerator + 34 QRScanner + 58 ProfileScreen). 4287 total tests pass.

### Screen-by-Screen Parity Analysis

#### ProfileScreen (25/25 actions) — FULL PARITY
- Identity selector with auto-selection of identity with profile
- View mode: avatar display (img tag), display name, DPNS usernames, identity ID with copy, bio
- Click avatar → dialog with larger image display
- Edit mode: display name (25 char max), bio (140 char), avatar URL (500 char, http/https validation)
- Real-time character counters with color coding (green→orange→red thresholds)
- Real-time validation with inline error messages
- Unsaved changes detection with discard ConfirmationDialog
- Profile Guidelines info sheet, Avatar Guidelines info sheet
- Fee estimation display, identity balance check
- Wallet unlock dialog before save
- Create Profile vs Update Profile distinction with different button labels
- Loading/saving states with spinner
- Success screen after save

#### ContactsList (20/20 actions) — FULL PARITY
- Two tabs: My Contacts / Requests (with pending count badge)
- Identity selector in header (shared across tabs via DashPayScreen)
- Search input filtering across username, display name, identity ID
- Filter dropdown: All, With usernames, No usernames, With bio, Recent (7d), Hidden, Visible
- Sort dropdown: Name, Username, Date Added, Account
- Show hidden toggle checkbox
- Contact cards: avatar (img), display name with [Hidden] badge, @username, bio snippet
- Per-contact actions: View Profile (nav), Pay (nav), Hide/Unhide (toggle)
- Empty states: "No Contacts" with Add Contact nav, "No matches" for filtered empty
- DB load on mount, Platform refresh via button
- QR Generator and QR Scanner navigation buttons

#### ContactRequests (14/15 actions) — MINOR GAP
- Two sub-tabs: Incoming / Outgoing
- Incoming cards: avatar placeholder, display name/username/truncated ID, account label, timestamp, Accept/Reject buttons
- Accept flow: confirmation dialog → wallet unlock handled at store level → success
- Reject flow: confirmation dialog → rejection
- Outgoing cards: To name, identity ID, status badge, "Cannot be cancelled" note
- Name resolution: DB-cached display names
- Empty states per tab
- **GAP:** No structured MissingEncryptionKey error → "Add Key" action button. The ContactRequests component shows a plain error banner but doesn't parse structured errors to offer "Add Encryption Key" navigation. The AddContactScreen handles this correctly, but ContactRequests does not (P3 — the accept/reject backend operations also need encryption keys, and the egui version shows an action button to navigate to AddKeyScreen when this error occurs).

#### AddContactScreen (10/10 actions) — FULL PARITY
- Identity selector with auto key selection (AUTH, CRITICAL/HIGH)
- Key selector in Advanced Options
- Username or Identity ID input with validation (empty check, .dash suffix check)
- Relationship Label input (optional, 100 char max with counter)
- Structured error handling: MissingEncryptionKey/DecryptionKey → "Add Key" action buttons
- InvalidUsername → tip text, UsernameResolutionFailed → suggestion
- Retry button for recoverable errors
- Wallet unlock dialog
- Success screen with "Send Another" and "Back to Contacts" and "Back to DashPay" navigation

#### ContactDetailsScreen (8/8 actions) — FULL PARITY
- Profile header: avatar, display name, @username, bio, identity ID with copy
- Send Payment button navigates to `/dashpay/send-payment/$contactId`
- Private Contact Info section: nickname, note display
- View Profile navigation button
- Advanced Edit (account indices) link navigates to ContactInfoEditor
- Auto-fetch contact profile from store

#### ContactInfoEditorScreen (6/6 actions) — FULL PARITY
- Contact identifier display (display name → @username → raw ID fallback)
- Private nickname input with description
- Private note multiline textarea with description
- Hide contact checkbox with amber warning when hidden
- Accepted Account Indices input with Parse button (validates, deduplicates, sorts)
- Info dialog explaining private contact information
- Save flow: local DB + Platform update
- Saving spinner, success/error messages
- Form field disabling during save

#### ContactProfileViewer (7/8 actions) — MINOR GAP
- Public profile: avatar with img display, display name, identity ID with copy, public message
- Avatar Verification section (shows URL only)
- Refresh button to fetch from Platform
- Pay button navigates to send-payment screen
- Embedded private contact info: nickname/note with edit/save/cancel
- **GAP:** Avatar hash and fingerprint display not implemented. However, the egui version also noted "Not stored in contacts table yet" for these fields — the data was never actually populated. This is a non-issue (P3, cosmetic only).

#### ProfileSearchScreen (6/6 actions) — FULL PARITY
- DPNS username prefix search with Enter key trigger
- Search button, Clear button
- Search results cards: username (primary), display name, public message preview (60 char truncate), identity ID
- Per-result action buttons: View Profile (nav), Add Contact (nav with pre-populated ID)
- Loading spinner with "Searching..." label
- "No Users Found" empty state with search tip
- Info popup explaining profile search

#### SendPaymentScreen (8/8 actions) — FULL PARITY
- From identity display (alias → DPNS name → truncated ID)
- Wallet balance display with associated wallet lookup
- Wallet locked warning with unlock button
- To contact display with name resolution
- Amount input with Max button (fills confirmed wallet balance)
- Memo textarea with 100-char max and character counter with color warnings
- Send button with validation (amount > 0, memo length)
- Sending state with spinner
- Success screen with amount/recipient info, "Back to DashPay" / "Send Another Payment" navigation
- Cancel button
- Info popup with payment guidelines

#### PaymentHistoryScreen (5/5 actions) — FULL PARITY
- Identity-aware loading (auto-loads on mount/identity change)
- Refresh button to fetch from Platform
- Payment cards: direction indicators (incoming green ⬇ / outgoing red ⬆)
- Contact name resolution (displayName → username → truncated ID fallback)
- Amount display with +/- color coding
- Memo display in italics
- Transaction ID with copy-to-clipboard button
- Relative timestamp formatting
- Status badge for non-confirmed payments
- Empty state, error banner, loading spinner

#### QRCodeGeneratorScreen (6/6 actions) — FULL PARITY
- Identity selector
- Advanced options toggle (account index, validity hours 1-720)
- Wallet locked detection with warning banner and "Unlock Wallet" button
- Generate QR Code button (disabled when wallet locked)
- QR code display using QRCodeSVG from qrcode.react
- Collapsible QR data text with copy button
- Warning about auto-acceptance
- Info dialog explaining QR codes
- Clear button to reset
- No identities empty state

#### QRScannerScreen (5/5 actions) — FULL PARITY
- Identity selector
- QR data text input (paste)
- Parse QR Code button
- Parsed details display: identity ID, account reference, expiration time
- Expiration detection with red "EXPIRED" text and disabled Add Contact button
- Wallet locked detection with warning banner and "Unlock Wallet" button
- Add Contact button (sends contact request with proof)
- Sending state with spinner
- Success state with form clear
- Info section about QR code scanning
- No identities empty state

### Overall Assessment

**Strengths:**
- All 12 DashPay screens implemented with full or near-full parity
- 529 dedicated DashPay tests with high coverage
- Modern UI with shadcn/ui components, proper dark/light mode support
- Proper state management via Zustand store with event subscription
- All 23 backend IPC commands wired up and used
- Strong validation patterns (character limits, URL validation, structured errors)
- Good UX improvements over egui: inline character counters, confirmation dialogs, wallet unlock overlay
- Wallet-locked detection on QR screens (added in Run 172)

**Minor Gaps Found (2):**
1. **ContactRequests MissingEncryptionKey action button** (P3) — Accept/reject errors don't offer "Add Key" navigation
2. **Avatar hash/fingerprint in ContactProfileViewer** (P3, non-issue) — Data was never populated in egui either

### Fix Sub-tasks

None blocking. The 2 minor gaps are P3 and do not affect core functionality. The MissingEncryptionKey error handling in ContactRequests is a nice-to-have enhancement — the user can still navigate to identity key management manually if they encounter this error.
