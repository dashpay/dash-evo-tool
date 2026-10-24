# Send Screen Redesign — Requirements

Status: implemented; see [04-implementation-notes.md](04-implementation-notes.md) for deviations. Screen: Wallet › Send (`src/ui/wallets/send_screen/`).

## 1. Executive summary

**Problem.** The Send screen asks users to understand Core vs Platform vs
Identity vs Shielded *before* it knows who they are paying. It also runs two
parallel UIs, Simple and "Advanced Options", with different layouts, validation
and fee read-outs. Everyday users (Alex) just want "pay this address this
amount".

**Direction.** There is one form with two sections, **From** and **To**, and
**Auto** as the default way to pick the source. The app chooses the cheapest
compatible balance and says what it chose, live. Moving money between your own
wallets and accounts is a first-class mode, so you never copy your own address.
"Advanced Options" no longer switches to another screen. It becomes one option
inside each section.

## 2. Today's screen: inventory and problems

| Area | Today |
|---|---|
| Wallet | Read-only wallet info grid (no picker; only identity Top up has a `WalletSelector`) |
| Modes | Simple form; "Advanced Options" checkbox (top-right) swaps in a different form; shielded presets (Shield, Send (Private), Unshield); identity-bound modes (Send, Top up, Move to wallet) |
| Sources (Simple) | Radio cards: Core Wallet, Platform Addresses (combined), Identity (if identities exist **or** Expert view; dropdown when >1), Shielded Balance (feature gate + balance > 0) |
| Destination | One `AddressInput`. The accepted kinds depend on the source. It autocompletes wallets and identities and resolves DPNS names |
| Amount | `AmountInput` + Max; caption with fee reserve; minimum amount; "Transaction type: …" hint; a warning when no amount covers the funding fee |
| Extras | "Source breakdown" box listing Platform addresses used; fee summary card |
| Send | Button label changes per route: Send DASH, Fund Platform Address, Top Up Identity, Shield DASH, Transfer Credits, Withdraw to Wallet, … |
| Advanced form | Source type Core or Platform; per-address inputs **with per-address amounts**; output rows (+ Add Output) with type detection; fee strategy (4 options); its own fee summary |
| Gates | The unlock gate hides the whole form until the wallet is unlocked |
| Routes | 14 supported; Identity→Shielded and Shielded→Identity are refused at Send time |

Problems found (`UX-NNN`, severity per the `severity` skill):

| ID | Sev | Finding |
|---|---|---|
| UX-001 | High | Changing the source clears the typed destination (`address_input = None`, `validated_destination = None` on every source click). A pasted address is silently lost. |
| UX-002 | High | Users must pick an account kind before the recipient. That kind limits which addresses are accepted, so an address can be "invalid" only because of an unrelated earlier choice. |
| UX-003 | Medium | Two separate forms (Simple and Advanced) with different layouts, validation, fee summaries and button labels. Switching loses state and doubles what users must learn. |
| UX-004 | Medium | Fee information is spread over four places: the amount caption, the "Transaction type" hint, the source-breakdown box and the fee summary. |
| UX-005 | Medium | Paying into your own other wallet or account means finding and copying your own address (or knowing the `wallet:` search tag). |
| UX-006 | Low | The Send button label changes between about 10 verbs as inputs change. It repeats the "Transaction type" hint. |
| UX-007 | Low | The unlock gate hides the whole form. Users cannot see what they are about to do before unlocking. |
| UX-008 | Low | The Identity source card shows in Expert view even when the wallet has no identities, which leaves an empty option. |

## 3. Actors

| Actor | Goal on this screen |
|---|---|
| Alex (Everyday) | Pay an address or username fast (< 10 s) without knowing account kinds |
| Priya (Power) | Move funds between own wallets and accounts; occasionally choose which balance or addresses pay |
| Jordan (Developer) | Exercise every route (shielded, identity, multi-output) on testnet |
| Identity hub (entry point) | Opens the form bound to an identity (Send / Top up / Move to wallet) |
| Shielded tab (entry point) | Opens the Shield / Send (Private) / Unshield presets |

## 4. Requirements

`[U#]` = traces to the user's numbered requirement. `D-xx` = deviation (§5).

### Structure
- **SR-01** `[U1]` The form has exactly two sections, **From** and **To**, then Amount, Summary and actions. There is no separate Advanced form (D-01).
- **SR-02** `[U2]` From = wallet picker + **Pay from** options: Auto · Core · Platform · Identity · Shielded · Advanced.
- **SR-03** `[U3]` To = a two-way switch **Someone else | My wallets** (D-02).
  - *Someone else*: one address field (address, username or identity ID; type detected automatically).
  - *My wallets*: wallet picker (default: the From wallet) + **Receive into** options: Auto · Core · Platform · Identity · Shielded.
- **SR-04** Multiple recipients are not a To option. They appear as "+ Add another recipient" under the Someone-else field (D-03).

