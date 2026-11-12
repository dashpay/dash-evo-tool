# Token tutorial source toolkit

Maintain the six-part English token tutorial here when the application changes.
This directory contains source, tests and explicit input mappings. Recordings,
validated narration, fonts, private profiles and rendered deliveries stay outside
Git. A fresh checkout can plan narration, generate branded slides and assemble
newly supplied footage; it cannot recreate live blockchain outcomes from source.

Chad G. P. Taylor uses **Cedar**; Ewa Dashwood uses **Nova**. Narration uses the
remote `gpt-audio-1.5` service; there are no local model weights. The default mode
of each narration CLI is offline. `--execute` is an explicit paid operation.
Do not execute narration merely to check this toolkit.

## Sources and provenance

| File | Role |
|---|---|
| `scenario/dialogue.json` | Canonical 54 ordered scenes, current screen/control plans, speaker turns and 12 standalone bridges. Edit this source for future tutorials. |
| `scenario/dialogue.txt` | Readable English rendering, deterministically generated from the JSON. |
| `scenario/scenario.txt` | Current non-spoken scene plan; its actual SHA-256 is recorded by the canonical JSON. |
| `scenario/current-walkthrough.md` | Reusable scene/control/help checklist, pinned source anchors, worked formulas and pending live gates. |
| `scenario/accepted-media.json` | Archival mapping of 278 old takes and the historical visual edit. Incompatible with the refreshed source; do not reuse or update its hashes. |
| `scenario/accepted-combined.en.srt` | Accepted measured captions including the three-second silent opening; reference only, never timing for new audio. |
| `scripts/` | Offline-first narration, semantic GUI capture, media assembly and regression tests. |
| `assets/` | Small official logo, editable slide sources, font license and eight emission-example recipes. No font binaries. |
| `examples/project.json` | Executable configuration for a new private edit under ignored `local/`. |

The current source targets DET `975993070d0061f506c0b6a2ec650df4ec18f555`
and Platform `575d961fe0c54bd7e01a3cf3823494e58c6c4fdb`. It is awaiting
independent review and completion of live gates; it is not approved for recording.
This refresh authorizes documentation and offline checks only: no recording,
audio generation or provider API calls. See [author notes](scenario/dialogue-author-notes.md)
and the [current walkthrough](scenario/current-walkthrough.md).

Historical dialogue SHA-256
`88e140a8c3c11a46811c081b94015eca896d51ff927180f19225e5302a712abc`
covered DET `32f8ede362bd0b4e02b641f022ad6d88b0087a48`. Its accepted media and
subtitles remain unchanged for archival provenance. Their source hashes must
reject use with the current dialogue; old approval and old footage do not verify
the refreshed application. No take reuse is authorized for this refresh.

## Dependencies and private workspace

Use the system `/usr/bin/python3`, not pip or a virtual environment. On Debian or
Ubuntu the operator can install the required packages with apt:

```sh
sudo apt install python3 python3-pytest python3-pil python3-matplotlib \
  ffmpeg xvfb openbox xdotool scrot x11-utils dbus-x11 at-spi2-core python3-pyatspi
```

Slide/chart rasterization also requires a local Chromium-compatible browser
(`google-chrome` by default; `--browser chromium` selects another executable).
Supply an external font file. For the original appearance use Montserrat variable
from the source named in `assets/ATTRIBUTION.txt`; its license is included. A
system font also works, with its actual license and publication credits updated
in the private media manifest. No network is used to render slides or charts.

Run the following from `tools/token-tutorial/`:

```sh
TOOL=$PWD
umask 077
python3 scripts/project.py prepare examples/project.json
python3 scripts/project.py transcript scenario/dialogue.json > local/dialogue.txt
```

