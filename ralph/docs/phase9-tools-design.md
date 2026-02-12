# Phase 9: Tools Screens — Design

## 9.1 Tools Screens UX Design (Run 88)

### Tools Screen Inventory — 9 Screens, ~85 User Actions

All Tauri IPC commands already wired: platform_info (8), mnlist (5), grovestark (2), broadcast_state_transition (1). Routes scaffolded as placeholders. Only missing backend: `proof_log` (SQLite read).

### Architecture: Tools Landing + Sub-Tool Navigation

- Tools index (`/tools`) shows a grid of tool cards (icon + name + description)
- Each tool is a sub-route (`/tools/platform-info`, `/tools/address-balance`, etc.)
- Tools grouped into 3 categories:
  1. **Network Info** — Platform Info, Address Balance
  2. **Data Visualizers** — Contract Visualizer, Document Visualizer, Proof Visualizer, Transition Visualizer
  3. **Advanced** — Proof Log, Masternode List Diff, GroveSTARK

### UX Improvements over egui

- **Platform Info:** Card grid of query types; results in collapsible panel with copy; loading skeleton
- **Address Balance:** Single-card form with inline validation, dual Dash/credits display
- **Visualizers:** Unified layout — top: input textarea with format auto-detection badge (hex/base64/CSV), bottom: scrollable JSON output with syntax highlighting
- **Transition Visualizer:** Clickable contract IDs, broadcast button with elapsed time
- **Proof Log:** Full data table with sortable columns, pagination, detail panel with display mode tabs (Hex/JSON/PathQuery); gold hash highlighting
- **Masternode List Diff:** Tab-based (Core Items / QR Info / Quorum Viewer) with resizable split panels; file open/save via Tauri dialog API
- **GroveSTARK:** Two-mode UI (Generate/Verify) with stepper progress; research warning banner

### Shared Components Needed

- `HexInput` — multiline textarea with format auto-detection and decode-to-bytes
- `MonospaceOutput` — scrollable, selectable monospace text with optional syntax highlighting
- `ToolPageLayout` — consistent page layout (back nav + title + content area)

### Missing Backend Work

- Need Tauri command for `proof_log` — thin wrapper around `db.get_proof_log_items()`
- Visualizer parsing (contract/document/proof) done in Rust via new Tauri commands
