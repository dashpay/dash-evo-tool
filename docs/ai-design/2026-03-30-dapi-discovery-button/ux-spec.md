# UX Specification: Manual DAPI Node Discovery Button

**Date**: 2026-03-30
**Feature**: User-triggered node address discovery in Network Settings
**Status**: Draft

---

## 1. Problem Statement

Dash Evo Tool connects to the Dash Platform network through DAPI nodes (masternodes). Currently, node addresses are configured in a `.env` file and, for Mainnet/Testnet, discovered automatically at startup from a DCG-operated HTTPS endpoint. The app is moving to a manual, user-triggered discovery model where:

- No automatic discovery at startup -- the app always uses addresses from its config
- A button in the Network Settings screen lets users fetch current node addresses on demand
- Users understand they are fetching from a centralized service (DCG), not from the blockchain itself

## 2. Personas and Walkthrough

### Alex (Everyday User)

Alex opens Network Settings because the app said it could not connect to the network. Alex sees the Connection Settings card and notices a message near the empty node address area: "No node addresses configured." Next to it is a clearly labeled button. Alex clicks it, sees a brief spinner, and the field fills with addresses. Alex clicks Save and the app connects. Alex never needs to understand what DAPI means or where the addresses came from.

**Key need**: A single clear action that fixes the "can't connect" problem. No jargon.

### Priya (Power User)

Priya opens Network Settings to refresh her node list after a network upgrade. She sees the "Fetch Node List" button and the info icon next to it. She hovers the info icon and reads that addresses come from a DCG-operated service. She clicks the button. A confirmation dialog appears because she already has addresses configured -- it tells her the current count and the count about to be fetched. She confirms, reviews the new addresses in the text field, and saves.

**Key need**: Transparency about what is happening, confirmation before overwriting, ability to review and edit the result.

### Jordan (Platform Developer)

Jordan is on a Devnet tab. The Fetch Node List button is not visible -- Jordan knows devnet addresses must be entered manually. On Testnet, Jordan uses the button occasionally but is just as likely to paste addresses directly. Jordan appreciates that the button does not auto-save -- it populates the field and lets Jordan edit before committing.

**Key need**: Button stays out of the way on devnets. Does not auto-save; just populates.

## 3. Design

### 3.1 Placement

The discovery button lives in a new **"Node Addresses"** section within the existing **Connection Settings** card, placed immediately below the Network selector row (and SPV warning, if shown) and above the Core RPC Password section. This section is visible for **all networks** but the button is only available for Mainnet and Testnet.

```
Connection Settings Card
+-------------------------------------------------------+
| Connection Type: [SPV Client v]     (dev mode only)    |
| Network:         [Mainnet v]                           |
|                                                        |
| --- Node Addresses ---                                 |
| [multiline text field with current DAPI addresses]     |
| [Fetch Node List]  (i)                                 |
|   ^secondary btn    ^info icon with tooltip            |
|                                                        |
| --- Core RPC Password --- (RPC mode only)              |
| [password field] [Save]                                |
+-------------------------------------------------------+
```

**Layout details:**
- Section label: "Node Addresses" -- rendered as a bold subheading (`RichText::new(...).strong().color(text_primary)`)
- Text field: multiline `TextEdit` showing the current `dapi_addresses` value (comma-separated URLs). Editable. Approximately 3-4 lines tall. Uses `styled_text_edit_multiline()` with standard input strokes.
- Below the text field, a horizontal row containing the button and info icon
- Spacing follows existing patterns: `add_space(12.0)` before section, `add_space(8.0)` between label and field, `add_space(8.0)` between field and button row

### 3.2 Button Design

| Property | Value |
|----------|-------|
| **Label** | "Fetch Node List" |
| **Variant** | Secondary (outlined, `StyledButton` `ButtonVariant::Secondary`) |
| **Size** | Medium |
| **Icon** | None (egui icon support is limited; keep it text-only) |
| **Min width** | 160px |

**Rationale for "Fetch Node List"**:
- Avoids "DAPI" jargon -- Alex does not know what DAPI is
- "Fetch" communicates a network operation (important for trust/timing expectations)
- "Node List" is more concrete than "Discover Nodes" -- it describes what you get
- Secondary variant ensures it does not compete visually with the primary "Save" button in the password section

