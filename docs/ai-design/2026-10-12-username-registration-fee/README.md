# Username registration cost estimate

Where the constants in `PlatformFeeEstimator::estimate_username_registration`
(`src/model/fee_estimation.rs`) come from, and how to refresh them.

## What the estimate is

Registering a username sends two documents, each as its own request: a
preorder, then the name. The estimate is the balance an identity needs for both
to be accepted. It excludes the community vote fee, which is added separately.

That balance is higher than what the network charges. Before accepting a
request, Platform checks the balance against its own advance estimate of the
fee, and for the name document that advance estimate is about twice the fee it
then takes. An identity holding only the charge pays for the preorder and is
refused at the name.

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
| 1.10 | | | headroom, see "Known gaps" |

For a 29-character name that needs no vote: 131,233,300 credits.

## Where the numbers come from

They were read from Platform's own engine, offline, at the pinned revision
`9f7ed16935bd540e4c2a542688800d2953f9f67a`. No network was involved.

Method: a test added to a scratch copy of `rs-drive-abci`, next to the existing
`register_dpns_username` helper in
`src/execution/validation/state_transition/state_transitions/batch/tests/document/dpns.rs`.
It registers names through `process_raw_state_transitions` and records, per
request, the storage fee, the processing fee and the balance change. It then
searches for the smallest starting balance with which both requests are
accepted.

### Charged (protocol 14, first name of a new identity)

| Label | Vote | Preorder storage | Name storage | Total charged |
|---|---|---|---|---|
| 8 chars | no | 31,158,000 | 35,883,000 | 70,926,920 |
| 29 chars | no | 31,158,000 | 38,151,000 | 72,882,540 to 74,073,780 |
| 63 chars | no | 31,158,000 | 41,877,000 | 77,923,080 |
| 19 chars | yes | 31,158,000 | 72,846,000 | 109,453,260 plus the vote fee |

Storage is exact and depends only on the label length. Processing varies with
how full the trees are: 1.2 to 3.0 million credits for the preorder, 1.9 to 3.8
million for the name.

A second name for the same identity is cheaper: the preorder stores 726 bytes
instead of 1154, and a name without a vote 288 bytes fewer. The estimate assumes
a first name.

### Smallest accepted balance (without the vote fee)

| Label | Vote | Protocol 14 | Protocols 11 to 13 |
|---|---|---|---|
| 6 chars | no | 115,965,270 | |
| 29 chars | no | 118,215,941 to 118,309,783 | 117,578,125 to 118,096,160 |
| 63 chars | no | 122,264,862 | |
| 3 chars | yes | 156,187,414 | 155,487,060 to 155,944,824 |
| 5 chars | yes | 156,750,678 | |
| 19 chars | yes | 158,495,902 | 157,892,703 to 158,424,973 |

These are the figures the unit test
`username_registration_estimate_covers_the_balance_the_network_requires` pins.
The estimate lands 10 to 11% above each.

### Live measurement

One registration on testnet (2026-10-12, 29-character name, no vote, identity
created moments earlier) was charged 75,646,740 credits. The engine predicts
72.9 to 74.1 million for the same input, so the live network charged about 2 to
4% more. That fits the same stored bytes with more processing, which grows with
the size of the trees.

## Known gaps

- The smallest accepted balance was never measured on a live network. The 10%
  headroom is there to cover the difference seen in the one live charge.
- A name that joins an existing community vote was not measured. The figures are
  for the first request for a name.
- The reserve Platform wants is a property of its engine, not of the fee
  schedule alone. Re-run the method above when the pinned Platform revision
  changes how documents or their indexes are stored.
