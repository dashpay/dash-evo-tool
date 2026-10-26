# Send Screen Redesign — UX Spec

Requirement IDs (`SR-xx`, `D-xx`) refer to [01-requirements.md](01-requirements.md). Wireframes: [03-wireframes.md](03-wireframes.md).

## 1. Principles

1. **Who first, then how.** The recipient decides which routes are possible. The app picks the balance unless the user overrides it.
2. **Auto explains itself.** Every automatic choice comes with a plain sentence saying what it chose and why.
3. **Nothing typed is ever lost.** Changing one section never clears the other (SR-16).
4. **One place for money facts.** The fee, the total and what the recipient gets appear only in the Summary card.
5. **Disable with a reason, never refuse late.** An impossible choice is greyed out with a tooltip; it is not an error after Send.

## 2. Anatomy (top → bottom)

```
Heading            "Send Dash" (or the preset heading)
FROM               Wallet picker · Pay from options · status line · [option panel]
TO                 Someone else | My wallets · field or (wallet picker · Receive into options · status line)
AMOUNT             AmountInput (DASH) + Max · inline error
SUMMARY            You send · Fee · Total from <source> · [Recipient gets] · [Details ▸]
ACTIONS            hint line (why Send is disabled) · [Cancel] [Send]
```

Option rows are single-select chips (radio semantics). The selected chip shows a fill, a 2 px border **and** a ✓ glyph, so selection does not rely on colour. Each chip shows a short balance (`Core · 0.8123`). The status line sits directly under its chip row.

## 3. Option catalogue

### 3.1 Pay from (source)

| Option | Shown when | Enabled when | Extra control |
|---|---|---|---|
| Auto | always | always | — |
| Core | always | Core spendable > 0 | — |
| Platform | always | Platform balance > 0 | — |
| Identity | the From wallet has ≥ 1 identity | the selected identity has a usable transfer key | identity dropdown if > 1 (default: the app-selected identity if it is in this wallet, else the highest balance) |
| Shielded | shielded feature gate on (protocol v12+, Expert/Developer view) **and** balance > 0 | balance verified (SND-016) | — |
| Advanced | Expert/Developer view | always | §3.3 panel |

The tooltip on a disabled option says why: "Core has no funds in Main." / "Your private balance is still being checked." / "This identity has no key that can send credits."

### 3.2 Receive into (destination, *My wallets* only)

| Option | Shown when | Result |
|---|---|---|
| Auto | always | §4.2 |
| Core | always | the To wallet's next unused Core receive address |
| Platform | always | the To wallet's next unused Platform address |
| Identity | the To wallet has ≥ 1 identity | top up the chosen identity (dropdown) |
| Shielded | shielded gate on | the To wallet's shielded receive address |

The To wallet picker lists HD wallets on the active network. A single-key wallet can be a destination (Core only, its one address; Auto = Core).

Disabled (with tooltip):
- The option identical to an explicit source in the same wallet. Tooltip: "You're paying from this balance. Pick a different one, or a different wallet."
- Route-invalid options: Identity→Shielded, Shielded→Identity. Tooltip: "Private funds can't go straight to an identity. Receive into Platform first, then top up the identity."

### 3.3 Advanced (source) panel

```
Spend from   (●) Core addresses   ( ) Platform addresses
[✓] yXa1…9f   0.5000 DASH
[ ] yQb7…2c   0.3123 DASH
Selected: 0.5000 DASH
```

The fee strategy is not part of this panel. It lives in Summary › Fee options (§7.1).

- The checklist shows only addresses with a balance. Everything is unticked when first opened. The amount still comes from the single Amount field (D-11).
- Max = everything in the ticked addresses minus the fee.
- Platform: at most `MAX_PLATFORM_INPUTS` ticked. Further boxes are disabled with the tooltip "One payment can use at most {max} Platform addresses."
- The panel replaces the Platform "Source breakdown" box. Without Advanced, the breakdown lives in Summary › Details.

### 3.4 Several recipients (Expert/Developer view)

