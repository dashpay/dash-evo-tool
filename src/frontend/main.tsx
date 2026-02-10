import React from "react";
import ReactDOM from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { ThemeProvider } from "@/components/theme";
import { TooltipProvider } from "@/components/ui/tooltip";
import { router } from "./routes";
import "./index.css";

async function main() {
  // When running in E2E mock mode, initialize the browser-side IPC mocks
  // before rendering the app. This allows Playwright tests to configure
  // mock responses via window.__E2E_MOCK_IPC__.
  if (import.meta.env.VITE_E2E_MOCK === "true") {
    const { initE2EMockIPC } = await import("./e2e-mock-ipc");
    initE2EMockIPC();
  }

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <ThemeProvider defaultTheme="system">
        <TooltipProvider>
          <RouterProvider router={router} />
        </TooltipProvider>
      </ThemeProvider>
    </React.StrictMode>,
  );
}

main();