### Auto
- **SR-05** `[U4]` Auto is active on at most one side. Choosing Auto on one side while the other side is Auto pins the other side to the concrete option its Auto had resolved to. **The side the user last touched wins** (D-04).
- **SR-06** `[U5]` Source Auto tries, in order, the first candidate whose spendable maximum (same calculation as Max, SND-017) covers amount + fee:

  | Destination kind | Candidate order |
  |---|---|
  | Core | Core → Platform |
  | Platform | Platform → Core |
  | Identity | Platform → Core |
  | Shielded | Shielded → Platform → Core |
  | not yet known | the largest of Core and Platform (preview only) |

  Auto never spends Identity balances, never spends Shielded funds for a non-private destination, never splits one payment across two sources, and skips a candidate that is the destination account itself (D-05, D-06).
- **SR-07** `[U6]` Next to source Auto, a status line names the chosen balance and how much it holds. It updates live when the destination, amount or balances change, and says why when Auto fell back.
- **SR-08** `[U7]` Destination Auto (My wallets only) receives into an **unused** address of the resolved kind in the chosen wallet. Kind = same as the source kind. If the To wallet equals the From wallet, Auto uses the other kind: Core↔Platform, Shielded→Platform. The address is shown in the status line.
- **SR-09** `[U8]` If the source is Identity, destination Auto = a new Platform address.
- **SR-10** `[U9]` Next to destination Auto, a status line names the wallet, the kind and the short address.
- **SR-11** With *Someone else*, the destination kind comes from the typed value (Core / Platform / Shielded address, identity ID, or username → identity). Source Auto uses it.

### Defaults
- **SR-12** `[U10]` From = the current wallet + Auto. To = **Someone else** with an empty, focused field (D-07). When the user switches to *My wallets*: the To wallet is the From wallet. The To option is the kind holding the most funds in the From wallet when the wallets differ (literal U10). When they are the same wallet, it is the other kind, so Auto pays from the larger balance and never sends to itself (D-07).

