# Username registration cost estimate

Where the constants in `PlatformFeeEstimator::estimate_username_registration`
(`src/model/fee_estimation.rs`) come from, and how to refresh them.

## What the estimate is

Registering a username sends two documents, each as its own request: a
preorder, then the name. The estimate is the balance an identity needs for both
to be accepted. It excludes the community vote fee, which is added separately.

That balance is higher than what the network charges. Before accepting a
request, Platform checks the balance against its own advance estimate of the
fee, and for the name document that advance estimate is about 41 million
credits above the fee it then takes. An identity holding only the charge pays
for the preorder and is refused at the name.

## Formula

```
bytes    = 1154 + reserve + per_char × label length
estimate = (bytes × storage rate per byte + 3,500,000) × 1.10 × fee multiplier
```

| Constant | No vote | Community vote | Meaning |
|---|---|---|---|
| first document | 1154 | 1154 | bytes billed for the preorder: 726, plus 428 the first time an identity writes to the name contract |
| `reserve` | 3019 | 4517 | balance wanted before the name document of an empty label is accepted, in bytes at the storage rate |
| `per_char` | 4 | 5 | bytes each label character adds |
| 3,500,000 | | | processing charged for the preorder |
| 1.10 | | | headroom over the engine figures |

| Label | Vote | Estimate |
|---|---|---|
| 20 chars | no | 130,164,100 |
| 29 chars | no | 131,233,300 |
| 63 chars | no | 135,272,500 |
| 19 chars | yes | 175,100,200 plus the vote fee |

## Measured on testnet (2026-10-12)

Each identity was created moments before and funded with exactly the estimate.
All four registrations were accepted.

| Label | Vote | Preorder | Name | Total charged |
|---|---|---|---|---|
| 20 chars | no | 32,554,660 | 42,817,780 | 75,372,440 |
| 29 chars | no | | | 75,533,180 |
| 63 chars | no | 32,103,900 | 47,465,940 | 79,569,840 |
| 19 chars | yes | 32,432,480 | 83,152,940 plus the vote fee | 115,585,420 plus the vote fee |

The vote fee on testnet was 20,000,000,000 credits (0.2 DASH).

Identities funded with 60,000,000 credits paid for the preorder and were
refused at the name. The refusal names the balance the network wanted:

| Label | Vote | Preorder charged | Wanted for the name | Smallest balance for both |
|---|---|---|---|---|
| 20 chars | no | 32,090,520 | 83,627,060 | 115,717,580 |
| 63 chars | no | 32,400,260 | 88,397,980 | 120,798,240 |

The estimate is 12% above both. For a name that needs a vote the same run
reports only the vote fee plus 100,000, from an earlier check, so the smallest
balance for a voted name is known from the engine only.

## Read from Platform's engine

The constants were read from Platform's own engine, offline, at the pinned
revision `9f7ed16935bd540e4c2a542688800d2953f9f67a`. No network was involved.

Method: a test added to a scratch copy of `rs-drive-abci`, next to the existing
`register_dpns_username` helper in
`src/execution/validation/state_transition/state_transitions/batch/tests/document/dpns.rs`.
It registers names through `process_raw_state_transitions` and records, per
request, the storage fee, the processing fee and the balance change. It then
searches for the smallest starting balance with which both requests are
accepted. Each registration needs its own block time, as on a live network.

### Stored bytes (protocol 14, first name of a new identity)

| Document | Bytes | Storage fee |
|---|---|---|
| Preorder | 1154 | 31,158,000 |
| Name, no vote | 1297 + 4 per label character | 37,179,000 at 20 chars, 41,877,000 at 63 |
| Name, with a vote | 2764 + 5 per label character | 77,193,000 at 19 chars |

Protocols 11 to 13 store one byte less per document. The name with a vote
includes 161 bytes for the entry that schedules the end of its vote; a vote
ending at a time another vote already ends at would not pay them.

A second name for the same identity is cheaper: the preorder stores 726 bytes
instead of 1154, and a name without a vote 280 bytes fewer. The estimate assumes
a first name.

### Processing

Processing is charged on top of storage and grows with the number of names
already registered (protocol 13):

| Names already registered | Name, 20 chars, no vote | Name, 19 chars, with a vote |
|---|---|---|
| 10 | 2,759,920 | 3,074,020 |
| 500 | 4,945,880 | 4,191,800 |
| 6000 | 5,700,280 | 5,044,440 |

With 6000 names the engine charges 42,852,280 for a 20-character name; testnet
charged 42,817,780.

### Smallest accepted balance (without the vote fee)

| Label | Vote | Protocol 14 | Protocols 11 to 13 |
|---|---|---|---|
| 6 chars | no | 115,965,270 | |
| 29 chars | no | 118,215,941 to 118,309,783 | 117,578,125 to 118,096,160 |
| 63 chars | no | 122,264,862 | |
| 3 chars | yes | 156,110,524 to 156,187,414 | 155,487,060 to 155,950,188 |
| 5 chars | yes | 156,750,678 | |
| 19 chars | yes | 158,383,845 to 158,495,902 | 157,892,703 to 158,424,973 |

The balance wanted for the name document does not depend on the block time, and
differs by less than 0.3% between an empty engine and testnet. The unit test
`username_registration_estimate_covers_the_balance_the_network_requires` pins
these figures; the estimate lands 10 to 11% above each.

## Known gaps

- The smallest balance for a name that needs a vote was not measured on a live
  network; the estimate plus the vote fee was accepted there.
- A name that joins an existing community vote was not measured. The figures are
  for the first request for a name.
- The reserve Platform wants is a property of its engine, not of the fee
  schedule alone. Re-run the method above, and the `measure_dpns_registration`
  module of the backend end-to-end suite, when the pinned Platform revision
  changes how documents or their indexes are stored.

## Replacing the constants

The constants are fitted to how Drive bills the DPNS contract and should give
way to Platform's own numbers
([dashpay/platform#5383](https://github.com/dashpay/platform/issues/5383)):

- The charge: `drive::document::cost::document_create_cost`
  ([dashpay/platform#5159](https://github.com/dashpay/platform/pull/5159)). It
  is in Platform's `v4.2-dev` but not in the pinned revision, so it needs a
  Platform bump first.
- The balance Drive wants before it accepts a document: no function yet,
  requested in
  [dashpay/platform#5265](https://github.com/dashpay/platform/issues/5265).
