# Scenario: Token Creator workspace and registration recovery

**Verifies:** the complete token draft remains understandable and editable across
seven sections, and final creation uses the settings the user reviewed.

**Tier justification:** a real window is needed to check scrolling, responsive
layout, focus, readable help and navigation. The optional funded extension also
checks actual asynchronous registration and recovery. Deterministic unit tests
cover payload binding, nonce drift and simulated unknown/conflicting outcomes;
those tests do not prove that a live registration succeeded.

## Prerequisites and setup

- Follow the [GUI testing guidelines](../README.md): build the selected revision,
  record its commit and binary SHA-256, and launch on a verified display with a
  fresh isolated data directory and `.env.example`.
- Select **Testnet**. The main procedure needs no wallet, identity or funds.
- Enable accessibility when checking labels and keyboard focus. Identify this
  process explicitly if another app instance is running.
- Do not rebuild or replace the executable during the run. Record screenshots,
  window dimensions, theme and observations outside the repository.

## Comparing with the approved prototype

Use the approved HTML unchanged. Record its SHA-256 and the native process PID,
running executable path, commit SHA and executable SHA-256. A build log alone
does not prove which binary the window is running. On Linux, identify the window
PID and hash `/proc/<pid>/exe`; a deleted or replaced executable requires a fresh
launch before comparison.

Match both sides before capturing: light or dark theme, window dimensions,
client viewport, display scaling, browser zoom and device pixel ratio. Record
the values explicitly; native and browser window chrome can give different
client areas at the same outer size. Use the same draft and readiness state:
`StudioCredit` / `StudioCredits`, two decimal places, the same selected policy,
amounts, permissions, recipients, distribution settings, groups and history.
For the no-funds run, use the prototype's no-identity state. Do not compare its
funded demo identity with an empty native profile or invent matching balances.

Capture all seven sections, including scrolled content, expanded controls and
the relevant help, comparison and rule dialogs. Keep a per-section text
inventory alongside screenshots. Compare headings, labels, explanations,
examples, actions and consequences; then independently compare typography,
colors, spacing, hierarchy and clipping. Account for protocol differences
explicitly, including required plural names, names without spaces, real curve
calculations, validation, fees and registration recovery.

Report **content**, **visual fidelity** and **functional behavior** separately,
with PASS, FAIL or NOT TESTED and evidence for each. Navigation, a screenshot
gallery or passing kittests do not establish visual fidelity. A theme, viewport,
zoom, draft or readiness mismatch prevents a comparable visual verdict. List
remaining differences and their justification; never normalize the approved
prototype to make the native application pass.

## Procedure without funds

1. Open **Tokens → Token Creator**. Check the single horizontal sequence:
   **Basics**, **Supply & permissions**, **Distribution**, **Groups**, **History**,
   **Search & documents**, **Review**. Verify that **Back** and **Continue** follow
   that sequence and remain reachable while the editor scrolls.
2. Enter recognizable singular and plural names without spaces. Expand **Names
   in other languages** and add a localization. In **Supply & permissions**,
   change precision to zero with distributions disabled, then restore two
   decimals. Edit the description in **Search & documents**. Confirm that switching sections,
   **My Tokens → Create Token**, and the side navigation preserve this draft.
3. Compare the fixed and owner-managed examples. Inspect the proposed amount and
   permission changes before applying. Cancel one proposal; apply another; edit
   a field and undo. Check that amounts, permissions and schedule text match the
   restored draft, including an unfinished date such as `2026-12-`.
4. In **Supply & permissions**, edit initial and maximum supply in token and base
   units. Check an invalid fraction and a cap below initial supply: retain the
   text and identify the correction. Inspect all 13 rules, performers,
   administrators and their three allowances. Check that transferability, mint
   destination and recurring-rule authority are distinct settings.
