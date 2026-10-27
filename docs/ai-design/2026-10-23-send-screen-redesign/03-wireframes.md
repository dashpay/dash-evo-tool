# Send Screen Redesign — Wireframes

The example data is the same throughout. Wallet **Main**: Core 0.8123, Platform 0.3000, Shielded 0.1000, identity *alice* 0.2000. Wallet **Savings**: Core 2.0000.
Legend: `[✓ X]` selected chip · `[ X ]` chip · `[x X]` disabled chip · `( )`/`(●)` radio · `▾` dropdown · `⚠` error · `ⓘ` info · `🔒` locked.
Rules: [02-ux-spec.md](02-ux-spec.md).

## W1 — Default (just opened)

```
┌─ Send Dash ───────────────────────────────────────────────────────────┐
│ FROM                                                                  │
│ Wallet     [ HD: Main — 1.4123 DASH                              ▾ ]  │
│ Pay from   [✓ Auto ] [ Core 0.8123 ] [ Platform 0.3000 ] [ Identity ] │
│ Auto: picks a balance once you enter who you're paying.               │
│                                                                       │
│ TO                                                                    │
│            (● Someone else )  ( My wallets )                          │
│            [ Address, username or identity ID                     ]   │
│                                                                       │
│ AMOUNT                                                                │
│            [ 0.00             ] DASH   [ Max ]                        │
│                                                                       │
│ Enter who you're paying.                                              │
│                                              [ Cancel ]  [x Send ]    │
└───────────────────────────────────────────────────────────────────────┘
```
Everyday view. Shielded and Advanced are hidden. Identity is shown because Main has an identity.

## W2 — Happy path: Auto resolves from the recipient

```
│ Pay from   [✓ Auto ] [ Core 0.8123 ] [ Platform 0.3000 ] [ Identity ] │
│ Auto: paying from Core · 0.8123 DASH available                        │
│ TO         (● Someone else )  ( My wallets )                          │
│            [ yX12pQ…9fab                                  Core ✓ ]    │
│ AMOUNT     [ 0.5              ] DASH   [ Max ]                        │
│ ┌ Summary ───────────────────────────────────────────────────────┐    │
│ │ Pay from Main · Core to yX12…9fab                              │    │
│ │ You send                      0.5000 DASH                      │    │
│ │ Network fee                 ≈ 0.0001 DASH                      │    │
│ │ Total from Core             ≈ 0.5001 DASH                      │    │
│ └────────────────────────────────────────────────────────────────┘    │
│                                              [ Cancel ]  [ Send ]     │
```

## W3 — Source options (explicit)

**Core**
```
│ Pay from   [ Auto ] [✓ Core 0.8123 ] [ Platform 0.3000 ] [ Identity ] │
```
**Platform**
```
│ Pay from   [ Auto ] [ Core 0.8123 ] [✓ Platform 0.3000 ] [ Identity ] │
```
**Identity** (the dropdown appears only when the wallet has more than one identity)
```
│ Pay from   [ Auto ] [ Core 0.8123 ] [ Platform 0.3000 ] [✓ Identity ] │
│            Identity [ alice — 0.2000 DASH                        ▾ ]  │
```
**Shielded** (Expert view, gate on)
```
│ Pay from   [ Auto ] [ Core ] [ Platform ] [ Identity ] [✓ Shielded 0.1000 ] [ Advanced ] │
```
**Disabled examples**
```
│ Pay from   [✓ Auto ] [ Core 0.8123 ] [x Platform 0 ]  …                         │
│                        tooltip: "Platform has no funds in Main."                  │
│            [x Shielded 0.1000 ]  tooltip: "Your private balance is still being checked." │
```

## W4 — Destination options

**Someone else, username**
```
│ TO         (● Someone else )  ( My wallets )                          │
│            [ bob                                                  ]   │
│            bob · identity 7Hk2…Qp1  ✓                                 │
```
**My wallets: Core / Platform / Identity / Shielded**
```
│ TO         ( Someone else )  (● My wallets )                          │
│ Wallet     [ HD: Savings — 2.0000 DASH                           ▾ ]  │
│ Receive    [ Auto ] [✓ Core ] [ Platform ] [ Identity ] [ Shielded ]  │
│ into       To a new Core address in Savings · yQb7…2c1d               │
```
```
│ Receive    [ Auto ] [ Core ] [✓ Platform ] [ Identity ] [ Shielded ]  │
│ into       To a new Platform address in Savings · tdash1qx…9f3k       │
```
```
│ Receive    [ Auto ] [ Core ] [ Platform ] [✓ Identity ] [ Shielded ]  │
│ into       Identity [ carol — 0.0500 DASH                        ▾ ]  │
```
```
│ Receive    [ Auto ] [ Core ] [ Platform ] [ Identity ] [✓ Shielded ]  │
│ into       To your private balance in Savings                         │
```
**Same wallet, explicit source Core**: the identical account is disabled
```
│ Wallet     [ HD: Main — 1.4123 DASH                              ▾ ]  │
│ Receive    [ Auto ] [x Core ] [✓ Platform ] [ Identity ]              │
│ into         tooltip: "You're paying from this balance. Pick a different one, or a different wallet." │
```

