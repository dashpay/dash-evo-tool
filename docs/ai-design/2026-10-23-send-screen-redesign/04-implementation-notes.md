# Send Screen Redesign — Implementation Notes

How the redesign in [01](01-requirements.md)–[03](03-wireframes.md) was built, what it keeps from the
old screen, and where the build departs from the spec.

## 1. Binding rule: nothing is lost

The user overrode Q2 (README): every capability of the old screen, including the "Advanced Options"
form, stays available. Advanced capabilities live behind the **Advanced** chip and the Expert view,
as the design places them, so the Everyday view stays simple.

## 2. Structure

| Layer | Module | Role |
|---|---|---|
| Model | `src/model/send_route.rs` | Pure Auto resolution (source and destination), default receive kind, route refusals. Unit tested against the §4 tables |
| UI state | `src/ui/state/send_form.rs` | The form's choices: Pay from, To mode, Receive into, Auto pinning, Max intent, recipients, Advanced selection, fee strategy, preset locks |
| Component | `src/ui/components/choice_chips.rs` | Generic single-select chip row |
| UI state | `src/ui/state/own_receive_addresses.rs` | Own receiving addresses fetched for My wallets, asked once per wallet and kind |
| Screen | `src/ui/wallets/send_screen/mod.rs` | Screen state, route validation and dispatch (the route functions are unchanged), confirmation, task results |
| Screen | `src/ui/wallets/send_screen/resolve.rs` | Each frame resolves the form into one concrete source and destination, and lists the quotes and addresses it still needs |
| Screen | `src/ui/wallets/send_screen/form_view.rs` | Renders From, To, Amount, Summary and Send, and builds the payment Send confirms |

Backend tasks are unchanged. Destination Auto reuses `WalletTask::GenerateReceiveAddress`
(Core, public keys only), `WalletTask::GeneratePlatformReceiveAddress` (needs the To wallet unlocked)
and the cached shielded receive address.

## 3. Capability preservation

| # | Old screen | New location |
|---|---|---|
| 1–14 | The 14 routes: Core→Core, Core→Platform, Core→Identity, Core→Shielded (own pool), Platform→Platform, Platform→Core, Platform→Shielded (own pool), Platform→Identity, Shielded→Shielded, Shielded→Platform, Shielded→Core, Identity→Core, Identity→Platform, Identity→Identity | A Pay from chip (or Auto) plus To: the Someone else field or a My wallets option |
| 15 | Identity source dropdown (default: app-selected identity, else highest balance) | Identity chip with its dropdown |
| 16 | Max per route (Core send quote, asset-lock quote, shield-planner quote, identity fee reserve, Platform top-N) | Max on the resolved source; with Auto, the largest single source |
| 17 | Quote status and retry | Inline under Amount |
| 18 | Funding minimums, "no amount covers the fee" | Inline under Amount |
| 19 | Fee preview | Summary card |
| 20 | Platform "Source breakdown" | Summary › Details (Expert) |
| 21 | "Transaction type" hint, per-route button verb | Summary route sentence; the button always says Send |
| 22 | Confirmation dialog (danger mode, "recipient receives" sentence) | Confirmation with From / To / Amount / Fee / Total lines |
| 23 | Advanced: Core or Platform per-address inputs with amounts | Pay from › Advanced panel (Expert): tick addresses, optional amount per address |
| 24 | Advanced: several outputs, kind tag, mixed-kind refusal | "+ Add another recipient" rows (Expert), inline mismatch |
| 25 | Advanced: 4 fee strategies | Summary › Fee options (Expert, an explicit Core-to-Platform payment: per-address amounts or several recipients; see A6) |
| 26 | Advanced: Core→Platform and Platform→Core accept one destination | Same refusal, inline |
| 27 | Advanced fee estimate | Summary card |
| 28 | Platform input/output coalescing and overflow checks | Unchanged functions |
| 29 | Shield preset: Core / Platform toggle | Pay from chips Auto / Core / Platform; To fixed to the private balance |
| 30 | Send (Private) preset | From fixed to Shielded; To Someone else, shielded addresses only |
| 31 | Unshield preset (Platform or Core address) | From fixed; To defaults to My wallets · Auto (Platform); Someone else stays available |
| 32 | Identity Send (fixed sender, identity without a wallet, any identity on this device as recipient) | From fixed |
| 33 | Identity Top up (wallet picker, in-flight refusal, other identities as source) | From wallet picker and chips; To fixed |
| 34 | Move to wallet (own Core addresses only) | To fixed to My wallets · Core (new unused address), plus "Choose an address" (the old own-address picker) |
| 35 | Unlock gate, password-less auto-open | 🔒 line under the wallet; unlock when Send is pressed; auto-open kept |
| 36 | Address field: usernames, `wallet:` / `type:` search, identity autocomplete | Someone else field, unchanged |
| 37–40 | Send status screens, pending-send tracking, progress banner, background top-up, network reset, completion messages | Unchanged (network switch adds an info banner) |

## 4. Deviations from the spec

