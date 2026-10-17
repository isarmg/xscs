import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";

const web = new URL("../", import.meta.url);
const read = path => readFileSync(new URL(path, web), "utf8");
const manifest = JSON.parse(read("package.json"));
const lock = JSON.parse(read("package-lock.json"));
assert.deepEqual(manifest.dependencies ?? {}, {});
assert.deepEqual(lock.packages[""].devDependencies, manifest.devDependencies);
assert.match(read("../.node-version"), /^26\.7\.0\n?$/);

// Test the same checked-in assets that the Rust builder ships, never a second
// npm copy with a different version or retired React/Vite peer dependencies.
for (const name of ["@xcss/web", "react", "react-dom", "@vitejs/plugin-react", "vite", "typescript"]) {
  assert.equal(manifest.dependencies?.[name] ?? manifest.devDependencies?.[name], undefined, name);
  assert.equal(lock.packages[`node_modules/${name}`], undefined, name);
}
for (const directory of ["vendor/xcss", "xcss-web/vendor/xcss-web"]) {
  const source = JSON.parse(read(`${directory}/SOURCE.json`));
  assert.equal(source.format, 1);
  assert.ok(Object.keys(source.files).length > 0);
  for (const [path, digest] of Object.entries(source.files)) {
    assert.match(digest, /^[a-f0-9]{64}$/, path);
    const actual = createHash("sha256").update(readFileSync(new URL(`${directory}/${path}`, web))).digest("hex");
    assert.equal(actual, digest, `${directory}/${path}: resync from the recorded public source`);
  }
}
assert.match(read("rust-web.toml"), /"@xcss\/web\/"\s*=\s*"vendor\/xcss\/"/);
assert.match(read("vendor/xcss/admin-ui/styles.css"), /@import ["']\.\/content-blocks\.css["']/);
for (const snapshot of ["content-blocks.css", "provenance.json", "verify.mjs"]) {
  assert.equal(existsSync(new URL(`appearance/${snapshot}`, web)), false, "use the verified vendor snapshot");
}
const entryPath = read("rust-web.toml").match(/^entry = "([^"]+)"/m)?.[1];
assert.ok(entryPath, "Rust builder must declare its entry");
assert.ok(read("index.html").includes(`src="/${entryPath}"`), "HTML must use the Rust builder entry");
assert.doesNotMatch(read("index.html"), /node_modules/, "font preloads must use the vendored production assets");
const entry = read(entryPath);
assert.match(entry, /_web_bg\.wasm\?url/);
assert.match(entry, /startApplication/);
assert.match(read("xcss-web/vendor/xcss-web/startup.js"), /export function startApplication/);
for (const file of readdirSync(new URL("tests/", web)).filter(file => file.endsWith(".mjs"))) {
  assert.doesNotMatch(read(`tests/${file}`), /^(?:import|export)\s.*?["']@xcss\/web|import\s*\(\s*["']@xcss\/web/m, `${file}: test shipped vendor assets`);
}
console.log("Rust/WASM entry, verified public snapshots and browser-only npm dependencies passed");