## W5 — Auto on source: fallback

```
│ Pay from   [✓ Auto ] [ Core 0.8123 ] [ Platform 0.3000 ] [ Identity ] │
│ Auto: paying from Core · Platform can't cover 0.5000 DASH plus the fee. │
│ TO         (● Someone else )  ( My wallets )                          │
│            [ tdash1qz8…m4ve                           Platform ✓ ]    │
│ AMOUNT     [ 0.5              ] DASH   [ Max ]                        │
│ ┌ Summary ───────────────────────────────────────────────────────┐    │
│ │ Pay from Main · Core to tdash1qz8…m4ve                         │    │
│ │ You send                      0.5000 DASH                      │    │
│ │ Network fee                 ≈ 0.0002 DASH                      │    │
│ │ Recipient gets              ≈ 0.4998 DASH                      │    │
│ │ Total from Core               0.5000 DASH                      │    │
│ │ Your Dash is moved to Platform first, so this takes a little   │    │
│ │ longer than a regular payment.                                 │    │
│ └────────────────────────────────────────────────────────────────┘    │
```
Changing the amount to 0.2 re-resolves live: "Auto: paying from Platform · 0.3000 DASH available".

## W6 — Auto on destination (pins the source)

From the defaults, the user picks My wallets, then Auto under Receive into:
```
│ Pay from   [ Auto ] [✓ Core 0.8123 ] [ Platform 0.3000 ] [ Identity ] │
│ ⓘ Pay from is set to Core so Receive into can choose automatically.   │
│ TO         ( Someone else )  (● My wallets )                          │
│ Wallet     [ HD: Main — 1.4123 DASH                              ▾ ]  │
│ Receive    [✓ Auto ] [x Core ] [ Platform ] [ Identity ]              │
│ into       Auto: to a new Platform address in Main · tdash1qx…9f3k    │
│            (same wallet, so Auto chose the other balance)             │
```
With the To wallet set to Savings: "Auto: to a new Core address in Savings · yQb7…2c1d".
Clicking Auto under Pay from again pins Receive into to Platform and shows the mirrored ⓘ line.

## W7 — Identity source, destination Auto (SR-09)

```
│ Pay from   [ Auto ] [ Core 0.8123 ] [ Platform 0.3000 ] [✓ Identity ] │
│            Identity [ alice — 0.2000 DASH                        ▾ ]  │
│ TO         ( Someone else )  (● My wallets )                          │
│ Wallet     [ HD: Main — 1.4123 DASH                              ▾ ]  │
│ Receive    [✓ Auto ] [ Core ] [ Platform ] [ Identity ] [x Shielded ] │
│ into       Auto: to a new Platform address in Main · tdash1qx…9f3k    │
```

## W8 — Advanced (Expert view)

**Advanced source**
```
│ Pay from   [ Auto ] [ Core ] [ Platform ] [ Identity ] [ Shielded ] [✓ Advanced ] │
│ ┌──────────────────────────────────────────────────────────────────┐  │
│ │ Spend from   (● Core addresses )  ( Platform addresses )         │  │
│ │ [✓] yXa1…9f02   0.5000 DASH                                      │  │
│ │ [ ] yQb7…2c1d   0.3123 DASH                                      │  │
│ │ Selected: 0.5000 DASH                                            │  │
│ └──────────────────────────────────────────────────────────────────┘  │
```
**Summary with Fee options** (Expert view + Advanced/several recipients + Platform involved; collapsed by default)
```
│ ┌ Summary ───────────────────────────────────────────────────────┐    │
│ │ Pay from Main · Platform to tdash1qz8…m4ve                     │    │
│ │ You send                      0.2000 DASH                      │    │
│ │ Network fee                 ≈ 0.0001 DASH                      │    │
│ │ Total from Platform         ≈ 0.2001 DASH                      │    │
│ │ Fee options: Deduct from first input ▸                         │    │
│ │ Details ▸                                                      │    │
│ └────────────────────────────────────────────────────────────────┘    │
```
Expanded:
```
│ │ Fee options: Reduce first output ▾                             │    │
│ │   ( ) Deduct from first input                                  │    │
│ │   ( ) Deduct from last input                                   │    │
│ │   (●) Reduce first output                                      │    │
│ │   ( ) Reduce last output                                       │    │
│ │ Recipient gets              ≈ 0.1999 DASH     ← added by Reduce │    │
```
**Several recipients**
```
│ TO         (● Someone else )  ( My wallets )                          │
│            [ yX12…9fab                    ] [ 0.2     ] DASH  [✕]     │
│            [ yP44…71aa                    ] [ 0.1     ] DASH  [✕]     │
│            [ tdash1qz8…m4ve               ] [ 0.05    ] DASH  [✕]     │
│            ⚠ All recipients in one payment must be the same kind.     │
│              Send to this address separately.                         │
│            + Add another recipient                                    │
│ (the single AMOUNT field is hidden; the Summary totals the rows)      │
```