**Visibility rules:**
- **Mainnet, Testnet**: Button visible and enabled
- **Devnet, Regtest**: Button hidden. In its place, a small caption: "Enter node addresses manually for this network."

### 3.3 Info Icon and Trust Disclosure

To the right of the button, render a small info label `(i)` with an `info_tooltip()` (uses the Help cursor per UX design patterns).

**Tooltip text:**

> Fetches the current list of available nodes from a service operated by Dash Core Group (DCG). This is a convenience service over HTTPS -- it does not access the blockchain directly. Platform proofs are verified independently, so incorrect node addresses cannot forge data, but they could prevent the app from connecting.

**Rationale**: Progressive disclosure. Alex never reads it. Priya hovers once, understands the trust model, and is satisfied. Jordan already knows but appreciates the precision.

### 3.4 Empty State

When the text field is empty AND the current network is Mainnet or Testnet, show a hint message below the text field (above the button row):

```
[text field -- empty, showing placeholder "No node addresses configured"]

  Use "Fetch Node List" to get the current addresses, or enter them manually.

[Fetch Node List]  (i)
```

The hint text is rendered in `DashColors::text_secondary(dark_mode)`, `Typography::SCALE_SM`, italics. It disappears once the field has content.

For Devnet/Regtest with an empty field:

```
[text field -- empty, showing placeholder "No node addresses configured"]

  Enter node addresses for this network (comma-separated URLs).
```

### 3.5 Loading State

When the user clicks "Fetch Node List":

1. Button text changes to "Fetching..." with a spinner (`ui.spinner()`) to the left of the button text
2. Button is disabled (prevents double-submit per UX patterns)
3. Text field remains visible and read-only during the fetch (not disabled -- just non-editable, so the user can see existing content)
4. Expected duration is 1-10 seconds per the discovery module's 10-second timeout

**Implementation note**: This should be dispatched as a `BackendTask` to avoid blocking the UI thread. The screen stores a `discovery_in_progress: bool` flag. The discovery result arrives through the standard `display_task_result()` path.

### 3.6 Success State (No Existing Addresses)

When the field was empty before the fetch:

1. Text field is populated with the fetched addresses (comma-separated)
2. Button returns to default state
3. A success banner appears: "Found {count} node addresses. Review them below and save your settings."
4. The field is **not** auto-saved -- the user must click the existing Save mechanism (or the addresses are saved when the network config is saved)

**Important**: The fetched addresses populate the field but do not persist until the user explicitly saves. This gives all personas a chance to review and edit.

### 3.7 Success State (Existing Addresses -- Confirmation Dialog)

When the field already contains addresses and the user clicks "Fetch Node List":

1. A `ConfirmationDialog` appears before the fetch begins
2. Dialog content:

```
+---------------------------------------------------+
|  Update Node Addresses?                            |
|                                                    |
|  This will replace your current node addresses     |
|  with a fresh list fetched from the Dash network   |
|  service.                                          |
|                                                    |
|  You currently have {N} addresses configured.      |
|  You can review and edit the new list before       |
|  saving.                                           |
|                                                    |
|            [Cancel]          [Fetch]               |
+---------------------------------------------------+
```

| Dialog property | Value |
|-----------------|-------|
| Title | "Update Node Addresses?" |
| Confirm label | "Fetch" |
| Cancel label | "Cancel" |
| Danger mode | No (this is not destructive -- old addresses are only replaced in the field, not saved) |
| Escape/X | Cancels |

**After confirmation**, the fetch proceeds and the field is populated with new addresses. The old addresses are replaced in the field but not persisted until Save.

### 3.8 Error State

When the fetch fails:

1. Button returns to default state ("Fetch Node List", enabled)
2. An error banner appears via `MessageBanner::set_global()`:
   - **Timeout**: "Node list fetch timed out. Check your internet connection and try again."
   - **Network error**: "Could not fetch the node list. Check your internet connection and try again."
   - **No results**: "No available nodes were found. The network may be temporarily unavailable -- try again later."
   - **Other**: "Could not fetch the node list. Try again, or enter node addresses manually."
3. Existing addresses in the field are **not** modified on error