"+ Add another recipient" under the Someone-else field turns it into rows of `[address] [amount] [✕]`, and the single Amount field is hidden. All recipients must be the same kind, either all Core or all Platform (today's backend). A row of a different kind shows: "All recipients in one payment must be the same kind. Send to this address separately." Removing all rows but one returns to the single-recipient layout and keeps that row's amount.

## 4. Auto resolution

### 4.1 Source Auto

Inputs: the destination kind `D` (typed address kind, or the resolved My-wallets option), the amount `A` (or Max), and the per-candidate spendable maximum `max(c, D)`. That maximum is the same value Max would offer for that route, from the #1107 quotes for Core and asset-lock routes, or from the Platform/shield estimators.

```
candidates(D):  Core      → [Core, Platform]
                Platform  → [Platform, Core]
                Identity  → [Platform, Core]
                Shielded  → [Shielded, Platform, Core]
                unknown   → preview only: the larger of Core and Platform
drop any candidate that is the destination account itself (same wallet + same kind)
drop Shielded if the shielded gate is off or the balance is unverified
pick the first c with max(c, D) ≥ A          (A empty → first c with max > 0)
none → "no single balance" state (§8)
```

- **Re-resolves** when D changes, when A changes (each valid parse), when balances or quotes change, or when the From wallet changes.
- **Quote pending**: if the candidate being checked has its #1107 quote still loading, the status line reads "Auto: checking balances…". Send stays disabled until the quote arrives. Quotes are requested only for the candidate being checked, never for all candidates.
- **No split**: one payment comes from one source (D-06).
- **Max with Auto**: Max picks the candidate with the largest `max(c, D)` and fixes Auto on it while Max is active. Editing the amount releases the fix.
- An explicit source is never switched automatically, even when it cannot cover the amount (§8 error instead).

### 4.2 Destination Auto (My wallets)

```
kind = source kind      (Advanced source → its Core/Platform kind)
if source is Identity                  → Platform            (SR-09)
if To wallet == From wallet:
    Core → Platform, Platform → Core, Shielded → Platform
address = next unused receive address of `kind` in the To wallet
```

The status line shows "Getting a new address in Savings…" until the address arrives. The address is fetched again only when the To wallet or the kind changes (it stays the same while unused).

### 4.3 Auto on both sides

Choosing Auto on side X while side Y is Auto pins Y to the concrete option Y's Auto had resolved to. If Y had no resolution, Y gets its §5 default. A one-time info line appears under Y: "Pay from is set to Platform so Receive into can choose automatically." (or the mirror wording). It disappears on the next change. Auto on Y stays clickable; clicking it flips the pin.

## 5. Defaults

| Field | Default |
|---|---|
| From wallet | the wallet the screen was opened from (the app-selected wallet) |
| Pay from | Auto |
| To mode | Someone else, field focused |
| My wallets → To wallet | the From wallet |
| My wallets → Receive into | Different wallet: the kind (Core/Platform) with the most funds in the From wallet (U10). Same wallet: the other kind. Ties go to Core. Auto is never the default here, because Pay from is already Auto |

## 6. Amount and Max

- One `AmountInput` in DASH, with Max, a 150 pt minimum width, and the label "Amount".
- **Max is an intent.** The field shows "Max" in the input. A secondary line reads "Everything available in Core · ≈ 0.8121 DASH". Typing any digit returns to an exact amount (#1106 semantics). Core→Shielded Max moves the whole Core balance and is resumable (#1106); the Summary says so.
- Max with no fee coverage: Max is disabled with the tooltip "Core can't cover the network fee right now." (keeps SND-014 / SND-017 behaviour).
- Minimum amounts (asset-lock funding) are inline errors on the field, using today's copy.

## 7. Summary card

Shown once there is a destination and an amount (or Max).

| Row | When |
|---|---|
| Route sentence, e.g. "Pay from Main · Core to yX12…ab" | always (replaces "Transaction type" and the button verb) |
| You send | always |
| Network fee ≈ | the fee is estimable; otherwise "Network fee: calculated when you send" |
| Recipient gets ≈ | the fee is taken from the amount (Core→Platform funding, or a "Reduce … output" strategy) |
| Total from Core ≈ | always; names the source |
| Note | route-specific, at most one line (below) |
| Fee options ▸ | §7.1 |
| Details ▸ | Expert view: the addresses used and the amount from each (today's breakdown) |

### 7.1 Fee options (SR-22)

All 4 existing strategies are kept, with today's labels and default. They appear in a collapsed row inside the Summary card. The header shows the current choice, so the row is useful even while collapsed:

```
Fee options: Deduct from first input ▸
  (●) Deduct from first input     ← default
  ( ) Deduct from last input
  ( ) Reduce first output
  ( ) Reduce last output
```

- **Shown when all three hold:** Expert/Developer view, Advanced source or several recipients is active, and Platform is involved (a Platform source or a Platform recipient; today's condition). Otherwise the row is absent: Everyday users never see it, and simple sends keep today's per-route fee behaviour.
- It starts collapsed on every visit. The choice survives collapsing and section changes (SR-16), and resets with the form.
- Choosing a "Reduce … output" strategy adds the "Recipient gets ≈" row and the confirmation sentence "The network fee will be deducted from the output amount, so the recipient will receive less than the entered amount." (today's copy).
- Each option has an `info_tooltip` in plain words, e.g. "Reduce first output: the first recipient receives the amount minus the fee."

Route notes:
- Platform/Identity → Core: "Withdrawals to a Dash address can take a while to arrive."
- Core → Platform/Identity/Shielded: "Your Dash is moved to Platform first, so this takes a little longer than a regular payment."
- Core → Shielded Max: "Moves your whole Core balance. If the app closes, you can finish it from the Shielded tab."

## 8. Validation and error copy

Rule: validate on blur or on a completed paste, never on every keystroke. Errors appear inline under the field they concern, in the `VALIDATION_WARNING` colour with a ⚠ glyph.

| Situation | Where | Copy |
|---|---|---|
| Unrecognised address | To field | "This isn't a Dash address, username or identity ID. Check it and paste it again." |
| Other network | To field | "This is a Testnet address, but you're on Mainnet. Ask the recipient for a Mainnet address." |
| Username not found | To field | "No one has the username “bob”. Check the spelling." |
| Explicit source can't pay this kind | To field | "You can't send from an identity to a private address. Choose Auto or Platform under Pay from." |
| Own address = source account | To field (warning, not a block) | "This address belongs to Main · Core, the balance you're paying from. You'd only pay a fee." |
| Amount > explicit source max | Amount | "Core can send at most 0.8121 DASH after the fee." + [Use 0.8121] |
| Auto: nothing covers | Amount + status line | "No single balance in Main covers 2.0000 DASH plus the fee. The most one payment can send is 0.8121 DASH, from Core." + [Send 0.8121 instead] |
| Below funding minimum | Amount | today's copy (SND-008 / SND-017) |
| Max quote failed | Amount | today's "The available amount could not be checked." + [Retry available amount check] |
| Recipients of different kinds | recipient row | §3.4 |

**Disabled Send** always has a hint line above the buttons that names the first missing step: "Enter who you're paying." → "Enter an amount." → "Fix the highlighted field." The button also carries a `disabled_tooltip` with the same text.

## 9. Locked wallets, sync, network

- **Locked From wallet**: the form stays usable (D-10) and balances come from the snapshot. Next to the wallet picker, the 🔒 glyph and the line "Locked. You'll be asked for the password when you send." Pressing Send opens `WalletUnlockPopup`. On a successful unlock the confirmation dialog opens. Cancel returns to the form unchanged. *Fallback if snapshot balances are unavailable while locked:* show an inline [Unlock] in the From section only; To stays usable.
- **Locked To wallet** (My wallets): unused-address derivation should need only public keys. If it does not, the status line reads "Unlock Savings to get a receiving address." with an [Unlock] link.
- **Password-protected identity keys** (SEC-001): the existing sign-time prompt; there are no new states.
- **Sync not finished**: the status line adds "Balances are still syncing." Auto still resolves on what is known. Send follows today's rules.
- **Network switch** while open: the form resets to the defaults with the info banner "The network changed, so the send form was cleared." Identity-bound flows keep today's "Back to identity" screen.
- **Single-key wallet as From**: listed but greyed out in the picker with the SND-002 reason.

## 10. Confirmation and after sending

`ConfirmationDialog`. Title: "Send 0.5000 DASH?" Body: a two-column grid:

```
From     Main · Core
To       <the full recipient address>   (or "bob (<the full identity ID>)" / "your private balance in Main")
Amount   0.5000 DASH
Fee      ≈ 0.0001 DASH
Total    ≈ 0.5001 DASH
<route note, if any>
```

The confirmation always shows the recipient address in full, never middle-ellipsised. With a shortened address, an attacker only has to match a few characters at each end. A lookalike address pasted from history (address poisoning) would then pass this last check before an irreversible payment. A username shows with its full identity ID, and the own private balance shows by name.

Buttons: [Cancel] [Send] (Confirm on the right; Escape = Cancel). The action is built at the moment Send is pressed, so the dialog shows exactly what will be sent even if Auto would re-resolve later. Danger mode stays as it is today (irreversible send).

The waiting and success states are unchanged: a spinner with "Sending…", then the success message with [Send another] [Back to wallet].

## 11. Entry points and presets (SR-19)

| Entry | From | To | Fixed (read-only) |
|---|---|---|---|
| Wallet › Send | current wallet · Auto | Someone else | — |
| Shielded › Shield | current wallet · Auto (Core/Platform only) | My wallets · same wallet · Shielded | To |
| Shielded › Send (Private) | Shielded | Someone else (shielded addresses only) | From |
| Shielded › Unshield | Shielded | My wallets · Auto (→ Platform) | From |
| Identity › Send | Identity X | Someone else | From |
| Identity › Top up | wallet picker · Auto (Identity excluded) | My wallets · Identity X | To |
| Identity › Move to wallet | Identity X | My wallets · Core (own addresses only) | From, To mode |

A fixed section shows its value as text with the label "Set by Shield". It has no chips.

## 12. Progressive disclosure

| View | Sees |
|---|---|
| Everyday (Alex) | Auto, Core, Platform, Identity (only if the wallet has identities); Someone else / My wallets; Summary without Details |
| Expert (Priya) | + Advanced, + Add another recipient, + Summary Details, + Summary Fee options (§7.1), + Shielded (if gate) |
| Developer (Jordan) | same as Expert; amounts may also show credits where the app already does so elsewhere |

## 13. Responsive layout

| Width of central panel | Layout |
|---|---|
| ≥ 640 pt | Labels on the left (100 pt column), controls on the right. Chips on one row. Summary as a 2-column grid |
| 420–639 pt | Labels above controls. Chip rows wrap (`horizontal_wrapped`). The Summary stays 2 columns |
| < 420 pt | Chips drop the balance text (shown in their tooltip and the status line). The Summary becomes stacked label/value pairs. Recipient rows stack address over amount. Buttons go full-width, Send on top |

The form scrolls in the existing `ScrollArea`. The status lines wrap and are never truncated. Addresses are middle-ellipsised (`yX12…9fab`) with the full value in a tooltip and copy-on-click.

## 14. Accessibility (WCAG 2.2 AA within egui's limits)

- **Focus order** follows layout: From wallet → Pay from chips → option panel → To mode → To field(s) → Amount → Max → Details → Cancel → Send. Tab and Shift+Tab work; Space/Enter selects a chip. **Enter** in Amount opens the confirmation when valid.
- **Focus visible**: `BORDER_WIDTH_THICK`, 3:1 contrast. Chips meet the minimum target size (24×24 pt, 2.5.8).
- **Not colour alone**: the selected chip has ✓; errors have ⚠ plus text; Locked has 🔒 plus text; Auto status lines always start with the word "Auto:".
- **Status changes**: egui has no live regions. To make up for that, the status line text is complete and self-contained, and the confirmation dialog repeats the resolved route, so nothing is shown only as a change.
- **Tooltips**: `info_tooltip` on chip labels (what Core/Platform/Identity/Shielded mean in one sentence), `disabled_tooltip` on every disabled control, `clickable_tooltip` on copy-able addresses.
- **Dragging** (2.5.7): none needed. **Redundant entry** (3.3.7): values survive section changes (SR-16).
- Tooltip copy for chips:
  - Core: "Your regular Dash balance. Works with any Dash address."
  - Platform: "Your balance on Dash Platform. Fast and cheap to send to Platform addresses and identities."
  - Identity: "Credits held by one of your identities."
  - Shielded: "Your private balance. Sending from it to a non-private address reveals the amount."
  - Advanced: "Choose exactly which addresses to spend from."

## 15. Edge cases

| Case | Behaviour |
|---|---|
| Wallet with only Core funds | Platform chip disabled with "No funds". Auto → Core for any destination it can pay |
| Empty wallet | All funding chips disabled. Auto line: "Auto: Main has no funds to send." Amount is disabled. The hint line reads "Add funds to Main first." with a [Receive] link |
| Only identities hold funds | Auto cannot pay (it never spends identities). Auto line: "Auto: Main's balances are empty. Choose Identity to pay from an identity." |
| Pasted own address of another account | Allowed; Auto skips the same-account candidate |
| User picks an explicit source, then a destination it can't reach | The destination stays; there is an inline reason under the To field; Send is disabled |
| Amount typed before a destination | Auto previews the larger balance and resolves for real once a destination exists |
| Balance changes mid-form (incoming tx) | Auto re-resolves; Max (intent) follows the new maximum |
| Identity dropdown: the app-selected identity is not in the From wallet | Default to the highest-balance identity of that wallet |
| Top up: source wallet holds the target identity | The target is excluded from Pay from › Identity (today's rule) |
| More than `MAX_PLATFORM_INPUTS` Platform addresses needed | Auto treats Platform's max as the top-N sum (today's rule). The Summary note reads "Limited to the {max} largest Platform addresses. Send the rest in another payment." |
| Shielded To kind, shielded gate off | The typed shielded address is refused inline: "Private addresses aren't available in this view. Switch to Expert view to use them." |

## 16. Implementation notes (non-binding)

- The Auto resolver and the default-kind rule go in a pure `model/` function, unit tested against the §4 tables. The UI only renders the result.
- Reuse: `WalletSelector` (both wallet pickers), `AddressInput` (Someone else; keep its kind detection, DPNS lookup and `wallet:` search), `AmountInput`, `IdentitySelector`, `ConfirmationDialog`, `WalletUnlockPopup`, `ResponseExt` tooltips. The chip row is the only new widget. If it is reused elsewhere (Shield source toggle), it belongs in `ui/components/` as a generic `ChoiceChips`.
- Keep: `PlatformFeeStrategy` (all 4 variants, same default). Only its combo box moves into Summary › Fee options.
- Delete: the `show_advanced_options` form, `render_advanced_send`, the per-input amount fields, the "Transaction type" hint and the verb-per-route button label.