## W9 — Max with Auto

```
│ Pay from   [✓ Auto ] [ Core 0.8123 ] [ Platform 0.3000 ] [ Identity ] │
│ Auto: paying from Core · Max uses the largest balance.                │
│ TO         [ yX12…9fab                                    Core ✓ ]    │
│ AMOUNT     [ Max              ] DASH   [✓ Max ]                       │
│            Everything available in Core · ≈ 0.8121 DASH               │
│ ┌ Summary ───────────────────────────────────────────────────────┐    │
│ │ You send                    ≈ 0.8121 DASH                      │    │
│ │ Network fee                 ≈ 0.0002 DASH                      │    │
│ │ Total from Core               0.8123 DASH (all of it)          │    │
│ └────────────────────────────────────────────────────────────────┘    │
```
Core → Shielded Max (#1106) adds the note: "Moves your whole Core balance. If the app closes, you can finish it from the Shielded tab."
While the quote loads (#1107), the field reads "Checking the available amount…" and Max is disabled.

## W10 — Validation errors

**No single balance covers the amount (Auto)**
```
│ Auto: no single balance in Main covers 2.0000 DASH plus the fee.     │
│ AMOUNT     [ 2.0              ] DASH   [ Max ]                        │
│            ⚠ The most one payment can send is 0.8121 DASH, from Core. │
│              [ Send 0.8121 instead ]                                  │
│ Fix the highlighted field.                    [ Cancel ]  [x Send ]   │
```
**Bad address / wrong network**
```
│            [ XpAbc…zz                                             ]   │
│            ⚠ This is a Mainnet address, but you're on Testnet.        │
│              Ask the recipient for a Testnet address.                 │
```
**Explicit source can't reach this kind**
```
│ Pay from   [ Auto ] [ Core ] [ Platform ] [✓ Identity ] …             │
│            [ dash1z…shield                             Shielded ]     │
│            ⚠ You can't send from an identity to a private address.    │
│              Choose Auto or Platform under Pay from.                  │
```
**Amount above an explicit source**
```
│ Pay from   [ Auto ] [ Core 0.8123 ] [✓ Platform 0.3000 ] …            │
│ AMOUNT     [ 0.5              ] DASH   [ Max ]                        │
│            ⚠ Platform can send at most 0.2999 DASH after the fee.     │
│              [ Use 0.2999 ]                                           │
```

## W11 — Locked wallet

```
│ Wallet     [ HD: Main — 1.4123 DASH                              ▾ ]  │
│            🔒 Locked. You'll be asked for the password when you send. │
│ …form fully usable…                                                   │
│                                              [ Cancel ]  [ Send ]     │
```
Send → `WalletUnlockPopup` → (unlocked) → W12. Cancelling the unlock returns here with nothing changed.

## W12 — Confirmation

```
        ┌─ Send 0.5000 DASH? ─────────────────────────────┐
        │ From     Main · Core                            │
        │ To       bob (yX12…9fab)                        │
        │ Amount   0.5000 DASH                            │
        │ Fee      ≈ 0.0001 DASH                          │
        │ Total    ≈ 0.5001 DASH                          │
        │                                                 │
        │ Check the recipient. Payments can't be undone.  │
        │                          [ Cancel ]  [ Send ]   │
        └─────────────────────────────────────────────────┘
```
Own-wallet example: "To   Savings · new Core address (yQb7…2c1d)".

## W13 — Narrow (< 420 pt)

```
┌─ Send Dash ───────────────────┐
│ FROM                          │
│ Wallet                        │
│ [ HD: Main — 1.4123 DASH ▾ ]  │
│ Pay from                      │
│ [✓ Auto ] [ Core ]            │
│ [ Platform ] [ Identity ]     │
│ Auto: paying from Core ·      │
│ 0.8123 DASH available         │
│ TO                            │
│ (● Someone else )             │
│ ( My wallets )                │
│ [ yX12…9fab          Core ✓ ] │
│ AMOUNT                        │
│ [ 0.5        ] DASH [ Max ]   │
│ Summary                       │
│ You send                      │
│   0.5000 DASH                 │
│ Network fee                   │
│   ≈ 0.0001 DASH               │
│ Total from Core               │
│   ≈ 0.5001 DASH               │
│ [          Send             ] │
│ [         Cancel            ] │
└───────────────────────────────┘
```

## W14 — Preset: Shield (from the Shielded tab)

```
┌─ Shield ──────────────────────────────────────────────────────────────┐
│ Move funds from your wallet or Platform balance into your private     │
│ balance.                                                              │
│ FROM       Wallet [ HD: Main — 1.4123 DASH ▾ ]                        │
│ Pay from   [✓ Auto ] [ Core 0.8123 ] [ Platform 0.3000 ]              │
│ Auto: paying from Platform · 0.3000 DASH available                    │
│ TO         Your private balance in Main          (set by Shield)      │
│ AMOUNT     [ 0.2              ] DASH   [ Max ]                        │
│ …Summary, Send…                                                       │
└───────────────────────────────────────────────────────────────────────┘
```
