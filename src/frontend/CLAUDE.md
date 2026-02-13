# CLAUDE.md

## Migration Context

This is the **Tauri + React frontend** for Dash Evo Tool, being migrated from an egui (Rust immediate-mode GUI) frontend. The egui screens in `src/ui/` remain as the **authoritative reference** for expected behavior, screen flows, and business logic. When investigating bugs or implementing features, compare against the egui implementation.

## Commands

```bash
# Frontend development
npm run dev                     # Vite dev server (port 1420)
npm run build                   # Vite production build
npm run lint                    # ESLint
npm run typecheck               # TypeScript check (tsconfig.app.json)
npm run format:check            # Prettier check

# Testing
npm run test                    # Vitest run (all tests)
npm run test:watch              # Vitest watch mode
npx vitest run path/to/file     # Run a single test file
npm run test:e2e                # Playwright E2E (chromium + webkit)
npm run test:e2e-integration    # E2E with mock IPC (VITE_E2E_MOCK=true)

# Rust backend (from repo root)
cargo build                     # Debug build
cargo clippy --all-features --all-targets -- -D warnings
cargo +nightly fmt --all
```

## Architecture

**Stack:** React 19 + TypeScript + Vite + Tauri 2 IPC + Zustand + TanStack Router + Tailwind CSS 4 + shadcn/ui (Radix)

**Entry flow:** `main.tsx` → `ThemeProvider` → `TooltipProvider` → `RouterProvider` → `AppLayout` (sidebar + topbar + content) → lazy-loaded screens

**Path alias:** `@/*` maps to `src/frontend/*`

### Tauri IPC (bindings.ts)

`bindings.ts` is **auto-generated** by tauri-specta — never edit it manually. It exports:
- `commands` — 180+ typed async functions for calling the Rust backend
- `events` — typed event listeners (`taskResultEvent`, `taskErrorEvent`, `walletUpdatedEvent`, etc.)

**Two command patterns:**
1. **Task dispatch:** `commands.identityLoad(input)` → returns `{ taskId }` → listen for `events.taskResultEvent` with matching `taskId`
2. **Direct return:** `commands.getNetworkInfo()` → immediate typed response

### State Management (Zustand)

8 stores in `stores/`: `walletStore`, `identityStore`, `contractStore`, `documentStore`, `tokenStore`, `contestStore`, `dashpayStore`, plus `index.ts` barrel export.

Each store:
- Defines `State` + `Actions` interfaces, exports combined type
- Uses `commands.*` for backend calls
- Wraps task dispatch with `TaskTimeoutManager` (auto-rejects after 30s)
- Listens for `events.taskResultEvent` / `events.taskErrorEvent`

### Routing

TanStack React Router with lazy-loaded screens via `React.lazy()` in `routes.tsx`. 50+ screens organized in `screens/` — each exported as a named export (not default).

### Amount Formatting (Critical)

- **Wallet balances** are in **duffs** (1 DASH = 100,000,000 duffs) → use `formatAmount(duffs, 8)`
- **Platform/Identity balances** are in **credits** (1 DASH = 100,000,000,000 credits) → use `formatCreditsAsDash(credits)` from `AmountInput.tsx`
- 1 duff = 1,000 credits (`CREDITS_PER_DUFF`)
- **Never** use `formatAmount(credits, 8)` for Platform credits — it will be 1000x off

### Key Type

`WalletRefDto` is a discriminated union: `{ type: "hd"; seedHash: string } | { type: "singleKey"; keyHash: string }`

## Testing

**Framework:** Vitest + jsdom + @testing-library/react + @testing-library/user-event

**Test infrastructure in `test/`:**
- `mock-ipc.ts` — `createMockBindings(overrides)` + `mockBindingsModule()` for mocking all 180+ IPC commands
- `render-helpers.tsx` — `renderWithProviders()` (wraps in ThemeProvider + TooltipProvider), `createRouterMock()`, `createThemeMock()`
- `fixtures/` — factory functions: `createMockHdWallet()`, `createMockIdentity()`, `createMockToken()`, etc.
- `setup.ts` — jsdom polyfills (ResizeObserver, matchMedia, pointer capture)

**Screen test pattern:**
```typescript
const { mockNavigate } = vi.hoisted(() => ({ mockNavigate: vi.fn() }));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => ({}),
}));
vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import("@/test/mock-ipc");
  return mockBindingsModule(createMockBindings({
    walletListAll: vi.fn().mockResolvedValue({ status: "ok", data: { hdWallets: [], singleKeyWallets: [], selected: null } }),
  }));
});
```

**jsdom quirk:** Radix DropdownMenu `onSelect` doesn't fire from `user.click()` — only `onClick` works. Dropdown close animation can steal focus from newly mounted elements; fix with delayed `setTimeout(() => ref.focus(), 10)`.

## Component Organization

- `components/ui/` — shadcn/ui primitives (Radix-based), managed by shadcn CLI
- `components/shared/` — cross-domain reusable components (AmountInput, IdentitySelector, CopyButton, WalletUnlockDialog, etc.)
- `components/{wallet,identity,token,dpns,dashpay,contract}/` — domain-specific components
- `components/{feedback,layout,navigation,theme}/` — infrastructure components
- `screens/` — full-page screen components (named exports, lazy-loaded)

## Key Utilities

- `lib/utils.ts` — `cn()` (clsx + tailwind-merge), `waitForTask()`, `hexToBase58()`, `displayId()`
- `lib/errorTranslation.ts` — maps backend error strings to user-friendly messages
- `lib/toastError.ts` — `toastError(error)` for toast notifications (Sonner)
- `lib/taskTimeout.ts` — `TaskTimeoutManager` class used by stores
- `hooks/` — `useUtxoMonitor`, `useTaskListener`, `useKeyboardShortcuts`, `useFrozenIdentities`
