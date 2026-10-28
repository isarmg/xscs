import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { expect } from "@playwright/test";

// Exercise the actual WASM/font loader, including recovery by a user action.
// Consumers supply their real built page and any API fixtures it needs.
export async function verifyStartupFailures(browser, url, preparePage = async () => {}, contextOptions = {}) {
  for (const resource of ["wasm", "invalid-wasm"]) {
    const context = await browser.newContext({ ...contextOptions, locale: "en-US" });
    try {
      const page = await context.newPage();
      await preparePage(page);
      let documents = 0, blocked = 0;
      page.on("request", request => { if (request.isNavigationRequest() && request.frame() === page.mainFrame()) documents++; });
      const pattern = /\.wasm(?:\?|$)/u;
      const reject = route => {
        blocked++;
        return route.fulfill(resource === "invalid-wasm"
          ? { status: 200, contentType: "application/wasm", body: "invalid wasm fixture" }
          : { status: 503, contentType: "text/plain", body: "resource unavailable fixture" });
      };
      await page.route(pattern, reject);
      await page.goto(url, { waitUntil: "domcontentloaded" });
      const panel = page.locator("#xcss-startup-error");
      await expect(panel).toBeVisible({ timeout: 30_000 });
      await expect(panel).toHaveAttribute("role", "alert");
      await expect(panel).toBeFocused();
      await expect(page.locator("html")).toHaveAttribute("data-xcss-startup", "failed");
      await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "failed");
      assert.equal(await page.locator("html").getAttribute("data-xcss-runtime"), null);
      assert.equal(await panel.evaluate(node => getComputedStyle(node).visibility), "visible");
      assert.ok(blocked > 0, `The ${resource} fault must reach the actual network request`);
      const failedRequests = blocked;
      await page.clock.install();
      await page.clock.runFor(120_000);
      assert.equal(blocked, failedRequests, "Startup must not retry resources automatically");
      assert.equal(documents, 1, "Startup must not refresh automatically");
      await expect(panel).toHaveCount(1);
      await page.unroute(pattern, reject);
      await page.clock.resume();
      await panel.getByRole("button", { name: "Reload page", exact: true }).focus();
      await panel.getByRole("button", { name: "Reload page", exact: true }).press("Enter");
      await expect(page.locator("html")).toHaveAttribute("data-xcss-startup", "ready", { timeout: 30_000 });
      await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "ready");
      await expect(page.locator("html")).toHaveAttribute("data-xcss-runtime", "xcss-web");
      await expect(panel).toHaveCount(0);
      assert.equal(documents, 2, "Exactly one explicit retry should reload the page");
      console.log(`${browser.browserType().name()}: ${resource} failure visible; no automatic retries; keyboard reload recovered`);
    } finally { await context.close(); }
  }
}

export async function verifyStartupTimeout(browser, url, preparePage = async () => {}, contextOptions = {}) {
  const context = await browser.newContext({ ...contextOptions, locale: "en-US" });
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  try {
    const page = await context.newPage();
    await preparePage(page);
    await page.clock.install();
    await page.route(/\.wasm(?:\?|$)/u, async route => { await gate; await route.continue(); });
    await page.goto(url, { waitUntil: "commit" });
    await expect(page.locator("html")).toHaveAttribute("data-xcss-startup", "pending", { timeout: 30_000 });
    await page.clock.runFor(30_001);
    await expect(page.locator("#xcss-startup-error")).toBeVisible();
    await expect(page.locator("#xcss-startup-error")).toHaveAttribute("data-stage", "timeout");
    release();
    await page.clock.runFor(1_000);
    await expect(page.locator("html")).toHaveAttribute("data-xcss-startup", "failed");
    await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "failed");
    assert.equal(await page.locator("html").getAttribute("data-xcss-runtime"), null);
    console.log(`${browser.browserType().name()}: bounded WASM wait; late completion preserves the visible failure`);
  } finally { release(); await context.close(); }
}


