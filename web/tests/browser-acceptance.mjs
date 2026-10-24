import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import {chromium, firefox, expect} from '@playwright/test';
import {preview} from '../../tools/web-build/test-preview.mjs';
import {randomUUID} from 'node:crypto';

const session = {authenticated: true, user_id: 'A'.repeat(43), username: 'admin', role: 'admin', csrf_token: 'A'.repeat(43)};
const evidenceDirectory = process.env.XCSS_BROWSER_EVIDENCE_DIR;
if (evidenceDirectory) await mkdir(evidenceDirectory, {recursive: true});
const server = await preview({preview: {host: '127.0.0.1', port: 0, strictPort: true}});
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const page = await browser.newPage({locale: 'zh-CN', viewport: {width: 1440, height: 1000}});
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.emulateMedia({colorScheme: 'light'});
      await page.addInitScript(() => localStorage.removeItem('sarmg:theme'));
      const diagnostics = {backend: 'rustdesk', execution_mode: 'embedded', integration_state: 'awaiting_integration', rustdesk_version: null};
      const device = {
        id: randomUUID(), name: '原生 RustDesk 主机', role: 'controlled',
        addresses: [{host: 'host.example.com', port: 21118}], addresses_at_micros: Date.now() * 1000,
        registered: true, pairing_pending: false, revoked: false, client_online: true, diagnostics,
        capabilities: {protocol: 'xscs-management/2', client_version: '2.0.0-dev', os: 'windows_x86_64', backend: 'rustdesk', execution_mode: 'embedded', diagnostics: true},
        last_seen_at_micros: Date.now() * 1000,
      };
      const controller = {...device, id: randomUUID(), name: 'RustDesk 主控端', role: 'controller', addresses: [], addresses_at_micros: null};
      const devices = [device, controller], operations = [], edges = new Set();
      const authorizations = new Map([[device.id, 'c'.repeat(36)], [controller.id, 'd'.repeat(36)]]);
      let submitted = 0, createdCount = 0, accessRevision = 0, accessWrites = 0;
      let authorizationWrites = 0, revokeRequests = 0;
      let nextAccessRead = 'ok', pendingAccess = null, conflictNextWrite = false;
      const revision = () => accessRevision.toString(16).padStart(64, '0');
      const access = id => ({peer_ids: edges.has('allowed') ? [id === device.id ? controller.id : device.id] : [], revision: revision()});
      const apiError = (status, message) => ({status, json: {code: status === 409 ? 'access_revision_conflict' : 'unavailable', message, retryable: false}});
      await page.route('**/api/v1/**', async route => {
        const request = route.request(), path = new URL(request.url()).pathname;
        if (path === '/api/v1/rustdesk/devices') {
          if (request.method() === 'POST') {
            createdCount++;
            assert.equal(request.headers()['x-csrf-token'], session.csrf_token);
            assert.deepEqual(request.postDataJSON(), {name: '新增控制端', role: 'controller'});
            const created = {...device, id: randomUUID(), name: request.postDataJSON().name, role: request.postDataJSON().role, addresses: [], addresses_at_micros: null, registered: false, client_online: false, pairing_pending: true, capabilities: null, diagnostics: null};
            devices.push(created);
            authorizations.set(created.id, 'b'.repeat(36));
            return route.fulfill({status: 201, json: {device: created, manager_id: randomUUID(), token: 'b'.repeat(36)}});
          }
          return route.fulfill({json: devices});
        }
        if (path.endsWith('/access')) {
          const id = path.split('/').at(-2), peer = id === device.id ? controller.id : device.id;
          if (request.method() === 'PUT') {
            accessWrites++;
            assert.equal(request.headers()['x-csrf-token'], session.csrf_token);
            assert.equal(request.postDataJSON().expected_revision, revision());
            assert.ok(request.postDataJSON().peer_ids.every(value => value === peer));
            if (conflictNextWrite) {
              conflictNextWrite = false;
              accessRevision++;
              return route.fulfill(apiError(409, '授权已被其他管理员更新，请重新读取。'));
            }
            if (request.postDataJSON().peer_ids.length) edges.add('allowed'); else edges.delete('allowed');
            accessRevision++;
          } else {
            const mode = nextAccessRead;
            nextAccessRead = 'ok';
            if (mode === 'hold') {
              pendingAccess = {route, id};
              return;
            }
            if (mode === 'fail') return route.fulfill(apiError(503, 'permission storage unavailable'));
            if (mode === 'missing') return route.fulfill(apiError(404, 'instance no longer exists'));
          }
          return route.fulfill({json: access(id)});
        }
        if (path.endsWith('/authorization')) {
          const id = path.split('/').at(-2);
          if (request.method() === 'PUT') {
            authorizationWrites++;
            assert.equal(request.headers()['x-csrf-token'], session.csrf_token);
            assert.deepEqual(Object.keys(request.postDataJSON()), ['authorization_code']);
            const code = request.postDataJSON().authorization_code;
            assert.match(code, /^[a-z0-9]{36}$/);
            assert.notEqual(code, authorizations.get(id));
            authorizations.set(id, code);
            Object.assign(devices.find(value => value.id === id), {
              registered: false, pairing_pending: true, client_online: false,
              last_seen_at_micros: null, diagnostics: null, capabilities: null,
              addresses: [], addresses_at_micros: null,
            });
          }
          return route.fulfill({json: {manager_id: randomUUID(), device_id: id, authorization_code: authorizations.get(id)}});
        }
        if (path.endsWith('/revoke')) {
          revokeRequests++;
          assert.equal(request.method(), 'POST');
          assert.equal(request.headers()['x-csrf-token'], session.csrf_token);
          const id = path.split('/').at(-2);
          Object.assign(devices.find(value => value.id === id), {revoked: true, client_online: false, pairing_pending: false});
          return route.fulfill({status: 204});
        }
        if (path.endsWith('/tasks')) {
          if (request.method() === 'POST') {
            submitted++;
            assert.equal(request.headers()['x-csrf-token'], session.csrf_token);
            assert.ok(request.headers()['idempotency-key']);
            assert.deepEqual(request.postDataJSON(), {kind: 'read_diagnostics'});
            const operation = {operation_id: `op_${randomUUID()}`, device_id: device.id, action: 'rustdesk.diagnostics.read', state: 'succeeded', created_at_server: new Date().toISOString(), result: {kind: 'diagnostics_read', snapshot: diagnostics}, reconciliation: null};
            operations.unshift(operation);
            return route.fulfill({status: 202, json: operation});
          }
          assert.equal(new URL(request.url()).search, '?recent=50');
          return route.fulfill({json: operations});
        }
        return route.fulfill({json: session});
      });
      const refresh = async () => {
        const response = page.waitForResponse(value => new URL(value.url()).pathname === '/api/v1/rustdesk/devices' && value.request().method() === 'GET');
        await page.getByRole('button', {name: '刷新', exact: true}).click();
        await response;
      };
      const releaseAccess = async () => {
        await expect.poll(() => pendingAccess !== null).toBe(true);
        const held = pendingAccess;
        pendingAccess = null;
        await held.route.fulfill({json: access(held.id)});
      };
      const screenshot = async name => {
        if (evidenceDirectory) {
          await page.screenshot({path: resolve(evidenceDirectory, `${engine.name()}-fixture-${name}.png`), fullPage: true});
        }
      };

      const wasm = page.waitForResponse(response => new URL(response.url()).pathname.endsWith('.wasm') && response.status() === 200);
      await page.goto(server.resolvedUrls.local[0]);
      await wasm;
      await expect(page.locator('html')).toHaveAttribute('data-xcss-runtime', 'xcss-web', {timeout: 30_000});
      await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
      await page.emulateMedia({colorScheme: 'dark'});
      await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
      await page.getByRole('button', {name: '切换到浅色模式', exact: true}).click();
      await page.emulateMedia({colorScheme: 'light'});
      await page.emulateMedia({colorScheme: 'dark'});
      await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
      await expect(page.getByRole('heading', {name: '实例管理', exact: true})).toBeVisible();
      await expect(page.getByRole('link', {name: device.name, exact: true})).toBeVisible();
      await screenshot('instances-desktop');
      await page.setViewportSize({width: 390, height: 844});
      await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      await screenshot('instances-mobile');
      await page.setViewportSize({width: 1440, height: 1000});

      // The user's search and role selection survive a real instance refresh.
      await page.getByRole('searchbox', {name: '搜索实例', exact: true}).fill('主控端');
      await page.getByRole('combobox', {name: '筛选角色', exact: true}).selectOption('controller');
      await refresh();
      await expect(page.getByRole('searchbox', {name: '搜索实例', exact: true})).toHaveValue('主控端');
      await expect(page.getByRole('combobox', {name: '筛选角色', exact: true})).toHaveValue('controller');
      await expect(page.getByRole('link', {name: controller.name, exact: true})).toBeVisible();
      await expect(page.getByRole('link', {name: device.name, exact: true})).toHaveCount(0);
      await page.getByRole('searchbox', {name: '搜索实例', exact: true}).fill('');
      await page.getByRole('combobox', {name: '筛选角色', exact: true}).selectOption('all');

      nextAccessRead = 'hold';
      await page.getByRole('link', {name: device.name, exact: true}).click();
      await expect(page.getByRole('heading', {name: device.name, exact: true})).toBeVisible();
      const peerCheckbox = () => page.getByRole('checkbox', {name: controller.name, exact: true});
      const saveAccess = () => page.getByRole('button', {name: '保存控制权限', exact: true});
      const reloadAccess = () => page.getByRole('button', {name: '重新读取授权', exact: true});
      await expect(peerCheckbox()).toBeDisabled();
      await expect(saveAccess()).toBeDisabled();
      await releaseAccess();
      await expect(peerCheckbox()).toBeEnabled();
      await expect(page.getByText('被控端 · 等待原生客户端就绪', {exact: true})).toBeVisible();
      await expect(page.getByText('host.example.com:21118', {exact: true})).toBeVisible();
      await peerCheckbox().check();
      await saveAccess().click();
      await expect(page.getByRole('status').filter({hasText: '控制权限已保存'})).toBeVisible();
      await page.getByRole('link', {name: '实例', exact: true}).click();
      await page.getByRole('link', {name: controller.name, exact: true}).click();
      await expect(page.getByRole('checkbox', {name: device.name, exact: true})).toBeChecked();
      await expect(page.getByText('控制端 · 控制端已连接', {exact: true})).toBeVisible();
      // Even an enrolled, online controller cannot run a host diagnostic task.
      await expect(page.getByRole('button', {name: '读取主机诊断', exact: true})).toBeDisabled();
      await page.getByRole('checkbox', {name: device.name, exact: true}).uncheck();
      await saveAccess().click();
      await expect(page.getByRole('status').filter({hasText: '控制权限已保存'})).toBeVisible();
      await page.getByRole('link', {name: '实例', exact: true}).click();
      await page.getByRole('link', {name: device.name, exact: true}).click();
      await expect(peerCheckbox()).not.toBeChecked();

      await page.getByRole('button', {name: '读取主机诊断', exact: true}).click();
      await expect(page.getByRole('cell', {name: '读取主机诊断', exact: true})).toBeVisible();
      assert.equal(submitted, 1);
      await expect(page.getByRole('button', {name: '显示实例密码', exact: true})).toBeDisabled();
      await expect(page.getByText('已绑定', {exact: true}).first()).toBeVisible();
      await page.getByRole('button', {name: '撤销客户端凭据', exact: true}).click();
      await expect(page.getByText(/撤销会使该实例永久失效，不可再次绑定。需要恢复请新建实例/)).toBeVisible();
      assert.equal(revokeRequests, 0);
      await page.getByRole('button', {name: '取消', exact: true}).click();
      assert.equal(revokeRequests, 0);

      // Reset is explicit, cancellable, and only the confirmed PUT invalidates registration.
      await page.getByRole('button', {name: '重置实例密码', exact: true}).click();
      await expect(page.getByRole('button', {name: '确认重置实例密码', exact: true})).toBeVisible();
      assert.equal(authorizationWrites, 0);
      await page.getByRole('button', {name: '取消重置', exact: true}).click();
      assert.equal(authorizationWrites, 0);
      await expect(page.getByRole('button', {name: '显示实例密码', exact: true})).toBeDisabled();
      await page.getByRole('button', {name: '重置实例密码', exact: true}).click();
      const resetResponse = page.waitForResponse(response => new URL(response.url()).pathname.endsWith('/authorization') && response.request().method() === 'PUT');
      await page.getByRole('button', {name: '确认重置实例密码', exact: true}).click();
      await resetResponse;
      await expect(page.getByText(authorizations.get(device.id), {exact: true})).toBeVisible();
      await expect(page.getByText('c'.repeat(36), {exact: true})).toHaveCount(0);
      await expect(page.getByText('等待绑定', {exact: true}).first()).toBeVisible();
      await expect(page.getByRole('button', {name: '读取主机诊断', exact: true})).toBeDisabled();
      assert.equal(authorizationWrites, 1);
      assert.equal(device.registered, false);
      assert.equal(device.pairing_pending, true);
      await page.getByLabel('实例名称', {exact: true}).fill('尚未保存的名称');
      await peerCheckbox().check();
      controller.last_seen_at_micros += 5_000_000;
      await refresh();
      await expect(page.getByLabel('实例名称', {exact: true})).toHaveValue('尚未保存的名称');
      await expect(page.getByText(authorizations.get(device.id), {exact: true})).toBeVisible();
      await expect(peerCheckbox()).toBeChecked();
      await expect(saveAccess()).toBeEnabled();
      controller.last_seen_at_micros += 5_000_000;
      await page.waitForResponse(response => new URL(response.url()).pathname === '/api/v1/rustdesk/devices' && response.request().method() === 'GET', {timeout: 12_000});
      await expect(page.getByLabel('实例名称', {exact: true})).toHaveValue('尚未保存的名称');
      await expect(page.getByText(authorizations.get(device.id), {exact: true})).toBeVisible();
      await expect(peerCheckbox()).toBeChecked();
      await expect(saveAccess()).toBeEnabled();
      await page.getByRole('button', {name: '隐藏实例密码', exact: true}).click();
      await expect(page.getByText(authorizations.get(device.id), {exact: true})).toHaveCount(0);

      // A revision conflict preserves the draft and requires explicit reconciliation.
      conflictNextWrite = true;
      await saveAccess().click();
      await expect(page.getByText(/授权已被其他管理员更新，请重新读取。/)).toBeVisible();
      await expect(peerCheckbox()).toBeChecked();
      await expect(saveAccess()).toBeDisabled();
      await expect(page.getByRole('status').filter({hasText: '已保留未保存的选择'})).toBeVisible();
      nextAccessRead = 'hold';
      await reloadAccess().click();
      await expect(peerCheckbox()).toBeDisabled();
      await expect(peerCheckbox()).toBeChecked();
      await expect(saveAccess()).toBeDisabled();
      await releaseAccess();
      await expect(peerCheckbox()).not.toBeChecked();
      await expect(saveAccess()).toBeEnabled();

      // A failed GET never enables saving with the previous revision or draft.
      await peerCheckbox().check();
      nextAccessRead = 'fail';
      const failedAccess = page.waitForResponse(response => new URL(response.url()).pathname.endsWith('/access') && response.status() === 503);
      await reloadAccess().click();
      await failedAccess;
      await expect(page.getByRole('status').filter({hasText: '正在读取授权'})).toHaveCount(0);
      await expect(peerCheckbox()).toBeDisabled();
      await expect(peerCheckbox()).toBeChecked();
      await expect(saveAccess()).toBeDisabled();
      assert.equal(accessWrites, 3);
      await reloadAccess().click();
      await expect(peerCheckbox()).toBeEnabled();
      await expect(peerCheckbox()).not.toBeChecked();

      // A real API 404 leaves the workspace visible and explains recovery.
      nextAccessRead = 'missing';
      await reloadAccess().click();
      await expect(page.getByText('实例或记录已不存在，请刷新列表。', {exact: true})).toBeVisible();
      await expect(page.getByRole('heading', {name: device.name, exact: true})).toBeVisible();
      await expect(saveAccess()).toBeDisabled();
      await reloadAccess().click();
      await expect(saveAccess()).toBeEnabled();

      const rawDetails = page.locator('details').filter({has: page.getByText('原始 JSON', {exact: true})});
      await expect(rawDetails.locator('pre')).toBeHidden();
      await rawDetails.getByText('原始 JSON', {exact: true}).click();
      await expect(rawDetails.locator('pre')).toContainText('diagnostics_read');
      await rawDetails.getByText('原始 JSON', {exact: true}).click();
      assert.equal(await page.getByRole('button', {name: /PIN|重启|应用更改/}).count(), 0);
      await screenshot('workspace-desktop');
      await page.setViewportSize({width: 390, height: 844});
      await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      await screenshot('workspace-mobile');
      await page.setViewportSize({width: 1440, height: 1000});

      await page.getByRole('button', {name: '新建实例', exact: true}).click();
      const create = page.getByRole('button', {name: '创建实例并生成密码', exact: true});
      await expect(create).toBeDisabled();
      await page.getByLabel('实例名称', {exact: true}).fill('新增控制端');
      await page.getByRole('combobox', {name: '新实例角色', exact: true}).selectOption('controller');
      await create.click();
      await expect(page.getByRole('heading', {name: '新增控制端', exact: true})).toBeVisible();
      await expect(page.getByText('b'.repeat(36), {exact: true})).toBeVisible();
      assert.equal(createdCount, 1);
      await expect(page.getByRole('button', {name: '读取主机诊断', exact: true})).toBeDisabled();
      await page.getByRole('button', {name: '隐藏实例密码', exact: true}).click();
      await expect(page.getByText('b'.repeat(36), {exact: true})).toHaveCount(0);
      await page.getByRole('button', {name: '显示实例密码', exact: true}).click();
      await expect(page.getByText('b'.repeat(36), {exact: true})).toBeVisible();

      await page.evaluate(id => { location.hash = `details/${id}`; }, randomUUID());
      await expect(page.getByRole('heading', {name: '实例不存在或暂不可读取', exact: true})).toBeVisible();
      await page.getByRole('link', {name: '返回实例列表', exact: true}).click();
      await expect(page.getByRole('heading', {name: '实例管理', exact: true})).toBeVisible();
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: instance creation, permission recovery/CAS, polling drafts, filters, diagnostics, responsive layout and 404 passed.`);
    } finally {
      await browser.close();
    }
  }
} finally {
  await server.close();
}
console.log('Embedded RustDesk management browser acceptance passed (Chromium, Firefox; isolated API fixtures).');
