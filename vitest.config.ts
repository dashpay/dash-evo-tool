import path from "path";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src/frontend"),
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/frontend/test/setup.ts"],
    include: ["src/frontend/**/*.{test,spec}.{ts,tsx}"],
    css: true,
    passWithNoTests: true,
  },
});
