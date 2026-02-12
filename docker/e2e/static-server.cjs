/**
 * Minimal static file server for serving the Vite-built frontend.
 *
 * Tauri debug builds use devUrl (http://localhost:1420) instead of
 * embedding the dist/ directory. This server fills that role in Docker.
 *
 * Usage: PORT=1420 node docker/e2e/static-server.cjs
 */

const http = require("http");
const fs = require("fs");
const path = require("path");

const PORT = parseInt(process.env.PORT || "1420", 10);
const DIST_DIR = path.resolve(process.env.DIST_DIR || "dist");

const MIME_TYPES = {
  ".html": "text/html",
  ".js": "application/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
};

http
  .createServer((req, res) => {
    const url = req.url.split("?")[0]; // Strip query params
    let filePath = path.join(DIST_DIR, url === "/" ? "index.html" : url);

    // SPA fallback: serve index.html for routes without file extensions
    if (!path.extname(filePath) || !fs.existsSync(filePath)) {
      filePath = path.join(DIST_DIR, "index.html");
    }

    fs.readFile(filePath, (err, data) => {
      if (err) {
        res.writeHead(404);
        res.end("Not found");
        return;
      }
      const ext = path.extname(filePath);
      res.writeHead(200, {
        "Content-Type": MIME_TYPES[ext] || "application/octet-stream",
      });
      res.end(data);
    });
  })
  .listen(PORT, "0.0.0.0", () => {
    console.log(`Static server listening on http://0.0.0.0:${PORT}`);
    console.log(`Serving files from ${DIST_DIR}`);
  });
