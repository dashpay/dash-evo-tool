# Dialogue maintenance notes

`dialogue.json` is the canonical English spoken source; `dialogue.txt` is its deterministic readable rendering. `scenario.txt` is the current non-spoken scene plan, with a real SHA-256 referenced by the JSON. It is no longer a contradictory legacy narration. `current-walkthrough.md` maps every scene and help topic to source and verification gates.

The cast remains Chad G. P. Taylor (**Cedar**) and Ewa Dashwood (**Nova**). Keep a natural teacher/learner conversation, a hello and introduction, light self-deprecating humor, an AI-generated presentation disclosure and three seconds of opening silence. Only speaker turns and standalone bridges are narrated.

The source targets DET `975993070d0061f506c0b6a2ec650df4ec18f555` and Platform `575d961fe0c54bd7e01a3cf3823494e58c6c4fdb`, verified from that DET revision's Cargo.lock. It has 54 scenes, IDs 00–53, with exact ranges 00–09, 10–18, 19–29, 30–44, 45–49 and 50–53. Turn counts may change; identifiers and ranges remain the toolkit contract. The current source is awaiting independent review and completion of live gates, not approved for recording.

The current UI is seven sections, not a Simple/Advanced toggle. Two explicit example comparisons and five protocol-template comparisons have different authority semantics. Names are explicitly entered; unit switches preserve amounts; optional blank and zero differ. All 13 rules retain their performer, administrator and three allowances. Main-group replacement is separate.

Eight emission recipes use tokens at two decimals for amount fields and raw calculation coefficients for a/d/m/n/o/s. Keep the raw output arrays, period-zero caveat, first-five cumulative sums and current-parameter preview aligned with the pinned evaluator. A year is 365 days. Scheduled distributions use stable absolute local dates/times; duplicate phases or allocations are errors.

Review preparation is free of creation payment but requires network readiness and a usable issuer/key. Prepared IDs are the intended signing IDs, not accepted IDs. Create token and pay fees is the paid confirmation; no extra confirmation modal is promised. Registration charge excludes additional processing/storage charges. Unknown outcomes remain blocked until reconciled; status checks and receipt restoration do not create another token.

TODO: complete and link independent text/source review and coordinator GUI evidence before changing approval status. TODO: separately observe funded creation, receipts and network recovery branches before narrating any as completed. No audio generation, recording or provider API call is authorized by this rewrite.

`accepted-media.json` and `accepted-combined.en.srt` are immutable historical artifacts for dialogue SHA-256 `88e140a8c3c11a46811c081b94015eca896d51ff927180f19225e5302a712abc` (DET `32f8ede362bd0b4e02b641f022ad6d88b0087a48`). Their takes, timing and approval do not apply to the refreshed text. Do not update their source hashes, relabel old captions, or reuse historical takes for this refresh. The stale-hash guard must reject their direct use with current dialogue. An archived old source snapshot may be used only to inspect historical provenance, never as evidence of current UI behavior.
