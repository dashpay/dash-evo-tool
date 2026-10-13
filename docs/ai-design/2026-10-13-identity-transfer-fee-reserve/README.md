# Fee reserve for a transfer out of an identity

Where the `*_TRANSFER_RESERVE_*` constants behind
`PlatformFeeEstimator::estimate_credit_transfer` and
`estimate_credit_transfer_to_addresses` (`src/model/fee_estimation.rs`) come
from, and how to refresh them.

## What the estimate is

The balance an identity must keep, above the amount, for Platform to accept a
transfer of credits to another identity or to Platform addresses. Max and the
check made before sending both use it.

That balance is higher than the fee Platform charges. Platform checks a
transfer twice. The first check wants the amount plus a fixed minimum fee:
100,000 credits for another identity, 500,000 plus 6,000,000 per address for
Platform addresses. The second check, `validate_fees_of_event` in
`rs-drive-abci`, wants the amount plus Drive's own advance estimate of the fee,
which is several times the fee it then takes. An identity that keeps only the
minimum fee passes the first check and is refused at the second.

## Formula

```
to an identity:   8,755,720 × 1.10 × fee multiplier
to n addresses:  (11,261,680 + 6,176,980 × n) × 1.10 × fee multiplier
```

Neither falls below Platform's minimum fee for the transfer.

| Transfer | Estimate | In DASH |
|---|---|---|
| to an identity | 9,631,292 | 0.0000963 |
| to 1 address | 19,182,526 | 0.000192 |
| to 2 addresses | 25,977,204 | 0.000260 |
| to 3 addresses | 32,771,882 | 0.000328 |

## Measured on testnet

| Date | Transfer | Refused below, above the amount | Charged |
|---|---|---|---|
| 2026-10-13 | to an identity, 1,009,900,000 credits | 8,755,720 | |
| 2026-10-13 | to an identity, 10,000,000 credits | | 2,992,260 |
| 2026-10-12 | to 1 address | 17,331,040 | |

Both refusals are 1,980 credits above the engine figures below, 0.02%.

## Read from Platform's engine

The figures were read from Platform's own engine, offline, at the pinned
revision `9f7ed16935bd540e4c2a542688800d2953f9f67a`. No network was involved.

Method: a test module added to a scratch copy of `rs-drive-abci`, next to the
tests in
`src/execution/validation/state_transition/state_transitions/identity_credit_transfer/mod.rs`.
For each case it creates a sender with a given balance, sends the transfer
through `process_raw_state_transitions`, and raises the balance to what each
refusal asks for until the transfer is accepted. It then checks that one credit
less is refused.

### To an identity: smallest accepted balance above the amount

| Protocol | ECDSA key | Hashed ECDSA key | BLS key |
|---|---|---|---|
| 11 | 8,555,220 | 8,555,720 | 8,840,220 |
| 12 to 14 | 8,753,740 | 8,754,240 | 9,038,740 |

It does not depend on the amount (500,000 to 2 × 10^16 credits), on whether
the recipient holds credits, or on the block time and epoch.

### To Platform addresses: smallest accepted balance above the amount

Protocols 12 to 14, ECDSA key:

| Addresses | Each receives less than 2,147,483,648 credits | Each receives that or more |
|---|---|---|
| 1 | 17,329,060 | 17,438,660 |
| 2 | 23,396,440 | 23,615,640 |
| 3 | 29,463,820 | 29,792,620 |

Each further address adds 6,067,380, and 109,600 more once it receives
2,147,483,648 credits (0.0215 DASH) or more; nothing changes above that, up to
2 × 10^16 credits. The constants are fitted to the higher column.

Protocol 11 wants 237,860 less. A BLS key adds 285,000 and a hashed ECDSA key
500. The address type and whether the address already holds credits make no
difference, and neither do the block time and epoch.

### Charged

| Transfer | Engine | Testnet |
|---|---|---|
| to an identity | 614,040 to 2,008,520 | 2,992,260 |
| to a new address | 6,791,980 to 8,054,540 | |
| to an address that holds credits | 836,160 to 1,885,420 | |

The engine starts empty, so it charges less processing than a live network. A
new address costs 6,075,000 in storage.

## Margins

The estimate for a transfer to an identity is 10.0% above the testnet figure
and 6.6% above the engine figure for a BLS key. For one address it is 10.7%
above the testnet figure and 8.2% above the engine figure for a larger amount
signed with a BLS key. The unit tests
`credit_transfer_estimate_covers_the_balance_the_network_requires` and
`credit_transfer_to_addresses_estimate_covers_the_balance_the_network_requires`
pin the figures above and keep each estimate within 15% of them.

## What the user sees

The reserve is kept back in full and charged in part; the rest stays in the
identity. After a Max transfer to another identity about 0.00007 DASH remains.
The last 0.0000963 DASH of an identity cannot be transferred to another
identity, and the last 0.000192 DASH cannot be transferred to a Platform
address.

## Known gaps

- Acceptance at the estimate was not tried on a live network. The live figures
  are the two refusals above.
- The engine figures are for protocols 11 to 14. Mainnet and testnet were not
  asked which protocol they run.
- The Send screen keeps back the estimate for another identity when the
  destination is a Platform address, so it still reserves too little there.
- The reserve Platform wants is a property of its engine, not of the fee
  schedule alone. Re-run the method above when the pinned Platform revision
  changes how balances, nonces or address balances are stored.

## Replacing the constants

Platform offers no function for the balance it wants before it accepts a
transition; the SDK exposes only the minimum fee of the first check.
[dashpay/platform#5265](https://github.com/dashpay/platform/issues/5265) asks
for that function. When the pinned revision has it, the constants give way to
it.