| ID | Spec | Build | Why |
|---|---|---|---|
| A1 | AC-08, §4.1: a typed shielded address may be paid from Platform or Core | A shielded address that is **not** the From wallet's own is payable only from Shielded. Core and Platform pay into a shielded balance only for the own pool (My wallets · Shielded in the same wallet, the Shield preset, or the wallet's own pasted address) | `ShieldFromAssetLock` and `ShieldFromBalance` always deposit into the paying wallet's own pool and ignore the address. Paying a stranger that way would silently keep the money (§6) |
| A2 | §3.2: Receive into Shielded for any To wallet | With a different To wallet, only Shielded can pay it (a private transfer to that wallet's address). Core and Platform are disabled with a reason | Same backend limit as A1 |
| A3 | §4.1: quotes only for the candidate being checked | Holds, except that Max with Auto asks for every candidate's quote (at most three) | Max must compare all candidates |
| A4 | §6: the field shows the word "Max" | The field shows the amount; the caption reads "Everything available in Core · ≈ X" | Keeps `AmountInput` unchanged; the intent still holds until the amount is edited |
| A5 | D-11: per-address amounts dropped | Kept as an optional amount per ticked address. With none entered, the payment is spread over the ticked addresses. For Core the panel says so: the payment comes from the whole wallet, the ticked addresses only limit the amount, and Max offers no more than they hold | User decision (§1). **Kept behaviour:** Core "inputs" only cap the payment; `SendWalletPayment` builds from the pooled Core accounts and cannot select coins |
| A6 | §7.1: four fee strategies | Kept with today's effect: they change only Core→Platform (`fee_deduct_from_output`). Platform-source sends pay the fee from the largest input whatever the choice. The row appears only for an explicit Core-to-Platform payment (per-address amounts or several recipients): an Advanced source without amounts pays as the simple route does, which always takes the fee from the output, and Platform sources pick the paying input themselves | Kept behaviour, flagged for a follow-up. Showing a choice the payment would ignore was misleading |
| A7 | §3.4: several recipients | Core rows: from Core, Auto or Advanced Core. Platform rows: from Advanced Platform (as before) or the Platform chip (spread over all Platform addresses, every recipient excluded). Several recipients for Core→Platform or Platform→Core are still refused | Only the routes the backend supports |
| A8 | §3.1: Shielded only in Expert view | Shown whenever shielded operations are available (unchanged gate) | Nothing may be lost. Advanced and several recipients are Expert only (Q5) |
| A9 | §7, #1106: "Moves your whole Core balance" note, "Finish now" | Not shown. Max is an intent, so #1106 can add them | #1106 is not part of this branch |
| A10 | §10: a two-column grid in the confirmation | One line per field (From, To, Amount, Fee, Total) | The shared `ConfirmationDialog` takes text; each line stays one translation unit |
| A11 | §4.2: unused addresses | Platform: the first watched Platform address with no balance and no history, otherwise a new one, which needs the To wallet unlocked ("Unlock Savings to get a receiving address."). Core: the next unused receive address (public keys only) | Matches the Receive dialog |
| A12 | §3.2: a single-key wallet as a destination | Deferred | Out of scope |
| A13 | §3.1: Identity enabled when it has a usable transfer key | A transfer key for Platform and identity recipients, a withdrawal key for Core addresses | Withdrawals sign with a different key |
| A14 | §10: confirmation title | "Send {amount}?", confirm button "Send" | — |
| A15 | §2: wallet names in status lines | A wallet without a name reads "Unnamed wallet" | Sentences stay complete |

## 5. Issues found in the old screen

- **Core or Platform to a stranger's shielded address shields into your own pool** (A1). At base
  `4c31933b0`, `general_destination_kinds` (`src/ui/wallets/send_screen.rs:2758-2774`) accepts Shielded for
  the Core source. `send_core_to_shielded` (`:2029-2081`) dispatches `ShieldFromAssetLock { seed_hash, amount_duffs }`
  without the address, and `send_platform_to_shielded` (`:2166-2204`) dispatches `ShieldFromBalance` the same way.
  The backend deposits into `shielded_default_address(&seed_hash, 0)` (`src/backend_task/shielded/mod.rs:122`).
  Repro: Wallet › Send › Core Wallet, paste another wallet's shielded address, enter an amount, then Send and
  confirm. The funds land in the sender's own private balance. The redesign refuses this route before dispatch.
- A5 and A6 above: Core coin control is only a cap, and fee strategies only affect Core→Platform.

## 6. Tests

| Area | Tests |
|---|---|
| Auto rules | `src/model/send_route.rs` unit tests (24) |
| Form state | `src/ui/state/send_form.rs` (16), `src/ui/state/own_receive_addresses.rs` (3) |
| Chips | `src/ui/components/choice_chips.rs` kittests (4) |
| A1 regression | `core_and_platform_never_route_to_someone_elses_private_address` and `the_form_refuses_core_to_someone_elses_private_address` (`send_screen/mod.rs`), `w10_core_cannot_pay_someone_elses_private_address` (`tests/kittest/send_form.rs`) |
| UX-001 | `the_typed_recipient_survives_switching_to_my_wallets_and_back` |
| Flows | `tests/kittest/send_form.rs`: W1–W3, W5, W6, W8–W11, W13, W14. Each settles the harness before asserting; none counts frames |

W4 (destination options), W7 (identity source) and W12 (confirmation) are covered by the existing
identity and routing tests and by W2/W6, which open the confirmation and the My wallets options.

## 7. Follow-ups

- The shared `ComponentStyles::add_primary_button_enabled` draws a disabled button that only takes
  hover, so accessibility tools see it as enabled. The hint line above Send names the missing step, so
  the state stays visible. This affects every screen that uses the helper.
- A6: fee strategies for Platform-source sends.
- A12: single-key wallets as destinations.