This creates `local/media.json`, a frozen `local/scenario.json`, and empty capture,
slide and evidence directories. `prepare` refuses an existing edit. Relative
configuration paths resolve from the configuration file; media paths resolve
from the media manifest, regardless of the working directory. To use external
storage, create a private config with absolute `scenario`, `work`, and
`font_license` paths. Use a fresh work directory for each approved source revision;
keep old snapshots and journals. Set `TMPDIR` to a sufficiently large private
scratch directory before rendering; the system temporary directory is the default.

## Updating the scenario after an application change

1. Record the DET commit, actual binary SHA-256, Platform dependency revision,
   network and test date in a private production record. Inspect the live source,
   especially `src/ui/tokens/tokens_screen/creator/`,
   `src/model/token_creator/`, `src/ui/state/token_creator.rs`, and
   `src/backend_task/tokens/creator_registration.rs`. Locate renamed modules
   with `rg --files`. The seven-section workspace has no Simple/Advanced toggle.
2. Compare controls against every scene's `controls` and `screen` fields. Include
   hidden/advanced options, defaults, units, selectors, locks, validation and the
   exact final configuration. Check protocol behavior against the pinned Platform
   evaluator and validators, not just help text. The emission generator's links
   identify its original primary sources; review them for the new revision.
3. Update `dialogue.json` in place, preserving scene identifiers and the six part
   ranges unless deliberately revising the format. Only `turns`, `intro` and
   `outro` are spoken. Regenerate the readable source:

   ```sh
   python3 scripts/project.py transcript scenario/dialogue.json > scenario/dialogue.txt
   ```

4. Have a different reviewer audit text against actual source/control coverage,
   mathematics, irreversible policies and intended demonstrations. Record the
   approved scenario SHA-256 and findings outside the repo. Observe changed GUI
   outcomes before presenting them as results. Approval of an old script is not
   approval of a changed hash.
5. Prepare a fresh edit using a private config pointing to the updated source.
   This refreshed source must not reuse the accepted historical takes. Any future
   narration or reuse decision requires its own explicit authorization.

## Reuse and narration planning

All **278 historical turns already have audio** for the old source only. None is
approved for this rewritten scenario. In particular, 46 manually made
first-episode takes remain `planned` in the historical batch journal. Replaying
that journal would pay for duplicates. Preserve the explicit accepted mapping
for archival inspection only.

If the operator has the original production files, restore their `audio/`,
`assets/slides/`, `video/` and metadata directories under `local/archive/`, retaining
their relative layout. The archive mapping describes those historical inputs;
its source hash intentionally does not match the current dialogue.
No external host is required. To inspect historical provenance, preserve an old
JSON snapshot privately and point a private archive manifest at that snapshot,
retaining the original `scenario_sha256`. Do not repoint the accepted archive to
the refreshed dialogue or bypass a hash mismatch.

```sh
python3 scripts/batch_narration.py local/scenario.json --output local/narration
```

`reuse` uses the selected manifest's segment/turn mapping, not filename guesses.
It checks the old scenario hash, speaker, voice, exact text and per-turn hash,
current instructions, provider finish reason/transcript, PCM/WAV bytes and stored
checksums. It copies verified `ready/` data into a private journal and reports
rejected IDs. Changed turns remain planned; existing ready/failed attempts are
never reset. Missing raw responses or instruction provenance prevent automatic
reuse. Review the original take and recover its provenance offline, or explicitly
approve replacement of that particular turn; a filename alone is insufficient.

The batch journal is atomically saved and locked with `flock`; at most five
requests run concurrently. Re-running planning validates saved takes and skips
ready jobs. A source/helper hash change requires a fresh journal. The general
reuse tool can preserve unchanged turns when separately
authorized for a future project; it is not approved for this rewrite. The source
snapshot and per-turn checks must remain intact. Never edit a journal to turn a
failed attempt back into planned.

Inspect `selection.pending_requests`, job IDs and hashes before paid work.
The manifest summarizes known token usage and an **informative historical pricing
formula**, not a preflight quote. Before spending, verify current model access,
prices and a budget in the provider dashboard; missing response usage is unknown
cost. The default zero baseline excludes earlier production sessions.

