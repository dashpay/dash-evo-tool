# Scenario: Identity Home with a published profile

**Verifies:** the real profile avatar, username summary, and identity-bound entries into the shared send form.

**Tier justification:** live profile/image fetching and responsive rendering require the running app. Routing, fixed-party guards, and stale result handling also have deterministic tests.

## Prerequisites and setup

Follow the isolated-profile launch and accessibility setup in [the GUI testing guide](../README.md). Use testnet and an identity with a published avatar and at least four usernames. A public, view-only import is sufficient for this scenario; no keys or funds are needed. The `n00` identity was used during development.

## Procedure

1. Finish testnet synchronization. Open **Identities → Add → Load an existing identity → My username**, enter the public username, and search. Return to Identities and open the loaded identity card.
2. Check that the summary shows the published avatar, profile name, balance, and first three active usernames, with the main name first. Check that the summary sits above **Home / Contacts / Profile**. At desktop width, **Contacts** must start beside the action row at the top of the narrower right column. Setup, username notices, and payment history belong in the wider left column. A pending username notice must have left-aligned, wrapping text and must not push Contacts down.
3. Click **…** beside the usernames. Verify that Profile opens with all usernames, including pending requests. Click **+** in the summary and verify that registration names the same identity. A view-only import should explain that a key must be added before registration.
4. Return Home and open **Receive**. Verify the selected username and QR code, then close it.
5. Open **Send**. Verify the shared send form shows the selected identity as a fixed sender and offers recipient selection. Cancel, then open **Top up** and verify the same identity is the fixed recipient.
6. With a separate disposable wallet/key fixture, open **Move to wallet** and verify that only addresses of loaded wallets are offered. This action is disabled for a view-only identity without a withdrawal key.
7. Resize the window to a narrow width. Check that the summary remains readable, cards stack, and Profile's username section remains reachable. Repeat the layout checks in dark mode.

## Safety constraints

Do not register a name, change the public profile, accept a contact request, or confirm a transfer. Test only navigation, rendering, and form constraints. Use an isolated profile throughout.

## Pass criteria

The real avatar replaces the initial. Names and forms remain bound to the selected identity. No unfinished Activity tab or empty payment-history placeholder appears. Existing contacts and pending username information remain visible. Check the test profile's logs for panics.

## Known gotchas

- Username lookup before the masternode list is synced returns a retryable synchronization error; wait for the network screen to report synchronization complete.
- A public import has no signing keys. Disabled withdrawal and registration controls are expected.
- The sidebar scrolls independently in a short window; scroll it to expose Settings.
