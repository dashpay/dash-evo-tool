---
name: visual-diff
description: "Use to produce a visual diff of dash-evo-tool: annotated screenshots (spotlight style, one image per screen of the NEW build) showing what changed in the UI between two builds, with a commit-coverage table, for a presentation or release review."
---

# Visual diff

Two builds in, a gallery out: one annotated image per changed screen of the NEW build, a
clean variant of each, and an `index.html` that maps every commit to an image or says why none
exists. Scripts live in `scripts/` beside this file; all take paths as arguments. Do not
hard-code machine paths; the caller names the output folder (e.g. under `/data/artifacts/<project>/<date>/`).

Related: `release-review` (QA campaign, uses this for presentation images), `desktop-gui` skill
(display, a11y tree, launch), `docs/gui-testing/README.md` (safety rules). Read both before driving the GUI.

## 1. Inputs
- OLD and NEW build: release tags or commits. Release asset: `gh release download <tag> -p 'dash-evo-tool-linux-x86_64.zip'`, then `gh attestation verify <zip> -R dashpay/dash-evo-tool`. Local build: `cargo build`, then get the binary path from Cargo (`cargo metadata --format-version 1 --no-deps`, `target_directory`), never assume `./target`.
- Each build in its own directory; never run both at once (one display, unambiguous provenance).

## 2. History first
- `git log --no-merges --format='%h %s' OLD..NEW`; map every user-visible commit to the screen that shows it. That is the shot list and the coverage table.
- Also walk every reachable screen in both builds and run `scripts/diff.py`: it finds changes the history does not name. Clocks, sync progress, banners, scroll offsets, panel widths and the title-bar version are noise.

## 3. Capture (same state in both builds)
- Per build: own `DASH_EVO_DATA_DIR` (never the real config dirs), same network, view mode, data, window size. Normalise: `xdotool windowsize $WID 1260 780; xdotool windowmove $WID 10 0`. `scripts/cap.sh DIR NAME` (scrot) includes the title bar, so capture pixels = screen pixels; spec rects use these.
- Same names in `old/` and `new/` (`NN-screen.png`). Screens that exist only in NEW go in `new/` only.
- Wait for SPV "Synced" first. A red banner ("network is still syncing") before that is timing and must not be on screen, unless the banner itself is the change.
- Persisted state (last screen/tab/scroll) survives restarts; re-check the screen after relaunch.

## 4. Data (a shot without data is useless on a slide)
- Load a public identity view-only (identity ID or username, no key). Typing a PUBLIC id is fine; do not refuse it.
- Wallet-bound screens need a wallet: use the test credentials only through `scripts/type-secret.sh [--env-file F] VAR [WORD_INDEX]` (default env file: `.env` at the repo root). Allowed names: `E2E_WALLET_MNEMONIC` (word index for one-field-per-word forms), `E2E_MN_PROTX_HASH`, `E2E_MN_VOTING_KEY`, `E2E_MN_OWNER_KEY`, `E2E_IDENTITY_ID`. It types into the focused field and never prints.
- Never read, cat or grep the env file or the app's real config dirs.
- Secret protocol: locate the field and the confirm button first (coordinates before typing); click the field; run the helper; click confirm by known coordinates. While a secret field is filled take NO full capture and NO full a11y dump (only `a11y_dump.py ... --grep "<label>"`); verify with a crop of an area below the fields (`scrot -a x,y,w,h`) or pixel probes. Activate and focus the app window right before each helper call. Nothing secret in files, captions, reports or command lines; check every kept capture. Public identifiers (ProTxHash, identity ID, addresses, usernames) may appear.
- Nothing is broadcast: no send, top-up, vote, registration, profile change, contact request. Open forms and type values, stop before the final confirmation; clear staged vote choices before closing.
- Same imported data in both builds. At the end kill only your own PIDs and delete the data dirs (confirm with `ls`).

## 5. Annotate (approved "spotlight" style, do not change it)
`scripts/render.py SPEC.json --new-dir new --old-dir old --out-dir annotated` (see `scripts/example-spec.json`; rects in capture pixels, 1x). Per image a list of `{kind, rect, label, pill, inset}`:
- Unchanged area dimmed and desaturated; marked elements stay in full colour with a tight rounded outline.
- Colours: new = green (0,168,84), changed (moved, reworded, restyled) = violet (123,63,242), removed = red (229,57,53).
- Pill callout 2-5 words, "New: ...", "Changed: ...", "Removed: ..."; leader line to its outline; the pill must not cover content or the outline's label.
- removed = dashed outline where it was plus an inset cropped from the OLD capture, labelled "Before (previous build)". If NEW shows nothing there, an empty dashed box confuses: use the inset alone (small rect).
- wholly new screen: kind `screen`, no dimming, one outline around the screen, "New screen: ...".
- Key bottom-right, 2x output, labelled image in `out/`, clean variant (no pills, no insets) in `out/clean/`.
- One image per screen of the NEW build, never a before/after pair.
- Every rect comes from THAT image's own capture. A rect reused from another screen once landed on an error banner.

## 6. Verify
- `render.py` exits 1 if a new/changed rect is identical in the OLD and NEW capture (wrong rect) or a pill leaves the image. Fix and rerun.
- Then READ every labelled image: each outline encloses exactly what its label names; no pill clipped or covering content; the screen is the intended state (no banner, data present).

## 7. Gallery and report
- Write `meta.json` (see `scripts/example-meta.json`), run `scripts/gallery.py --root OUT --meta meta.json`; `old/`, `new/`, `annotated/` under OUT. Every commit row points at an image or says why not visible (backend only, needs state that was not reachable).
- Report: what was not reached and why (e.g. no open contests on testnet; do not create one); presenter notes: real faces in profile pictures, error banners, data that differs between shots (balances change during a session: data, not UI).
- The first pass without data was a mistake: load data before the first full capture round.
