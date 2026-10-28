import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import {chromium, firefox, expect} from '@playwright/test';
import {preview} from '../../tools/web-build/test-preview.mjs';
import {randomUUID} from 'node:crypto';

const session = {authenticated: true, user_id: 'A'.repeat(43), username: 'admin', role: 'admin', csrf_token: 'A'.repeat(43)};
const evidenceDirectory = process.env.XCSS_BROWSER_EVIDENCE_DIR;
const evidencePrefix = process.env.XCSS_BROWSER_EVIDENCE_PREFIX || "";
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
        id: randomUUID(), name: '原生 RustDesk 主机', role: 'controlled', backend: 'rustdesk',
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
      let holdNextCreate = false, pendingCreate = null;
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
            assert.deepEqual(request.postDataJSON(), {name: '新增控制端', role: 'controller', backend: 'rustdesk'});
            const created = {...device, id: randomUUID(), name: request.postDataJSON().name, role: request.postDataJSON().role, backend: request.postDataJSON().backend, addresses: [], addresses_at_micros: null, registered: false, client_online: false, pairing_pending: true, capabilities: null, diagnostics: null};
            devices.push(created);
            authorizations.set(created.id, 'b'.repeat(36));
            const reply = {status: 201, json: {device: created, manager_id: randomUUID(), token: 'b'.repeat(36)}};
            if (holdNextCreate) { holdNextCreate = false; pendingCreate = {route, reply}; return; }
            return route.fulfill(reply);
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
      const screenshot = async (name, fullPage = true) => {
        if (evidenceDirectory) {
          await page.screenshot({path: resolve(evidenceDirectory, `${engine.name()}-fixture-${evidencePrefix}${name}.png`), fullPage});
        }
      };

      const select = name => page.getByRole('combobox', {name, exact: true});
      const openSelect = async name => {
        const control = select(name);
        if (await control.getAttribute('aria-expanded') !== 'true') await control.click();
        const panel = page.locator('#' + await control.getAttribute('aria-controls'));
        await expect(control).toHaveAttribute('aria-expanded', 'true');
        await expect(panel).toBeVisible();
        return {control, panel};
      };
      const chooseOption = async (name, label) => {
        const {control, panel} = await openSelect(name);
        await panel.getByRole('option', {name: label, exact: true}).click();
        await expect(control).toHaveAttribute('aria-expanded', 'false');
        await expect(control).toBeFocused();
        await expect(control).toContainText(label);
      };
      const rowGeometry = async name => page.locator('tbody tr').filter({has: page.getByRole('link', {name, exact: true})}).evaluate(row => {
        const rect = node => { const box = node.getBoundingClientRect(); return {x:box.x,y:box.y,width:box.width,height:box.height}; };
        const table = rect(row.closest('table')), box = rect(row);
        const textCenter = element => {
          const parts=[];
          const walker=document.createTreeWalker(element,NodeFilter.SHOW_TEXT);
          while(walker.nextNode()) {
            const node=walker.currentNode;
            if(!node.textContent.trim() || node.parentElement.closest('svg,[data-xcss-server-corner-decoration]')) continue;
            const range=document.createRange(); range.selectNodeContents(node);
            for(const part of range.getClientRects()) if(part.width && part.height) parts.push(part);
          }
          if(!parts.length) return null;
          return (Math.min(...parts.map(part=>part.y))+Math.max(...parts.map(part=>part.bottom)))/2;
        };
        const cells=[...row.children].filter(node=>node.matches('th,td'));
        return {height:box.height,y:box.y-table.y,tableWidth:table.width,
          columns:cells.map(cell=>{const b=rect(cell);return {x:b.x-box.x,width:b.width};}),
          textOffsets:cells.map(cell=>{const center=textCenter(cell);return center===null?null:center-(box.y+box.height/2);}),
          buttons:[...row.querySelectorAll('button')].filter(node=>{const b=rect(node);return b.width>0 && b.height>0 && getComputedStyle(node).visibility!=='hidden';})
            .map(button=>{const b=rect(button),center=textCenter(button);return {height:b.height,offset:b.y+b.height/2-(box.y+box.height/2),textOffset:center===null?null:center-(box.y+box.height/2)};})};
      });
      const assertCenteredRow = geometry => {
        assert.ok(Math.abs(geometry.height-36)<0.25, `instance row is ${geometry.height}px`);
        for(const offset of geometry.textOffsets) if(offset!==null) assert.ok(Math.abs(offset)<1.6, `cell text vertical offset ${offset}`);
        for(const button of geometry.buttons) {
          assert.ok(Math.abs(button.height-34)<0.25, `table button is ${button.height}px`);
          assert.ok(Math.abs(button.offset)<0.6, `button box vertical offset ${button.offset}`);
          if(button.textOffset!==null) assert.ok(Math.abs(button.textOffset)<1.6, `button text vertical offset ${button.textOffset}`);
        }
      };
      const assertStableInlineDelete = async (name, remove, cancel) => {
        const row=page.locator('tbody tr').filter({has:page.getByRole('link',{name,exact:true})});
        const before=await rowGeometry(name); assertCenteredRow(before);
        await row.getByRole('button',{name:remove,exact:true}).click();
        await expect(row.getByRole('button',{name:cancel,exact:true})).toBeVisible();
        await expect(row.getByRole('button',{name:cancel==='Cancel'?'Confirm delete':'确认删除',exact:true})).toBeVisible();
        const confirming=await rowGeometry(name); assertCenteredRow(confirming);
        assert.ok(Math.abs(confirming.y-before.y)<0.5, 'confirmation must not move the row');
        for(let index=0;index<before.columns.length;index++) {
          assert.ok(Math.abs(confirming.columns[index].x-before.columns[index].x)<0.5, 'confirmation must not move table columns');
          assert.ok(Math.abs(confirming.columns[index].width-before.columns[index].width)<0.5, 'confirmation must not resize operation/table columns');
        }
        await row.getByRole('button',{name:cancel,exact:true}).click();
        await expect(row.getByRole('button',{name:remove,exact:true})).toBeVisible();
        const after=await rowGeometry(name); assertCenteredRow(after);
        assert.ok(Math.abs(after.y-before.y)<0.5);
        for(let index=0;index<before.columns.length;index++) assert.ok(Math.abs(after.columns[index].width-before.columns[index].width)<0.5);
      };
      const assertSingleCodeRow = async () => {
        const geometry=await page.locator('.xscs-sensitive').evaluate(element=>{
          const box=element.getBoundingClientRect(), parts=[];
          const walker=document.createTreeWalker(element,NodeFilter.SHOW_TEXT);
          while(walker.nextNode()) {const node=walker.currentNode;if(!node.textContent.trim()||node.parentElement.closest('svg,[data-xcss-server-corner-decoration]'))continue;const range=document.createRange();range.selectNodeContents(node);parts.push(...range.getClientRects());}
          return {height:box.height,lines:parts.length,offset:parts.length?(Math.min(...parts.map(p=>p.y))+Math.max(...parts.map(p=>p.bottom)))/2-(box.y+box.height/2):null};
        });
        assert.ok(Math.abs(geometry.height-36)<0.25, `sensitive row is ${geometry.height}px`);
        assert.equal(geometry.lines,1); assert.ok(Math.abs(geometry.offset)<1.6);
      };
      const assertMenuFitsViewport = async panel => {
        const box = await panel.boundingBox();
        assert.ok(box);
        const viewport = page.viewportSize();
        assert.ok(box.x >= 7.5 && box.y >= 7.5 && box.x + box.width <= viewport.width - 7.5 && box.y + box.height <= viewport.height - 7.5);
      };

      const wasm = page.waitForResponse(response => new URL(response.url()).pathname.endsWith('.wasm') && response.status() === 200);
      await page.goto(server.resolvedUrls.local[0]);
      await wasm;
      await expect(page.locator('html')).toHaveAttribute('data-xcss-runtime', 'xcss-web', {timeout: 30_000});
      // Geometry screenshots exercise the final application font metrics; startup itself is independent.
      await expect(page.locator('html')).toHaveAttribute('data-xcss-fonts', 'ready', {timeout: 30_000});
      await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
      await page.emulateMedia({colorScheme: 'dark'});
      await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
      await page.getByRole('button', {name: '切换到浅色模式', exact: true}).click();
      await page.emulateMedia({colorScheme: 'light'});
      await page.emulateMedia({colorScheme: 'dark'});
      await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
      await expect(page.getByRole('heading', {name: '实例管理', exact: true})).toBeVisible();
      await expect(page.getByRole('link', {name: device.name, exact: true})).toBeVisible();
      await assertStableInlineDelete(device.name,'删除','取消');
      await screenshot('instances-desktop');
      await page.setViewportSize({width: 390, height: 844});
      await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      await screenshot('instances-mobile');
      await page.setViewportSize({width: 1440, height: 1000});

      // Shared zxs geometry and custom listbox, including usable square fallback.
      let menu = await openSelect('筛选角色');
      await expect(menu.control).toHaveCSS('height', '36px');
      await expect(page.getByRole('searchbox', {name: '搜索实例', exact: true})).toHaveCSS('height', '36px');
      await expect(menu.panel.getByRole('option').first()).toHaveCSS('height', '36px');
      await expect(menu.control).toHaveAttribute('data-xcss-server-corner-profile', 'zxs-g4');
      await expect(menu.panel).toHaveAttribute('data-xcss-server-corner-profile', 'zxs-g4');
      await assertMenuFitsViewport(menu.panel);
      await screenshot('instances-dropdown-desktop', false);
      await menu.control.press('Escape');
      await expect(menu.control).toBeFocused();
      await expect(menu.panel).toBeHidden();

      await menu.control.press('ArrowDown');
      await menu.control.press('ArrowDown');
      await menu.control.press('Enter');
      await expect(menu.control).toContainText('控制端');
      await expect(menu.control).toBeFocused();
      await menu.control.press('End');
      await menu.control.press('Escape');
      await expect(menu.control).toContainText('控制端');
      await expect(menu.control).toHaveAttribute('aria-expanded', 'false');
      await menu.control.press(' ');
      await menu.control.press('Tab');
      await expect(menu.panel).toBeHidden();
      await expect(menu.control).not.toBeFocused();
      await chooseOption('筛选角色', '全部角色');

      await page.setViewportSize({width: 390, height: 844});
      menu = await openSelect('筛选角色');
      await assertMenuFitsViewport(menu.panel);
      await screenshot('instances-dropdown-mobile', false);
      await page.setViewportSize({width: 390, height: 250});
      await expect(menu.panel).toBeHidden();
      await select('筛选角色').scrollIntoViewIfNeeded();
      await select('筛选角色').evaluate(element => window.scrollBy(0, element.getBoundingClientRect().bottom - innerHeight + 8));
      menu = await openSelect('筛选角色');
      await assertMenuFitsViewport(menu.panel);
      assert.ok((await menu.panel.boundingBox()).y + (await menu.panel.boundingBox()).height <= (await menu.control.boundingBox()).y - 3);
      await menu.control.press('Escape');
      await page.setViewportSize({width: 1440, height: 1000});

      await page.getByRole('button', {name: '切换到深色模式', exact: true}).click();
      menu = await openSelect('筛选角色');
      await screenshot('instances-dropdown-dark', false);
      await menu.control.press('Escape');
      await page.getByRole('button', {name: '切换到浅色模式', exact: true}).click();
      await page.emulateMedia({forcedColors: 'active'});
      menu = await openSelect('筛选角色');
      await expect(menu.panel).toHaveAttribute('data-xcss-server-corner-profile', 'zxs-g4');
      await expect(menu.panel).toHaveAttribute('data-xcss-server-corner-state', 'fallback');
      await expect(menu.panel).toHaveAttribute('data-xcss-server-corner-reason', 'forced-colors');
      await chooseOption('筛选角色', '被控端');
      await expect(menu.panel).toBeHidden();
      await page.emulateMedia({forcedColors: 'none'});
      await chooseOption('筛选角色', '全部角色');

      // The user's search and role selection survive a real instance refresh.
      await page.getByRole('searchbox', {name: '搜索实例', exact: true}).fill('主控端');
      await chooseOption('筛选角色', '控制端');
      await refresh();
      await expect(page.getByRole('searchbox', {name: '搜索实例', exact: true})).toHaveValue('主控端');
      await expect(select('筛选角色')).toContainText('控制端');
      await expect(page.getByRole('link', {name: controller.name, exact: true})).toBeVisible();
      await expect(page.getByRole('link', {name: device.name, exact: true})).toHaveCount(0);
      await page.getByRole('searchbox', {name: '搜索实例', exact: true}).fill('');
      await chooseOption('筛选角色', '全部角色');

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
      await peerCheckbox().focus(); await peerCheckbox().press('Space'); await expect(peerCheckbox()).toBeChecked();
      await peerCheckbox().press('Space'); await expect(peerCheckbox()).not.toBeChecked();
      await page.emulateMedia({forcedColors:'active'});
      await peerCheckbox().focus(); await peerCheckbox().press('Space'); await expect(peerCheckbox()).toBeChecked();
      await expect(peerCheckbox()).toBeEnabled();
      await peerCheckbox().press('Space'); await expect(peerCheckbox()).not.toBeChecked();
      await page.emulateMedia({forcedColors:'none'});
      await expect(page.getByText('被控端 · 等待设备或客户端就绪', {exact: true})).toBeVisible();
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
      await assertSingleCodeRow();
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
      const newRole = select('新实例角色');
      const roleBox=await newRole.boundingBox(), nameBox=await page.getByLabel('实例名称',{exact:true}).boundingBox();
      assert.ok(Math.abs(roleBox.height-44)<0.25 && Math.abs(nameBox.height-44)<0.25);
      assert.ok(Math.abs(roleBox.y-nameBox.y)<0.5 && Math.abs(roleBox.y+roleBox.height-nameBox.y-nameBox.height)<0.5, 'create role and name share top/bottom edges');
      await newRole.focus();
      await newRole.press('Enter');
      await newRole.press('End');
      await newRole.press('Enter');
      await expect(newRole).toContainText('控制端');
      assert.equal(createdCount, 0, 'Enter in the role selector must not submit the form');
      await newRole.press('Enter');
      await expect(newRole).toHaveAttribute('aria-expanded', 'true');
      holdNextCreate = true;
      // Avoid pointerdown dismissal: busy must itself close and disable the open menu.
      await create.evaluate(button => button.click());
      await expect.poll(() => pendingCreate !== null).toBe(true);
      await expect(newRole).toBeDisabled();
      await expect(newRole).toHaveAttribute('aria-expanded', 'false');
      await expect(page.locator('#' + await newRole.getAttribute('aria-controls'))).toBeHidden();
      const heldCreate = pendingCreate;
      pendingCreate = null;
      await heldCreate.route.fulfill(heldCreate.reply);
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
      // Display labels can be translated while the submitted/filter value remains a role key.
      const translated = new URL(server.resolvedUrls.local[0]);
      translated.searchParams.set('lang', 'en');
      translated.hash = 'instances';
      await page.goto(translated.href);
      await expect(page.getByRole('heading', {name: 'Instance management', exact: true})).toBeVisible();
      await chooseOption('Filter by role', 'All roles');
      await assertStableInlineDelete(device.name,'Delete','Cancel');
      await chooseOption('Filter by role', 'Controller');
      await expect(page.getByRole('link', {name: controller.name, exact: true})).toBeVisible();
      await expect(page.getByRole('link', {name: device.name, exact: true})).toHaveCount(0);
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