// Font delivery is optional enhancement work. Exercise the real browser font
// requests and Rust mount, rather than mocking FontFaceSet or production hooks.
export async function verifyOptionalFonts(browser, url, preparePage = async () => {}, contextOptions = {}) {
  for (const scenario of ["unavailable", "delayed", "pagehide", "bfcache"]) {
    const context = await browser.newContext({ ...contextOptions, locale: "en-US" });
    let release;
    const gate = new Promise(resolveGate => { release = resolveGate; });
    try {
      const page = await context.newPage();
      await preparePage(page);
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      let fontRequests = 0;
      await page.route(/\.woff2(?:\?|$)/u, async route => {
        fontRequests++;
        if (scenario === "unavailable") return route.fulfill({status: 503, contentType: "text/plain", body: "optional font unavailable fixture"});
        await gate;
        await route.continue();
      });
      const started = performance.now();
      await page.goto(url, {waitUntil: "domcontentloaded"});
      await expect(page.locator("html")).toHaveAttribute("data-xcss-startup", "ready", {timeout: 15_000});
      const functionalReadyMs = performance.now() - started;
      await expect(page.locator("html")).toHaveAttribute("data-xcss-runtime", "xcss-web");
      await expect(page.locator("#xcss-startup-error")).toHaveCount(0);
      const input = page.locator('input[name="username"]');
      await expect(input).toBeVisible();
      await expect(input).toBeEnabled();
      await input.fill("font-startup-probe");
      await expect(input).toHaveValue("font-startup-probe");
      assert.ok(fontRequests > 0, "The font fault must reach real network requests");
      if (scenario === "unavailable") {
        await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "failed");
      } else {
        await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "pending");
        if (scenario === "delayed") {
          await page.clock.install();
          await page.clock.runFor(60_000);
          await expect(page.locator("html")).toHaveAttribute("data-xcss-startup", "ready");
          await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "pending");
          await expect(input).toBeVisible();
          await page.clock.resume();
        }
        if (scenario === "pagehide" || scenario === "bfcache") {
          await page.evaluate(persisted => {
            window.__fontMutations = [];
            const observer = new MutationObserver(records => {
              window.__fontMutations.push(...records.map(record => record.attributeName));
            });
            observer.observe(document.documentElement, {attributes: true, attributeFilter: ["data-xcss-fonts"]});
            window.dispatchEvent(new PageTransitionEvent("pagehide", {persisted}));
          }, scenario === "bfcache");
        }
        const fontsDelivered = page.waitForResponse(response => /\.woff2(?:\?|$)/u.test(new URL(response.url()).pathname));
        release();
        await fontsDelivered;
        await page.evaluate(() => document.fonts.ready);
        if (scenario === "pagehide") {
          await page.waitForTimeout(100);
          await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "pending");
          assert.deepEqual(await page.evaluate(() => window.__fontMutations), [], "Disposed document receives no late font state writes");
        } else {
          await expect(page.locator("html")).toHaveAttribute("data-xcss-fonts", "ready");
          await expect(page.locator("html")).toHaveAttribute("data-xcss-startup", "ready");
          await expect(page.locator(".xcss-auth-card")).toHaveAttribute("data-xcss-server-corner-profile", "zxs-g4");
        }
      }
      assert.deepEqual(errors, []);
      if (process.env.XCSS_BROWSER_EVIDENCE_DIR) {
        const directory = process.env.XCSS_BROWSER_EVIDENCE_DIR;
        await mkdir(directory, {recursive: true});
        await page.screenshot({path: resolve(directory, `${browser.browserType().name()}-font-${scenario}.png`), fullPage: true});
      }
      console.log(JSON.stringify({engine: browser.browserType().name(), scenario, functionalReadyMs: Math.round(functionalReadyMs), fontRequests, functionalReady: true, errors}));
    } finally { release(); await context.close(); }
  }
}