5. In **Distribution**, enable recurring allocation and inspect each of the
   eight functions and its worked example. Compare table amounts with the
   current parameters; initial supply is separate from cumulative allocation.
   Check time/block/epoch intervals and the EvoNode recipient's epoch constraint.
   Add a scheduled allocation with an explicit local date and time. Leave and
   return: the displayed instant must stay fixed. Inspect once-per-identity
   allocation and the selected network's support notice.
6. In **Groups**, add two members with public fixture identity IDs and powers,
   set a reachable threshold and assign a rule to the group. A referenced group
   cannot disappear silently. Duplicate members and unreachable thresholds must
   produce corrections. Removing a different group must preserve the logical
   target; final review shows the actual contract positions.
7. In **History**, change individual ledgers and check the mixed master toggle.
   In **Search & documents**, edit keywords and a document-type map. Check invalid
   JSON, duplicate object keys and an external `$ref`: inputs stay editable and
   do not trigger external file or URL retrieval.
8. Open contextual help from the start, a permission and a distribution
   parameter. Verify examples and consequences. Press Tab twice, then Escape;
   focus returns to the opener. Check accessible names for text fields, selectors
   and help buttons. **Video tutorial** points to the approved tutorial folder.
9. Open **Review** without an identity. The draft overview remains readable;
   creation is unavailable and the screen explains how to choose an owner.
   Nothing in this procedure spends credits or produces a success receipt.
10. Repeat the important layouts in light and dark themes, at a normal desktop
    width and a narrower window. The selected section stays visible in the
    horizontal row; summary and navigation remain reachable. Inspect all seven
    sections and the comparison/help/rule dialogs for clipping or unreadable text.
11. Inspect only this isolated profile's logs for a crash or panic. Record each
    observation independently of the expected behavior.

## Optional funded extension — separate operator authorization required

<!-- TODO(gui-qa): Execute this live extension with an operator-approved testnet
identity and explicit spending cap. No paid creation or live recovery is claimed
by the automated tests or the no-funds procedure. -->

Use only an isolated testnet fixture. Import credentials at runtime from the
operator's secure store; never put them in this document or evidence. Agree on
one registration and a maximum spend first. Account for the registration charge
plus additional processing/storage charges; the app does not promise an exact
settled total. An equivalent fixture is required for each build in an A/B run.

1. Load and unlock the fixture owner. Select an eligible signing key and use
   **Check settings**. Inspect owner, network, amounts, powers, distribution,
   group positions, history, documents and the full JSON. Compare the displayed
   registration charge and owner balance in DASH and credits.
2. Edit one setting after the check. Creation must require another check and
   review. If another client changes the owner's state before creation, the app
   must return to review before broadcasting a changed payload.
3. Acknowledge the reviewed settings and select **Create token and pay fees**
   once. Navigate away and back during the request. No second paid submission
   may become available while the outcome is unresolved.
4. On confirmed acceptance, compare the receipt's contract and token IDs with
   the accepted contract and **My Tokens**. Check copy/open actions. A failed
   local save or balance refresh must not relabel a confirmed creation as failed.
5. If the connection fails after submission and the outcome is unknown, stop
   all paid attempts. Restart this isolated app and use **Check status · no
   payment**. Absence alone must not unlock retry; a different payload at the
   expected ID must show a conflict. Only a matching accepted contract confirms
   this attempt. Do not delete its pending record to force a retry.
6. Restore a confirmed receipt locally without another payment. Create another
   token only under a newly approved spending allowance.

## Expected outcome and known limits

Judge the actual baseline/development observations under the shared A/B
contract. Features absent in the baseline should be recorded as absent, not
treated as a completed baseline test. The development draft must retain all
supported advanced options and user input, distinguish editing from readiness,
and never invent an estimate, proof or success.

Drafts are in-memory and survive navigation; they are not promised to survive an
app restart. Public registration recovery metadata does survive restart. A
confirmed historical receipt remains confirmed even if the contract later
changes. A safely identified failure before any send can return to editing;
ambiguous transport errors cannot justify another payment.

<sub>🤖 Co-authored by [Claudius the Magnificent](https://github.com/lklimek/claudius) AI Agent</sub>
