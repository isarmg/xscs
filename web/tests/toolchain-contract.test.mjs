import assert from 'node:assert/strict';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

const web = new URL('../', import.meta.url);
function fixture(run) {
  const root = mkdtempSync(join(tmpdir(), 'web-toolchain-'));
  const directory = join(root, 'web');
  try {
    for (const file of ['package.json', 'package-lock.json', 'index.html', 'rust-web.toml', 'scripts/check-xcss-toolchain.mjs']) {
      cpSync(new URL(file, web), join(directory, file), { recursive: true });
    }
    cpSync(new URL('../../.node-version', import.meta.url), join(root, '.node-version'));
    // Read-only production snapshots: never modify the linked paths in tests.
    for (const path of ['vendor', 'xcss-web', 'src']) symlinkSync(new URL(path, web).pathname, join(directory, path), 'dir');
    cpSync(new URL('tests/startup-failure.mjs', web), join(directory, 'tests/fixture.mjs'), { recursive: true });
    const check = () => spawnSync(process.execPath, [join(directory, 'scripts/check-xcss-toolchain.mjs')], { encoding: 'utf8' });
    const json = (file, change) => {
      const path = join(directory, file), value = JSON.parse(readFileSync(path, 'utf8'));
      change(value); writeFileSync(path, JSON.stringify(value));
    };
    run({ directory, check, json });
  } finally { rmSync(root, { recursive: true, force: true }); }
}

test('current production snapshots pass without node_modules', () => fixture(({ check }) => {
  const result = check(); assert.equal(result.status, 0, result.stderr);
}));
test('retired npm package is rejected even with a consistent lockfile', () => fixture(({ check, json }) => {
  json('package.json', data => { data.devDependencies['@xcss/web'] = '1.0.0'; });
  json('package-lock.json', data => { data.packages[''].devDependencies['@xcss/web'] = '1.0.0'; });
  const result = check(); assert.notEqual(result.status, 0); assert.match(result.stderr, /@xcss\/web/);
}));
test('old npm font preload paths fail the production contract', () => fixture(({ directory, check }) => {
  const path = join(directory, 'index.html');
  writeFileSync(path, readFileSync(path, 'utf8').replace('/vendor/xcss/', '/node_modules/@xcss/web/dist/'));
  const result = check(); assert.notEqual(result.status, 0); assert.match(result.stderr, /font preloads/);
}));
test('browser tests cannot silently import a second npm asset version', () => fixture(({ directory, check }) => {
  writeFileSync(join(directory, 'tests/stale.mjs'), "import { geometry } from '@xcss/web/admin-ui/server-corners';");
  const result = check(); assert.notEqual(result.status, 0); assert.match(result.stderr, /test shipped vendor assets/);
}));

test('changed vendor bytes fail the source digest contract', () => fixture(({ directory, check }) => {
  rmSync(join(directory, 'vendor'));
  const vendor = join(directory, 'vendor/xcss');
  mkdirSync(vendor, { recursive: true });
  writeFileSync(join(vendor, 'SOURCE.json'), JSON.stringify({ format: 1, files: { 'asset.css': '0'.repeat(64) } }));
  writeFileSync(join(vendor, 'asset.css'), 'changed bytes');
  const result = check(); assert.notEqual(result.status, 0); assert.match(result.stderr, /resync from the recorded public source/);
}));