These messages align with the existing `DapiDiscoveryError` variants in `src/dapi_discovery.rs`.

### 3.9 Interaction with Save

The node addresses field needs a Save mechanism. Two options:

**Recommended approach**: Add the node addresses field to the existing config save flow. When the Core RPC Password "Save" button is clicked (or a new dedicated "Save" is added for this section), persist the current contents of the node addresses field to the `.env` config file using `Config::update_config_for_network()`.

If the Node Addresses section is above the Core RPC Password section, adding a small "Save" button on the same row as the Fetch button keeps the interaction local:

```
[Fetch Node List]  (i)                    [Save]
```

The Save button here:
- Uses `StyledButton` Secondary variant, same size as Fetch
- Saves the current text field content to the config file
- Shows success/error via `MessageBanner`
- Is always enabled (even if the field has not changed -- simpler, and egui does not trivially track dirty state)

### 3.10 State Diagram

```
                    +------------------+
                    |   Default State  |
                    | Field: current   |
                    | Button: enabled  |
                    +--------+---------+
                             |
                    User clicks "Fetch Node List"
                             |
                    +--------v---------+
              yes   | Field has        |  no
           +--------+ existing addrs?  +--------+
           |        +------------------+        |
           v                                    v
  +--------+---------+              +-----------+--------+
  | Confirmation     |   Cancel     | Fetching State     |
  | Dialog shown     +---+         | Button: disabled   |
  +--------+---------+   |         | Spinner shown      |
           |             |         +---------+----------+
       Confirm           |                   |
           |             |          +--------v---------+
           v             |     yes  |   Fetch result?  |  no
  +--------+---------+   |   +-----+                  +-----+
  | Fetching State   |   |   |     +------------------+     |
  | Button: disabled |   |   v                              v
  | Spinner shown    |   | +-+-------------+    +-----------+-+
  +--------+---------+   | | Success       |    | Error       |
           |             | | Field updated |    | Field stays |
           |             | | Banner: info  |    | Banner: err |
  +--------v---------+   | +------+--------+    +------+------+
  |   Fetch result?  |   |        |                    |
  +--+------------+--+   |        v                    v
  yes             no     |   +----+--------------------+----+
   |               |     |   |        Default State         |
   v               v     +-->|   (user can now Save)        |
 Success        Error        +------------------------------+
```

## 4. Component Specification

### 4.1 Node Address Text Field

```
Component: NodeAddressField
Purpose: Display and edit the comma-separated list of DAPI node URLs
Type: multiline TextEdit (styled_text_edit_multiline)
States:
  - default: editable, shows current addresses
  - empty: shows placeholder "No node addresses configured"
  - read-only-during-fetch: text visible but not editable
Responsive: full available width, 3-4 lines height (approximately 80px)
Accessibility:
  - Tab-focusable
  - Placeholder text visible when empty
```

### 4.2 Fetch Node List Button

```
Component: StyledButton (Secondary variant)
Label: "Fetch Node List"
States:
  - default: outlined secondary button, enabled
  - hover: pointing hand cursor (automatic from StyledButton)
  - loading: text changes to "Fetching...", disabled, spinner adjacent
  - hidden: on Devnet/Regtest networks
Min width: 160px
Accessibility:
  - Tab-focusable
  - disabled_tooltip when loading: "Fetching node addresses..."
```

### 4.3 Info Icon

```
Component: label "(i)" with info_tooltip()
Purpose: Trust disclosure for the discovery service
States:
  - default: subtle text in text_secondary color
  - hover: Help cursor, tooltip shown
Accessibility:
  - info_tooltip provides the help cursor automatically
```

### 4.4 Save Button (Node Addresses)

```
Component: StyledButton (Secondary variant)
Label: "Save"
Purpose: Persist node addresses to config file
States:
  - default: enabled
  - hover: pointing hand cursor
Placement: right-aligned on the button row
```

### 4.5 Confirmation Dialog

```
Component: ConfirmationDialog (existing pattern)
Title: "Update Node Addresses?"
Body: explains what will happen, shows current address count
Confirm label: "Fetch"
Cancel label: "Cancel"
Danger mode: false
Trigger: clicking Fetch when field already has content
```

