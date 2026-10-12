import {prepareApplicationFonts, startAfterFonts} from "@xcss/web/web-fonts";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";

import { assertXcssWebToolchain } from "@xcss/web/web-toolchain";
import manifest from "../package.json" with { type: "json" };

const lock = JSON.parse(
  readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"),
);
// One exact published xcss version, with tarball integrity and no local links.

assert.deepEqual(Object.keys(manifest.dependencies ?? {}).filter(name => name.startsWith("@xcss/")), ["@xcss/web"]);
const xcssPackages = ["@xcss/web"];

const nodeVersion = readFileSync(
  new URL("../../.node-version", import.meta.url),
  "utf8",
);
assert.match(nodeVersion, /^26\.7\.0\n?$/);
assertXcssWebToolchain(manifest, nodeVersion);
for (const dependency of xcssPackages) {
  const expected = "https://github.com/isarmg/xcss/releases/download/v1.0.8/xcss-web-1.0.8.tgz";
  assert.equal(manifest.dependencies?.[dependency], expected);
  assert.equal(lock.packages?.[""]?.dependencies?.[dependency], expected);

  const locked = lock.packages?.[`node_modules/${dependency}`];
  assert.equal(locked?.link, undefined);
  assert.equal(locked?.resolved, expected);
  assert.equal(locked?.version, "1.0.8");
  assert.match(locked?.integrity ?? "", /^sha512-[A-Za-z0-9+/]+={0,2}$/);
}
const adminStyles = readFileSync(new URL("../node_modules/@xcss/web/dist/admin-ui/styles.css", import.meta.url), "utf8");
assert.match(adminStyles, /@import ["']\.\/content-blocks\.css["']/);
for (const snapshot of ["content-blocks.css", "provenance.json", "verify.mjs"]) {
  assert.equal(existsSync(new URL(`../appearance/${snapshot}`, import.meta.url)), false, "xcss CSS must not be copied into the product");
}

assert.equal(typeof prepareApplicationFonts, "function");
assert.equal(typeof startAfterFonts, "function");
