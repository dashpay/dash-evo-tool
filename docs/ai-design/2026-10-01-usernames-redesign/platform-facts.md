# DPNS usernames & contested-resource voting — platform facts

Source: dashpay/platform @ `9f7ed16935bd540e4c2a542688800d2953f9f67a` (DET pin). Paths are relative to
`/home/ubuntu/.cargo/git/checkouts/platform-7a21f318038a582f/9f7ed16/packages/`.
`LATEST_VERSION = PROTOCOL_VERSION_14` (`rs-platform-version/src/version/mod.rs:36`).
Network status (web, unverified against chain): mainnet + testnet run **PV13** (Platform 4.1); PV14 (4.2) is the next upgrade.
Credits: 1 DASH = 100,000,000,000 credits.

Abbreviations: PV = protocol version; "testing" = any network != Mainnet (testnet, devnet, regtest).

## 1. Fees

| Fact | Value | PV-dependent | Citation |
|---|---|---|---|
| Contested registration contribution ("vote resolution fund") | **0.2 DASH** (20,000,000,000 cr) PV1–PV13; **0.1 DASH** (10,000,000,000 cr) PV14+ | Yes (`fee_version`) | `rs-platform-version/src/version/fee/vote_resolution_fund_fees/v1.rs:4`, `.../v2.rs:4-8`; `fee/v3.rs:6-10`; `version/v14.rs:537` (FEE_VERSION3), `v13.rs:91` (FEE_VERSION2) |
| Same on mainnet vs testnet | identical | — | (no network branch in fee version) |
| How the amount gets into the tx | dpp auto-sets `prefunded_voting_balance = (index_name, amount)` from the **client SDK's** `PlatformVersion` when the document matches the contested index | Yes | `rs-dpp/src/data_contract/document_type/methods/versioned_methods.rs:649-681`; `rs-dpp/.../document_create_transition/v0/from_document.rs:34` |
| Exact-match rule | Platform rejects if paid amount != active PV amount (`DocumentContestNotPaidForError`); also rejects a prefund on a non-contested doc (`DocumentContestNotRequiredError`) and a prefund naming a different index | Yes | `rs-drive-abci/.../document_create_transition_action/advanced_structure_v1/mod.rs:59-124` |
| How DET reads it | `sdk.version().fee_version.vote_resolution_fund_fees.contested_document_vote_resolution_fund_required_amount` (SDK auto-detects network PV unless pinned with `with_version`) | — | `rs-platform-version/src/version/fee/vote_resolution_fund_fees/mod.rs:5-13`; `rs-sdk/src/sdk.rs:397,639-645,1044` |
| Who pays / when | The registering identity, at execution of the `domain` create batch (same tx as the document). Moved from identity balance into a per-contest prefunded specialized balance | — | `rs-drive/src/state_transition_action/action_convert_to_operations/batch/document/document_create_transition.rs:98-113` |
| Every joiner pays too | Yes — each contender's create carries its own prefund into the same contest balance | — | same as above |
| Refund on win / lose / lock | **Never refunded.** Each vote spends 0.0001 DASH from the fund; at contest end the remainder is emptied into epoch processing credits (masternode reward pool). Applies equally to winner, losers, lock | — | `rs-drive/.../masternode_vote_transition.rs:48-56`; `rs-drive-abci/.../clean_up_after_contested_resources_vote_polls_end/v1/mod.rs:42-75` |
| "Unlock" amount | 4 DASH constant exists (`contested_document_vote_resolution_unlock_fund_required_amount`) but is only quoted in the `DocumentContestCurrentlyLockedError`; **no unlock path is implemented** | — | `vote_resolution_fund_fees/v1.rs:5`; `rs-drive-abci/.../document_create_transition_action/state_v1/mod.rs:239-247` |
| Non-contested cost | No fixed fee. Normal document fees for 2 docs (preorder + domain): storage 27,000 cr/byte + processing; min 100,000 cr per batch sub-transition. Order of magnitude ~0.0005–0.001 DASH (DET's "0.2006" = 0.2 + ~0.0006 estimate) | Fee tables versioned, unchanged PV14 | `rs-platform-version/src/version/fee/storage/v1.rs:6-9`; `fee/state_transition_min_fees/v1.rs:8` |
| Registration is 2 txs | preorder (salted double-sha256 of salt‖`label.dash`) then domain; domain create verifies the preorder exists | — | `rs-sdk/src/platform/dpns_usernames/mod.rs:148-311`; `rs-drive-abci/.../triggers/dpns/v1/mod.rs:329-396` |

## 2. Document mutability

| Flag / capability | Value | PV-dependent | Citation |
|---|---|---|---|
| `documentsMutable` | false (Replace impossible) | — | `dpns-contract/schema/v1/dpns-contract-documents.json:3` |
| `canBeDeleted` | true in schema, **but Delete is rejected by a DPNS data trigger** on all PVs | — | schema `:4`; `rs-drive-abci/.../data_triggers/bindings/list/v2/mod.rs:39-51` |
| `transferable: 1`, `tradeMode: 1` (direct purchase) | Declared since genesis; Transfer/Purchase/UpdatePrice were blocked by reject triggers until **PV13**, allowed PV13+ | Yes (bindings v0 → v1/v2) | schema `:5-6`; `bindings/list/v0/mod.rs:49-61`; `bindings/list/v1/mod.rs:16-26` |
| History | DPNS contract v2 (PV13+) adds `keepsTransferHistory/PurchaseHistory/PricingHistory` (via document-history contract) | Yes | `dpns-contract/schema/v2/dpns-contract-documents.json:7-9`; `system_data_contract_versions/v2.rs:5-11`; upgrade at `rs-drive-abci/.../perform_events_on_first_block_of_protocol_change/v0/mod.rs:682` |
| `records.identity` on transfer/purchase | Drive rewrites it to the new owner | PV13+ | `rs-drive/.../batch/document/mod.rs:18-26`; `document_transfer_transition.rs:102,138`; `document_purchase_transition.rs:114,155` |
| `records.identity` at create | Schema requires `records` with only `identity` (32-byte id). The trigger's owner-equality checks target legacy keys `dashUniqueIdentityId`/`dashAliasIdentityId`, which the schema forbids → **`records.identity` is not enforced to equal `$ownerId`** | — | schema `:83-99`; `triggers/dpns/v1/mod.rs:150-183`; `dpns-contract/src/v1/mod.rs:17-18` |
| Primary/"main" name | **No on-chain concept.** Schema has no such field; purely client-side | — | schema `:41-115` |
| Owned-by-identity index | `identityId` index on `records.identity` (`nullSearchable: false`) | — | schema `:31-39` |
| Subdomains | `subdomainRules.allowSubdomains` required; SDK sets false | — | schema `:100-114`; `rs-sdk/.../dpns_usernames/mod.rs:232-238` |

## 3. Contest lifecycle

| Fact | Value | PV-dependent | Citation |
|---|---|---|---|
| When a contest exists | First create of a name matching the contested index opens it **immediately with one contender**; no 2-contender minimum | — | `rs-drive/src/drive/document/insert_contested/add_contested_document_for_contract_operations/v0/mod.rs:54-104` |
| Contested rule | `normalizedLabel` matches `^[a-zA-Z01-]{3,19}$` (after homograph normalization: only letters, `0`, `1`, `-`; length 3–19) | — | schema `:20-29` |
| Contest duration (end = start block time + duration) | Mainnet **14 days** (1,209,600,000 ms). Testing **90 min** (5,400,000 ms) PV3+; 14 days PV1–2 | Yes (`voting_versions`) | `dpp_voting_versions/v2.rs:3-7`, `v1.rs:3-7`; insert code `:54-70` |
| Join window (new contenders) | Mainnet **7 days** (604,800,000 ms). Testing **45 min** (2,700,000 ms) PV2+; 7 days PV1. After it: `DocumentContestNotJoinableError` | Yes | `dpp_validation_versions/v1.rs:31-32`, `v2.rs:31-32`, `v3.rs:32-33`; `state_v1/mod.rs:249-260` |
| When voting is possible | Whole time the poll status is `Started` (includes join window) until resolution; otherwise `VotePollNotAvailableForVotingError` | — | `rs-drive-abci/.../masternode_vote/state/v0/mod.rs:72-92` |
| What ends it | Per-block `check_for_ended_vote_polls` using block time ≥ end date; max 2 end-time entries processed per block → resolution can lag a few blocks | constants versioned | `rs-drive-abci/src/execution/platform_events/voting/check_for_ended_vote_polls/v0/mod.rs:66-85`; `drive_abci_validation_versions/v10.rs:391-392` |
| Winner | Highest weighted tally among contenders (top 100 considered). Abstain is ignored. If **lock tally > top tally → Locked**; lock tie → contender wins | — | `check_for_ended_vote_polls/v0/mod.rs:203-290`; `tally_votes.../v0/mod.rs:25-38` |
| Tie between contenders | `max_by(created_at, then block height, core height, doc id)` → the **latest-created** document wins the tie (counter-intuitive; not first-come) | — | `check_for_ended_vote_polls/v0/mod.rs:252-268` |
| Zero votes | All contenders tie at 0 → latest-created contender **wins** unless ≥1 lock vote. A lone uncontested requester wins after 14 days with no votes | — | same |
| Losers | Contender documents, votes, end-date entries removed; loser's doc is not stored | — | `rs-drive-abci/.../clean_up_after_contested_resources_vote_polls_end/v1/mod.rs:34-40`; `rs-drive/src/drive/votes/cleanup/` |
| Record kept | Stored poll info persists with `WonByIdentity(id)` / `Locked` / `NoWinner` plus voter lists | — | `rs-dpp/src/voting/vote_info_storage/contested_document_vote_poll_winner_info/mod.rs:18-23`; `check_for_ended_vote_polls/v0/mod.rs:292-300` |
| Locked name | **Permanently unregistrable** in this version: new contested create → `DocumentContestCurrentlyLockedError` (unlock not implemented) | — | `state_v1/mod.rs:239-247` |
| Awarded name re-request | Rejected (`DocumentAlreadyPresentError`); domain cannot be deleted so the name only moves via transfer/purchase | — | `state_v1/mod.rs:230-238` |

## 4. Voting rules

| Fact | Value | PV-dependent | Citation |
|---|---|---|---|
| Who votes | Any masternode in the platform's full masternode list (regular or Evo) | — | `rs-drive-abci/.../masternode_vote/transform_into_action/v0/mod.rs:111-124` |
| Weight | Regular **1**, Evonode **4** | — | same `:121-124` |
| Voter identity | Auto-created per masternode: id = `create_voter_identifier(proTxHash, votingAddress)` | — | `rs-dpp/src/identifier/mod.rs:15`; `masternode_vote/advanced_structure/v0/mod.rs:58-73` |
| Voting key | `ECDSA_HASH160`, purpose `VOTING`, security `HIGH`, read-only; data = masternode voting address. Signer pubkey hash must equal the MN's current voting address | — | `rs-drive-abci/.../get_voter_identity_key/v0/mod.rs:12-25`; `advanced_structure/v0/mod.rs:43-55` |
| Choices | `TowardsIdentity(id)`, `Abstain`, `Lock` | — | `rs-dpp/src/voting/vote_choices/resource_vote_choice/` |
| Max votes per masternode per contest | **5 total** (first vote + 4 changes); 6th → `MasternodeVotedTooManyTimesError`. Re-casting the same choice → `MasternodeVoteAlreadyPresentError` | Yes (`votes_allowed_per_masternode`, 5 in all versions) | `transform_into_action/v0/mod.rs:60-103`; `rs-drive/.../register_contested_resource_identity_vote/v0/mod.rs:95-117`; `dpp_validation_versions/v1.rs:33` |
| Counting key | Votes keyed by **proTxHash**, not voter identity (changing the voting key keeps the count) | — | `rs-drive/.../masternode_vote_transition.rs:41-47` |
| Cutoff | Only while `Started`; none after end/award/lock | — | `masternode_vote/state/v0/mod.rs:72-92` |
| Cost to voter | **Zero.** Fixed 0.0001 DASH (10,000,000 cr) per vote deducted from the contest's prefunded balance; tx fails if balance < 100,000 cr | — | `masternode_vote_transition.rs:48-56`; `rs-drive-abci/src/execution/types/execution_event/mod.rs:337-347`; `masternode_vote/balance/v0/mod.rs:45-80`; `fee/state_transition_min_fees/v1.rs:11` |
| Fund capacity | 0.2 DASH ⇒ 2,000 votes per contender contribution (0.1 DASH ⇒ 1,000 in PV14) | Yes | derived |
| Shared voting key across MNs | Allowed by protocol, but each MN has its own voter identity + nonce → **one transition per masternode**, all signed by the same key | — | `MasternodeVoteTransitionV0` fields `rs-dpp/.../masternode_vote_transition/v0/mod.rs:43-53` |
| Batching | **None** — one `MasternodeVoteTransition` = one `proTxHash` × one vote | — | same |
| Vote expiry | Votes of masternodes removed from the list are deleted each block | — | `rs-drive-abci/.../remove_votes_for_removed_masternodes/v0/mod.rs:16`; `run_dao_platform_events/v0/mod.rs:23-33` |

## 5. SDK queryability (rs-sdk)

| Need | API | Gotchas | Citation |
|---|---|---|---|
| (a) availability | `Sdk::is_dpns_name_available(name)` / `check_dpns_name_availability(label)` | **Only checks awarded `domain` docs.** Returns `true` for names in an active contest (contenders are not in primary storage) and for **locked** names. Must also query the contest vote state | `rs-sdk/src/platform/dpns_usernames/mod.rs:313-366`; `queries.rs:91` |
| (a') contest for a label | `get_contested_dpns_vote_state(label, limit)` → `Contenders { winner, contenders, abstain_vote_tally, lock_vote_tally }` | `winner` is `Some` once finished (incl. `Locked`) | `contested_queries.rs:125-158`; `rs-drive-proof-verifier/src/types.rs:163-172` |
| (b) active contests | `get_current_dpns_contests(start,end,limit)` → `label → end_time` (auto-paginates, default 100/page); low level `VotePollsByEndDateDriveQuery` | `get_contested_dpns_normalized_usernames` lists contested resources incl. historical; `get_contested_non_resolved_usernames` fans out one vote-state query per contest | `contested_queries.rs:432-502, 62-112, 325-375` |
| (c) tally & contenders | `ContenderWithSerializedDocument` via (a'); voters per contender: `get_contested_dpns_voters_for_identity` | `limit` u16 | `contested_queries.rs:171-200` |
| (d) a masternode's current votes | `ResourceVote::fetch_many(sdk, ContestedResourceVotesGivenByIdentityQuery{ identity_id = proTxHash, .. })` | **`Sdk::get_contested_dpns_identity_votes` is a stub that always returns empty** — do not use. Votes are keyed by proTxHash | `rs-sdk/src/platform/fetch_many.rs:515`; stub `contested_queries.rs:213-232`; `rs-drive/src/query/contested_resource_votes_given_by_identity_query.rs:37,244` |
| (e) finished result | Same vote-state query; `winner: Option<(WonByIdentity/Locked/NoWinner, BlockInfo)>` | — | `rs-drive-proof-verifier/src/types.rs:163-166` |
| (f) names owned | `get_dpns_usernames_by_identity(id, limit)` (index on `records.identity`, default limit 10); pending contests: `get_non_resolved_dpns_contests_for_identity` | owner semantics follow `records.identity` (rewritten on transfer) | `queries.rs:40-89`; `contested_queries.rs:390-415` |
| register | `Sdk::register_dpns_name(RegisterDpnsNameInput)` — preorder + domain, prefund auto-set | — | `mod.rs:148-311` |

## 6. Normalization & contest detection

| Fact | Citation |
|---|---|
| Homograph: lowercase, `o→0`, `i,l→1` — `dpp::util::strings::convert_to_homograph_safe_chars` (public; DET already uses it) | `rs-dpp/src/util/strings.rs:1-13` |
| Enforced on-chain: `normalizedLabel == convert(label)` | `triggers/dpns/v1/mod.rs:121-131` |
| Label regex `^[a-zA-Z0-9][a-zA-Z0-9-]{0,61}[a-zA-Z0-9]$`, 3–63 chars; full name ≤ 253 | schema `:42-48`; `triggers/dpns/v1/mod.rs:30,108-118` |
| Contested regex lives only in the contract JSON (`contested.fieldMatches`). Callable from DET without duplication: `dpns_contract.document_type_for_name("domain")?.find_contested_index()` → `contested_index.field_matches[..].matches(value)`, or build the domain `Document` and call `document_type.prefunded_voting_balance_for_document(&doc, sdk.version())` (returns `Some((index, amount))` ⇒ contested **and** gives the exact fee) | `rs-dpp/.../versioned_methods.rs:649-681`; `rs-dpp/src/data_contract/document_type/mod.rs:202-216` |

## 7. Other UX-relevant facts

- Contest opens with a single requester; the requester sees "in contest" for 14 days (mainnet) even with no rival, then wins automatically unless a lock vote exists.
- One identity cannot submit a second contested create with the same document id (`DocumentContestDocumentWithSameIdAlreadyPresentError`); a non-contested create may not reuse a live contested id either. `rs-drive-abci/.../state_v1/mod.rs:192-220`, `state_v2/mod.rs:74-111`.
- Testnet only: structure checks skipped before epoch 2080 (historic). `advanced_structure_v1/mod.rs:52-56`.
- A contender cannot withdraw (domain Delete rejected; contested docs are not in primary storage).
- End time is block-time based (ms), not height; displayed end_time = start block time + duration.

## Contradictions with DET

1. `src/ui/identity/register_dpns_name_screen.rs:539` hard-codes "Cost ≈ 0.2006 Dash": correct for PV13 (0.2 + ~0.0006 fees), **wrong from PV14 (0.1 DASH)**. Read it from `sdk.version()` or via `prefunded_voting_balance_for_document`.
2. The 0.2/0.1 DASH is **not refundable** in any outcome, and losers keep nothing. UI copy must not call it a deposit.
3. `is_contested_name` lives in `ui/` (`register_dpns_name_screen.rs:678`) — duplicates the contract regex; placement policy wants `model/`, ideally delegating to the contract's `field_matches`.
4. If DET relies on SDK availability helpers, active-contest and locked names show as "available".
5. Masternode "my votes" via `get_contested_dpns_identity_votes` returns nothing (stub); use `ResourceVote::fetch_many` with the proTxHash.
6. Ties go to the **latest** created contender, not the first.
