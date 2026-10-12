import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Pin the readiness selector: a completed native fallback is usable, while
// unprocessed and not-visible panels must still wait for the renderer.
test('dropdown readiness accepts completed fallback without exposing initial or closed panels', () => {
  const css = readFileSync(new URL('../src/sunshine.css', import.meta.url), 'utf8');
  const rule = css.split('\n').find(line => line.startsWith('.sunshine-config-select-options:not('));
  assert.equal(rule, '.sunshine-config-select-options:not([data-xcss-server-corner-state="clip"]):not([data-xcss-server-corner-state="fallback"]:not([data-xcss-server-corner-reason="not-visible"])) { visibility: hidden; }');
  assert.match(css, /@media \(forced-colors: active\)\s*\{\s*\.sunshine-config-select-options:not\(\[data-xcss-server-corner-state="clip"\]\) \{ visibility: visible; \}/);
});