### Amount, Max, fees
- **SR-13** Max works on the resolved source. With source Auto, Max takes the largest single-source maximum among the candidates for the destination and fixes Auto on that source. Max is an *intent* ("everything in Core"). It stays Max until the amount is edited (compatible with #1106).
- **SR-14** There is one **Summary** card with all fee information: amount, estimated fee, total debited from the named source, and "Recipient gets" when the fee comes out of the amount. Address-level detail is behind "Details" (Expert view). This replaces the amount caption, the "Transaction type" hint and the breakdown box (D-08).
- **SR-15** When no source can cover amount + fee, an inline error under Amount names the largest amount that one payment can send and offers it in one click.

### Safety and state
- **SR-16** Changing any From option never clears the To value, and the reverse. Incompatible combinations show an inline reason (fixes UX-001).
- **SR-17** The Send button label is always **Send**. The confirmation dialog states route, source, destination, amount, fee and total (D-09).
- **SR-18** A locked wallet does not hide the form. Balances stay visible, and **Send** asks to unlock, then continues to the confirmation (D-10).
- **SR-19** The presets (Shield, Send (Private), Unshield, identity Send / Top up / Move to wallet) open this same form pre-filled. The fields the preset fixes are shown read-only.
- **SR-20** Unsupported routes (Identity→Shielded, Shielded→Identity) cannot be chosen. Their options are disabled with a reason; they are not refused after clicking Send.
- **SR-21** Advanced (source) = "spend only from these addresses": one checklist of Core *or* Platform addresses with balances (D-11). Visible in Expert/Developer view only (D-13).
- **SR-22** All 4 existing fee strategies are kept. They sit in a collapsed **Fee options ▸** row inside the Summary card. The row is shown only in Expert/Developer view, only while Advanced source or several recipients is active, and only when Platform is involved (today's condition). The default stays "Deduct from first input" (D-12).

### Acceptance criteria (key)

| ID | Given | When | Then |
|---|---|---|---|
| AC-01 | Main: Core 0.8, Platform 0.3; From Auto | Alex pastes a Core address and types 0.5 | The status line reads "Auto: paying from Core · 0.8000 DASH available". Send is enabled. |
| AC-02 | Same wallet as AC-01 | Alex changes the amount to 1.0 | The status line reads "Auto: no single balance in Main covers 1.0000 DASH plus the fee." Amount shows the max-offer link. Send is disabled. |
| AC-03 | Same wallet, amount 0.5 | Alex pastes a Platform address | Platform (0.3) cannot cover the amount, so Auto falls back to Core and the line reads "Auto: paying from Core · Platform can't cover 0.5000 DASH plus the fee." |
| AC-04 | From Auto, To My wallets · Core | The user picks Auto under To | From is pinned to its resolved option (e.g. Platform). An info line reads "Pay from is set to Platform so Receive into can choose automatically." |
| AC-05 | From Identity "alice" | The user picks My wallets · Auto | The To status line reads "Auto: to a new Platform address in Main · tdash1q…9f". |
| AC-06 | The user typed an address, then changes Pay from | — | The address stays. If the new source cannot pay it, an inline reason appears under the address. |
| AC-07 | Locked wallet, valid form | The user presses Send | The unlock dialog opens. After a successful unlock, the confirmation dialog opens. Cancelling the unlock leaves the form unchanged. |
| AC-08 | From Auto, Shielded destination typed, Shielded balance 0 | — | Auto resolves to Platform, then Core. Max follows the shield-quote rules (#1107/#1106). |

## 5. Deviations from the user's requirements

| ID | Requirement | Deviation | Rationale |
|---|---|---|---|
| D-01 | U2 "Advanced renders the hidden sources" | Advanced is a source option *inside* the one form, not a separate form | Removes the second UI (UX-003); the state is kept |
| D-02 | U3 "pick a wallet, then options + a manual field" | **Someone else / My wallets** switch; the manual field is its own mode | One active destination at a time. Without the switch, a wallet+option choice and a typed address compete, and nothing shows which one wins |
| D-03 | U3 "same options as source" (incl. Advanced) | No Advanced chip under To; multi-recipient = "+ Add another recipient" under the address field | Several recipients are external payees, so they belong to *Someone else*. A To-side Advanced chip would hide the address field |
| D-04 | U4 states "only one side" | Rule: last-touched wins and the other side is pinned to its resolved value; it is not disabled | A disabled Auto creates "why can't I click this?" moments. Pinning keeps what the user saw and can be undone |
| D-05 | U5 "fall back to Platform" | Full order per destination (SR-06); Core is the final fallback; Identity is never spent automatically; Shielded only for shielded destinations | Identity credits pay for usernames and Platform fees, so spending them silently would surprise users. Unshielding silently breaks privacy |
| D-06 | (gap) | Auto never splits one payment across two sources | A split is two transactions that can partly fail. "No single balance covers this" is honest and fixable with one click (SR-15) |
| D-07 | U10 default destination = current wallet + Core/Platform | Default To = **Someone else**. U10 applies when the user picks *My wallets*; for the same wallet it is adapted to "the other kind" | With U10 as written plus U5, the default is a send to yourself (Core→Core in the same wallet) that pays only a fee, and Send is enabled as soon as an amount is typed. Alex's main task is paying someone (SND-001) |
| D-08 | (gap) | One Summary card replaces four fee/route read-outs | UX-004 |
| D-09 | (gap) | The button always says "Send"; the route verb moves to the summary and confirmation | UX-006. With Auto, a label that changes as you type is unsettling |
| D-10 | (gap) | Unlock when Send is pressed, not up front | UX-007. Matches the sign-time unlock pattern already used elsewhere |
| D-11 | U2 Advanced = today's hidden contents | Per-address amounts are dropped. The user ticks the addresses to spend from; the amount comes from the one Amount field | Per-address amounts are the most error-prone part of the screen. Coin control means "which addresses", not "how much from each" |
| D-12 | (today) 4 fee strategies in the Advanced form | **All 4 kept**, moved into a collapsed "Fee options ▸" row in the Summary card (Expert view, Advanced/several recipients, Platform involved). The default is unchanged | The Advanced form no longer exists, so the choice needs a new home. Fees belong next to the fee read-out, and collapsing keeps every other view free of it. *User decision: keep all 4.* |
| D-13 | U2 lists Advanced for everyone | Advanced and "+ Add another recipient" show only in Expert/Developer view | Progressive disclosure: used by fewer than 20% of users |
| D-14 | U2 labels "Core Wallet", "Platform Addresses", "Shielded Balance" | Short labels **Core**, **Platform**, **Shielded**, **Identity**, plus info tooltips | Match the wallet screen's pills; they fit narrow widths |

## 6. Dependencies

- **#1107** — Max for Core / asset-lock / Platform→Shielded comes from upstream quotes, and Core pools all transparent accounts. Source Auto must use those same quotes as its "covers" test (SR-06), so "Core" means the pooled Core balance.
- **#1106** — Core→Shielded Max drains everything, can be resumed, and has "Finish now". SR-13 treats Max as an intent that stays until the amount is edited. The Summary and confirmation say "Moves your whole Core balance" for that route.
- **Unused-address allocation** for destination Auto: Core uses `WalletBackend::next_receive_address`. Platform and Shielded must reuse whatever the Receive dialogs use (WAL-010, WAL-029). It must work for a locked To wallet (xpub only); otherwise, see the 02 spec §9.
- **Auto resolver** = a pure function in `model/` (inputs: candidate maxima, destination kind, amount, same-account flag) so it can be unit tested. The UI only renders it.
- `WalletSendScreen::new_with_wallet()` hard-codes `SourceSelection::CoreWallet`. The new default (Auto) and the presets (SR-19) need a constructor that takes an initial From/To instead. (The DashPay `SendPaymentScreen` has the same Core hard-code. It is out of scope, but should be aligned later.)
- Today the general form shows the wallet read-only; only identity Top up has a `WalletSelector`. A From wallet picker is new and reuses `WalletSelector` (shared label format from user story UX-006, `set_unavailable` for single-key wallets per SND-002).

## 7. Decisions

Q1–Q5 are resolved. See [README.md](README.md#decisions-resolved-open-questions).