## 5. Screen State Additions

New fields on `NetworkChooserScreen`:

```rust
/// Current text in the node addresses field per network
node_addresses_text: HashMap<Network, String>,

/// Whether a discovery fetch is in progress
discovery_in_progress: bool,

/// Confirmation dialog for overwriting existing addresses
discovery_confirm_dialog: Option<ConfirmationDialog>,
```

On construction, `node_addresses_text` is populated from `Config::load_from()` for each network's `dapi_addresses` field.

## 6. Backend Task

A new `SystemTask` variant handles the async discovery:

```rust
SystemTask::DiscoverDapiNodes { network: Network }
```

This calls the existing `try_discover_nodes()` from `src/dapi_discovery.rs` (the async variant, not the sync one with fallback). The result is returned as a new `BackendTaskSuccessResult` variant:

```rust
BackendTaskSuccessResult::DapiNodesDiscovered {
    network: Network,
    addresses: Vec<String>,
}
```

The screen's `display_task_result()` handler populates `node_addresses_text[network]` with the comma-separated result and shows the success banner.

## 7. Accessibility

| Requirement | Implementation |
|-------------|----------------|
| Keyboard navigation | All elements (text field, buttons, info icon) are Tab-focusable in layout order |
| Focus indicator | Standard egui focus ring (BORDER_WIDTH_THICK per UX patterns) |
| Tooltips | info_tooltip on (i) icon, disabled_tooltip on button during fetch |
| Screen readers | egui has limited a11y; no additional ARIA annotations possible |
| Color contrast | All text meets WCAG AA (inherited from DashColors theme system) |
| Click targets | Buttons use StyledButton which meets WCAG AA minimum targets |

## 8. Responsive Behavior

The Node Addresses section uses `ui.available_width()` for the text field (full width). The button row is horizontal with the Fetch button left-aligned and Save button right-aligned. On narrow windows, buttons wrap naturally via egui's horizontal layout (they will stack if space is insufficient, which is acceptable for this secondary feature).

## 9. Edge Cases

| Case | Behavior |
|------|----------|
| User clicks Fetch, then switches network tab during fetch | The result is tagged with the network it was fetched for. If the user has switched networks, the result populates `node_addresses_text[original_network]` silently. The banner still appears. |
| Fetch returns identical addresses to what was already configured | Treat as success. Field is updated (same content). Banner shows the count. |
| User edits the field manually, then clicks Fetch | Confirmation dialog appears (field has content). Fetch replaces the manual edits in the field. |
| User clicks Fetch, gets results, then clicks Fetch again without saving | Confirmation dialog appears (field has content from the first fetch). Second fetch replaces. |
| Config file is read-only or missing | Save button shows error banner: "Could not save settings. Check that the application folder is writable and retry." (matches existing `ConfigError::SaveError` pattern). |
| Discovery returns hundreds of addresses | All addresses populate the field. The multiline TextEdit scrolls. User can edit to trim if desired. |

## 10. What This Spec Does NOT Cover

- Automatic migration of old hardcoded addresses (handled separately in `config.rs`)
- Changes to the startup discovery flow (separate task)
- The `.env` file format or config parsing (existing code in `Config`)
- MCP/CLI exposure of discovery (out of scope for UI spec)

## 11. Implementation Checklist

- [ ] Add `node_addresses_text: HashMap<Network, String>` and related fields to `NetworkChooserScreen`
- [ ] Render the Node Addresses section in `render_network_table()` between Network selector and Core RPC Password
- [ ] Implement `StyledButton::secondary()` variant usage for Fetch and Save buttons
- [ ] Add `SystemTask::DiscoverDapiNodes` backend task variant
- [ ] Add `BackendTaskSuccessResult::DapiNodesDiscovered` result variant
- [ ] Wire `display_task_result()` to populate the text field on success
- [ ] Add `ConfirmationDialog` for overwrite confirmation
- [ ] Add Save button that persists `node_addresses_text` to config
- [ ] Hide Fetch button and show manual-entry hint on Devnet/Regtest
- [ ] Show empty-state guidance when no addresses are configured
- [ ] Test light and dark themes
- [ ] Verify keyboard navigation (Tab through field, Fetch, Save)