Only after independent script approval, human listening of reusable takes, and
explicit budget approval, set `OPENAI_API_KEY` in the process environment (never
in Git, a command transcript or JSON). The single-take CLI additionally accepts
an explicit `--key-file`; it parses assignments without executing the file.
Neither tool searches implicit credential locations.

```sh
# PAID: replace both placeholders from the reviewed offline plan.
python3 scripts/batch_narration.py local/scenario.json --output local/narration \
  --ids CHANGED_JOB_ID --execute --approved-script-sha256 APPROVED_SHA256
```

The exact source hash plus `--execute` are required. There are no automatic paid
retries. An uncertain network result remains failed; inspect its saved response
and account usage first. For a complete saved response, `openai_voice.py SOURCE
NEW_TAKE --recover-from FAILED_TAKE --voice cedar` validates it offline. Recovery
never changes the old attempt; explicitly map the newly validated take into the
private media manifest after human listening. Only when another charge is
approved use `--retry-failed EXACT_FAILED_ID --execute
--approved-script-sha256 APPROVED_SHA256`. A different directory is not permission
to repeat an uncertain request. Retain all journals and raw responses privately.

## Branded slides and numerical examples

```sh
FONT=/absolute/path/to/Montserrat-variable.ttf
python3 assets/slides/render-slides.py --font "$FONT" --output local/slides
python3 assets/emission-examples/generate.py --font "$FONT" --output local/emission-examples
```

Outputs must be empty. Add `--browser chromium` when appropriate. Browser
sandboxing stays enabled by default; `--no-sandbox` is available only when an
operator deliberately uses an isolated restricted desktop. Generated SVGs refer
to the external font; generated PNGs are standalone. The opening includes the AI
mistake disclosure. It precedes narration with **three seconds of silence in every
standalone episode and the combined package**.

Scenes 01/02/50 have small editable editorial SVGs. Update these when their script
or final policies change. The eight emission charts are explanatory calculations,
not a live Platform evaluator: re-audit recipe parameters and rounding against the
new Platform revision. Show charts alongside actual recorded controls and identify
editorial explanations visibly; never replace a failed operation with a claimed
success. Captures can include these charts via an operator-reviewed local edit,
or point a whole explanatory scene's visual at its generated native Full HD PNG.

## Record the actual application

Read [the GUI testing rules](../../docs/gui-testing/README.md) first. Build or
supply the reviewed app; Cargo's `metadata --format-version 1 --no-deps` reports
its actual `target_directory`. Keep a new private profile outside public assets.
Import/unlock keys while recording is stopped. Never capture a recovery phrase,
private key, password, credential dialog, environment file or live user database.

Run a checked, isolated launcher in one terminal:

```sh
BINARY=/absolute/path/to/dash-evo-tool
PROFILE=/absolute/private/path/to/tutorial-profile
BINARY_SHA256=$(sha256sum "$BINARY" | cut -d ' ' -f1)
python3 scripts/gui/launch.py --check --binary "$BINARY" \
  --binary-sha256 "$BINARY_SHA256" --profile "$PROFILE" \
  --config ../../.env.example --evidence "$TOOL/local/evidence"
dbus-run-session -- python3 scripts/gui/launch.py --binary "$BINARY" \
  --binary-sha256 "$BINARY_SHA256" --profile "$PROFILE" \
  --config ../../.env.example --evidence "$TOOL/local/evidence"
```

The launcher reserves a new local X display (default `:100`), verifies the binary,
refuses unrelated nonempty profiles and writes a private session manifest. Verify
fullscreen/capture bounds manually before filming. It terminates only processes
it spawned. Do not recompute the expected hash after an unreviewed binary change.

In another terminal, list observed semantic labels, then author a small scene
JSON using `scripts/gui/smoke.json` as the structural example. Its labels assume
an empty test profile and must be replaced with the observed current labels.

