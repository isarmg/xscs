import { chromium, firefox } from "@playwright/test";
import { preview } from "../../tools/web-build/test-preview.mjs";
import { verifyStartupFailures, verifyStartupTimeout } from "../xcss-web/vendor/xcss-web/tests/startup-failures.mjs";

const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const url = new URL("?lang=en", server.resolvedUrls.local[0]).href;
      const prepare = page => page.route("**/api/**", route => route.fulfill({
        status: 401, contentType: "application/problem+json",
        body: JSON.stringify({ status: 401, code: "auth.required" }),
      }));
      await verifyStartupFailures(browser, url, prepare);
      await verifyStartupTimeout(browser, url, prepare);
    } finally { await browser.close(); }
  }
} finally { await server.close(); }
