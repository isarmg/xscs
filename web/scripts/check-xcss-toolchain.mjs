import {prepareApplicationFonts, startAfterFonts} from "@xcss/web/web-fonts";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";

import { assertXcssWebToolchain } from "@xcss/web/web-toolchain";
import manifest from "../package.json" with { type: "json" };
assert.deepEqual(manifest.dependencies ?? {}, {});

const lock = JSON.parse(
  readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"),
);
// One exact published xcss version, with tarball integrity and no local links.

const xcssPackages = ["@xcss/web"];

const nodeVersion = readFileSync(
  new URL("../../.node-version", import.meta.url),
  "utf8",
);
assert.match(nodeVersion, /^26\.7\.0\n?$/);
assertXcssWebToolchain(manifest, nodeVersion);
for (const dependency of xcssPackages) {
  const expected = "https://github.com/isarmg/xcss/releases/download/v1.0.0/xcss-web-1.0.0.tgz";
  assert.equal(manifest.devDependencies?.[dependency], expected);
  assert.equal(lock.packages?.[""]?.devDependencies?.[dependency], expected);

  const locked = lock.packages?.[`node_modules/${dependency}`];
  assert.equal(locked?.link, undefined);
  assert.equal(locked?.resolved, expected);
  assert.equal(locked?.version, "1.0.0");
  assert.equal(locked?.integrity, "sha512-QR/zmdzyQkLnFzTTp+EHYnOL9XEe5N95XVUasnDlKqdNWw/nVoY4evq083QTxMED/ElRRYeegEItnTxCUcb7TQ==");
}
const adminStyles = readFileSync(new URL("../node_modules/@xcss/web/dist/admin-ui/styles.css", import.meta.url), "utf8");
assert.match(adminStyles, /@import ["']\.\/content-blocks\.css["']/);
for (const snapshot of ["content-blocks.css", "provenance.json", "verify.mjs"]) {
  assert.equal(existsSync(new URL(`../appearance/${snapshot}`, import.meta.url)), false, "xcss CSS must not be copied into the product");
}

assert.equal(typeof prepareApplicationFonts, "function");
assert.equal(typeof startAfterFonts, "function");

// The default production entry must execute Rust; React remains a reference only.
const entry = readFileSync(new URL("../src/main-xcss-web.js", import.meta.url), "utf8");
const index = readFileSync(new URL("../index.html", import.meta.url), "utf8");
assert.match(index, /src\/main-xcss-web\.js/);
assert.match(entry, /_web_bg\.wasm\?url/);
for (const name of ["react", "react-dom", "@vitejs/plugin-react"]) {
  assert.equal(manifest.dependencies?.[name] ?? manifest.devDependencies?.[name], undefined);
  assert.equal(lock.packages?.[`node_modules/${name}`], undefined);
}
