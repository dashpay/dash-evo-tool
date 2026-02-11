import path from "path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src/frontend"),
    },
  },

  // Prevent Vite from obscuring Rust errors
  clearScreen: false,

  // Tauri expects a fixed port; fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          // Core React runtime — loaded on every page
          "vendor-react": ["react", "react-dom"],
          // Routing + state management — loaded on every page
          "vendor-router": [
            "@tanstack/react-router",
            "zustand",
          ],
          // Radix UI primitives (used by shadcn/ui components)
          "vendor-radix": [
            "@radix-ui/react-dialog",
            "@radix-ui/react-dropdown-menu",
            "@radix-ui/react-select",
            "@radix-ui/react-tabs",
            "@radix-ui/react-tooltip",
            "@radix-ui/react-popover",
            "@radix-ui/react-separator",
            "@radix-ui/react-label",
            "@radix-ui/react-slot",
            "@radix-ui/react-alert-dialog",
            "@radix-ui/react-checkbox",
            "@radix-ui/react-scroll-area",
            "@radix-ui/react-switch",
            "@radix-ui/react-collapsible",
          ],
          // Data table — used by several screens
          "vendor-table": ["@tanstack/react-table", "@tanstack/react-virtual"],
          // Drag-and-drop — used by identity/token reordering
          "vendor-dnd": ["@dnd-kit/core", "@dnd-kit/sortable", "@dnd-kit/utilities"],
          // QR code + JSON viewer — used by specific screens
          "vendor-display": ["qrcode.react", "react-json-view-lite"],
          // Tauri API — IPC bindings
          "vendor-tauri": ["@tauri-apps/api"],
        },
      },
    },
  },
});
