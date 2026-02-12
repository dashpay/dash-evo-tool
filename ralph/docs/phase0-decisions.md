# Phase 0: Project Foundation — Decisions

## 0.1 Framework & Library Choices (Run 1)

**Framework: React 19 + TypeScript + Vite**
- Largest component ecosystem — critical for 75+ screens needing sortable tables, tree views, code viewers, drag-and-drop, QR codes, formula visualization
- Gold-standard testing: Vitest + React Testing Library + jsdom
- Tauri's official docs and examples primarily use React; `@tauri-apps/api` works naturally with React hooks
- Largest community for troubleshooting a project of this complexity

**Component Library: shadcn/ui (Radix primitives + Tailwind CSS)**
- Components are copied into the project (full ownership and customization)
- Built on Radix UI — best-in-class accessibility (WCAG 2.1 AA) with focus management, keyboard nav, ARIA labels
- Tailwind CSS theming with CSS variables — dark/light mode built-in
- Covers: tables, tabs, dialogs, forms (react-hook-form + zod), dropdowns, toasts, badges, cards
- Supplementary libraries: `@tanstack/react-table` (sortable data tables), `@tanstack/react-virtual` (virtual scrolling), `@dnd-kit` (drag-and-drop), `react-json-view-lite` (JSON viewer), `qrcode.react` (QR codes), `react-arborist` (tree views)

**State Management: Zustand**
- Minimal boilerplate, TypeScript-native, async-friendly (perfect for Tauri IPC)
- Slices pattern for domain organization (wallets, identities, tokens, etc.)
- Middleware: persistence, devtools, immer

**Build Tool: Vite**
- Tauri 2.0 recommended, fast HMR, excellent TypeScript/React support