```sh
python3 scripts/gui/gui.py --session local/evidence/session.json list
python3 scripts/gui/gui.py scene scripts/gui/smoke.json
python3 scripts/gui/gui.py --session local/evidence/session.json \
  --evidence local/evidence scene local/scene-03.json \
  --execute --record local/captures/03.mkv
```

Scene dry-run reads JSON without touching a GUI. Supported steps are exact-label
`wait`, `wait_absent`, `click`, `screenshot` and bounded `hold`. Capture uses real
Full HD X11 frames. Missing/ambiguous accessibility state stops the action;
absence cannot succeed after losing the app. The helper intentionally does not
automate secret entry. Prepare fields manually with recording stopped as needed.

For a future separately authorized recording, use scenes 03–49 and 51–52 from
the current JSON's screen/control plan. Use real
completed-state evidence for scenes 08/09 and 51/52. **Never repeat token
registration or any irreversible action merely to re-record it.** Preserve the
original raw capture and registration journal; after an uncertain result verify
the public contract before another attempt. Record a current read-only result
view when available. Historical captures are
archival context only and must not stand in for an unobserved current result.
Read-only/private test
preparation is separate from any approved capped Testnet fund operation.

`local/media.json` is the capture/edit manifest: supply the real `path`, optional
`offset` and `hold_last_frame` for each visual. A too-short clip fails unless a
real last-frame hold is explicitly allowed. The renderer does not invent footage,
click timing or outcome states. For multi-shot explanations, compose real clips
with an external editor/ffmpeg into a reviewed scene file and set that path.
The historical edit includes manual zooms, cuts and captions; this toolkit does
not promise bit-identical recreation of those edits without their original
composed clips. The fresh capture manifest is an executable path to a new edit.

## Assemble, verify and export

Once every selected turn is ready and listened to:

```sh
python3 scripts/media/media.py import-audio local/media.json \
  local/narration/manifest.json local/media-with-audio.json
python3 scripts/media/media.py plan local/media-with-audio.json
python3 scripts/media/media.py build local/media-with-audio.json local/delivery
```

For a partial episode, append `--part 1` to import/plan/build and use a fresh
output directory. Full build creates six standalone MP4s plus a combined MP4.
Standalone episodes include their bridges; combined includes only scenes 00–53.
It measures WAV frame counts, renders at 30 fps, concatenates PCM, and encodes AAC
once per final package. Caption chunks preserve speaker labels and all words;
within-turn timing is proportional to word counts, not forced word alignment.
SRT, VTT, per-package transcripts/timelines, chapters, credits, checksums and a
small local HTML player are exported under `local/delivery/ready/`.
The input manifest and ffmpeg log remain private in the parent
`local/delivery/` directory; publish or serve only its reviewed `ready/` child.

Missing inputs or stale hashes stop validation before rendering. Repair the
specific manifest path or restore its source/metadata; do not suppress validation.
A failed render remains `pending/`; inspect its ffmpeg log and build into a fresh
output directory. Only `ready/` is a completed technical export. Keep private
manifests/journals and raw API responses out of a public download.

Run the offline regression suite and the synthetic render check (no API or GUI):

```sh
python3 -m pytest scripts -q
python3 scripts/media/integration_check.py --part1
```

The integration check prints its temporary output/proof path; `TMPDIR` controls
placement. Without `--part1` it verifies all seven packages, including exact
main-scene coverage. It checks streams, actual durations, chapters, bounded
speaker captions, embedded SRT parity and the silent lead. AAC may produce a
boundary pre-echo up to 32/32768 amplitude; the first 2.98 seconds must be exactly
zero. These synthetic checks do not validate real GUI behavior or human audio
quality. Listen/watch every final package and verify no secrets or misleading
states appear before publication.

For local preview, serve only the reviewed `ready/` directory with
`python3 -m http.server --bind 127.0.0.1 8000 --directory local/delivery/ready`.
Check the optional player's chapters/caption selection in a browser. External
hosting/export is a separate publication decision; include the AI disclosure,
Testnet context, logo/font credits and license links with published media.
