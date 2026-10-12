import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { chromium, firefox, expect } from "@playwright/test";
import { preview } from "vite";

const session = { authenticated: true, user_id: "A".repeat(43), username: "admin", role: "admin", csrf_token: "A".repeat(43) };
const version = "2026.914.233613";
const os = "linux_x86_64";
const fields = [
  { key: "sunshine_name", kind: "text", maximum_length: 32 },
  { key: "qp", kind: "integer", minimum: 0, maximum: 51 },
  { key: "controller", kind: "boolean" },
  { key: "keyboard", kind: "boolean" },
].map(field => ({ ...field, requires_restart: true, supported_sunshine_versions: [version], operating_systems: [os] }));
const snapshot = (revision, name, extra = {}) => ({
  revision: revision.repeat(64), sunshine_version: version,
  fields: { sunshine_name: name, qp: "28", controller: "true", ...extra },
  effectiveness: "pending_verification",
});
const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });

try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const context = await browser.newContext({ locale: "zh-CN", viewport: { width: 1280, height: 900 } });
      const page = await context.newPage();
      const errors = [];
      const commands = [];
      const operations = [];
      const operationCommands = new Map();
      let clock = Date.now() * 1000;
      let deviceRequests = 0;
      let recentRequests = 0;
      const device = {
        id: randomUUID(), name: "配置快照测试", registered: true, pairing_pending: false, revoked: false,
        client_online: true, sunshine_reachable: true, configuration_state: "awaiting_restart",
        last_seen_at_micros: clock,
        capabilities: {
          protocol: "xscs-management/1", client_version: "0.1.0", os, sunshine_version: version,
          restart_allowed: true, managed_fields: fields.map(field => field.key),
          configuration_overwrite:true,application_management: false, application_host_commands_allowed: false,
          pending_pairing_listing:true,moonlight_pairing_management: false, diagnostics: false, maintenance: false, service_control: false,
        },
        // The stored heartbeat snapshot must never be used as the editor's initial read result.
        snapshot: { ...snapshot("a", "Cached before opening", { keyboard: "false" }), effectiveness: "awaiting_restart" },
      };
      page.on("pageerror", error => errors.push(error.message));
      const readCommands = () => commands.filter(command => command.kind === "read_config");
      const writes = () => commands.filter(command => command.kind !== "read_config");
      const saves = () => commands.filter(command => command.kind === "save_config");
      const latestRead = () => operations.find(operation => operation.action === "sunshine.config.read");
      const completeRead = value => {
        const operation = latestRead();
        assert.ok(operation, "a read task must already exist");
        operation.state = "succeeded";
        operation.result = { kind: "config_read", snapshot: value };
        operation.updated_at_micros = ++clock;
        device.snapshot = value;
      };
      const completeWrite = (state, result, action) => {
        const operation = operations.find(value => ["save_config", "restart"].includes(operationCommands.get(value.operation_id)?.kind));
        assert.ok(operation, "a write task must already exist");
        operation.state = state;
        operation.result = result;
        if (action) operation.action = action;
        operation.updated_at_micros = ++clock;
        return operation;
      };
      const completeSave = () => {
        const operation = operations.find(value => operationCommands.get(value.operation_id)?.kind === "save_config");
        const command = operationCommands.get(operation?.operation_id);
        assert.ok(command, "a save task must already exist");
        const next = { ...device.snapshot.fields, ...Object.fromEntries(Object.entries(command.set).map(([key, value]) => [key, String(value)])) };
        for (const key of command.remove) delete next[key];
        device.snapshot = { ...snapshot("e", "Saved snapshot"), fields: next, effectiveness: "awaiting_restart" };
        device.configuration_state = "awaiting_restart";
        return completeWrite("succeeded", { kind: "config_saved", snapshot: device.snapshot });
      };
      await page.route("**/api/v1/**", async route => {
        const request = route.request();
        const path = new URL(request.url()).pathname;
        if (path.endsWith("/sunshine/config-fields")) return route.fulfill({ json: fields });
        if (path.endsWith("/sunshine/devices")) { deviceRequests++; return route.fulfill({ json: [device] }); }
        if (path.endsWith("/authorization")) return route.fulfill({ json: { manager_id: randomUUID(), device_id: device.id, authorization_code: "b".repeat(36) } });
        if (path.endsWith("/tasks/summary")) return route.fulfill({ json: { blocking_count: operations.filter(operation => operation.action !== "sunshine.config.read" && ["pending", "running", "unknown"].includes(operation.state)).length } });
        if (path.endsWith("/tasks/calendar")) return route.fulfill({ json: { today: "2030-01-02" } });
        if (/\/sunshine\/operations\/[^/]+$/.test(path)) {
          const operation = operations.find(value => value.operation_id === path.split("/").at(-1));
          assert.ok(operation, "operation polling must refer to a submitted task");
          return route.fulfill({ json: operation });
        }
        if (path.endsWith("/tasks")) {
          if (request.method() === "POST") {
            const command = request.postDataJSON();
            commands.push(command);
            assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
            assert.ok(request.headers()["idempotency-key"]);
            assert.ok(["read_config", "save_config", "restart"].includes(command.kind), "tasks must use the expected read, authoritative save or explicit restart commands");
            const now = ++clock;
            const operation = {
              operation_id: "op_" + randomUUID(), device_id: device.id,
              action: { read_config: "sunshine.config.read", save_config: "sunshine.config.save", restart: "sunshine.restart" }[command.kind],
              state: "pending", attempt: 1,
              created_at_micros: now, created_at_server: "2030-01-02 12:00:00.000000 UTC",
              updated_at_micros: now, result: null, reconciliation: null, resolution: null,
            };
            operationCommands.set(operation.operation_id, command);
            operations.unshift(operation);
            return route.fulfill({ json: operation });
          }
          if (new URL(request.url()).searchParams.has("recent")) recentRequests++;
          return route.fulfill({ json: operations });
        }
        return route.fulfill({ json: session });
      });

      await page.goto("http://127.0.0.1:" + server.httpServer.address().port);
      await page.getByRole("link", { name: "选择实例 配置快照测试", exact: true }).click();
      await expect.poll(() => readCommands().length).toBe(1);
      const categories = page.getByRole("navigation", { name: "Sunshine 配置分类" });
      await categories.getByRole("button", { name: "概况", exact: true }).click();
      const editor = page.getByRole("region", { name: "配置设置", exact: true });
      const configurationStatus = page.getByLabel("配置操作状态", { exact: true }).filter({ visible: true });
      const readStatus = "配置已读取，运行时生效待验证。";
      const savedStatus = "配置已保存，等待管理员重启。";
      const visibleSavedFeedback = editor.getByText("配置已保存，等待管理员重启", { exact: false }).filter({ visible: true });
      const name = page.getByLabel("Sunshine 名称", { exact: true });
      const preview = () => categories.getByRole("button", { name: "预览变更", exact: true }).click();
      const general = () => categories.getByRole("button", { name: "概况", exact: true }).click();
      const differences = page.getByRole("region", { name: "变更差异预览", exact: true });
      const differenceRow = label => differences.getByRole("row").filter({ has: page.getByRole("cell", { name: label, exact: true }) });
      const apply = page.getByRole("button", { name: "应用更改", exact: true });
      const noApplicableChanges = async () => {
        await expect(page.getByRole("region", { name: "预览变更", exact: true })).toContainText("没有需要应用的更改");
        await expect(apply).toHaveCount(0);
      };
      await expect(name).toHaveCount(0);
      await expect(visibleSavedFeedback).toHaveCount(0);
      await expect(page.getByRole("button", { name: /^(从 客户端 读取配置|使用最新配置|获取客户端最新配置)/ })).toHaveCount(0);

      // Persisted history can say awaiting_restart without proving a save on this page.
      completeRead({ ...snapshot("b", "Fresh after opening"), effectiveness: "awaiting_restart" });
      await expect(name).toHaveValue("Fresh after opening");
      await expect(configurationStatus).toHaveCount(0);
      await expect(visibleSavedFeedback).toHaveCount(0);
      await expect(page.getByRole("button", { name: "保存配置", exact: true })).toHaveCount(0);
      await expect(apply).toHaveCount(0);
      await categories.getByRole("button", { name: "实例概览", exact: true }).click();
      await expect(page.getByRole("region", { name: "Sunshine 状态", exact: true })).toContainText("配置已保存，等待管理员重启");
      await general();
      await name.fill("Unsaved first draft");
      await name.press("Enter");
      assert.equal(writes().length, 0, "Enter in an editor must not submit configuration");
      await categories.getByRole("button", { name: "输入", exact: true }).click();
      const controller = page.getByRole("combobox", { name: "控制器输入", exact: true });
      const keyboard = page.getByRole("combobox", { name: "键盘输入", exact: true });
      await controller.click();
      await page.getByRole("option", { name: "禁用", exact: true }).click();
      await keyboard.click();
      await page.getByRole("option", { name: "启用", exact: true }).click();
      await expect(configurationStatus).toHaveCount(0);
      await expect(apply).toHaveCount(0);
      await categories.getByRole("button", { name: "高级", exact: true }).click();
      const qp = page.getByLabel("量化参数 QP", { exact: true });
      const qpReset = page.getByRole("checkbox", { name: "量化参数 QP：恢复默认（删除显式设置）", exact: true });
      await qpReset.check();
      await preview();
      await expect(differences.getByRole("columnheader")).toHaveText(["字段", "原值", "新值", "撤销"]);
      await expect(differences.locator("tbody tr")).toHaveCount(4);
      await expect(configurationStatus).toHaveText(readStatus);
      await expect(visibleSavedFeedback).toHaveCount(0);
      await expect(differenceRow("键盘输入")).toContainText("默认");
      await differenceRow("键盘输入").getByRole("button", { name: "撤销", exact: true }).click();
      await expect(differenceRow("键盘输入")).toHaveCount(0);
      await expect(differences.locator("tbody tr")).toHaveCount(3);
      await categories.getByRole("button", { name: "输入", exact: true }).click();
      await expect(keyboard).toHaveText("未显式设置");
      await expect(controller).toHaveText("禁用");
      await preview();
      await expect(differenceRow("量化参数 QP")).toContainText("恢复默认");
      await differenceRow("量化参数 QP").getByRole("button", { name: "撤销", exact: true }).click();
      await expect(differenceRow("量化参数 QP")).toHaveCount(0);
      await categories.getByRole("button", { name: "高级", exact: true }).click();
      await expect(qp).toHaveValue("28");
      await expect(qpReset).not.toBeChecked();
      await expect(qp).toBeEnabled();
      await preview();
      await differenceRow("控制器输入").getByRole("button", { name: "撤销", exact: true }).click();
      await expect(differences.locator("tbody tr")).toHaveCount(1);
      await differenceRow("Sunshine 名称").getByRole("button", { name: "撤销", exact: true }).click();
      await noApplicableChanges();
      await general();
      await expect(name).toHaveValue("Fresh after opening");
      await categories.getByRole("button", { name: "输入", exact: true }).click();
      await expect(controller).toHaveText("启用");
      await general();
      await name.fill("Temporary draft");
      await name.fill("Fresh after opening");
      await preview();
      await noApplicableChanges();
      await general();
      await name.fill("Unsaved first draft");
      assert.equal(readCommands().length, 1, "switching menus and undoing changes must not fetch configuration");
      assert.equal(writes().length, 0, "editing, previewing and undoing must not submit a save or restart");

      device.snapshot = snapshot("c", "Background heartbeat drift", { qp: "40", keyboard: "true" });
      const beforeDevicePoll = deviceRequests;
      const beforeTaskPoll = recentRequests;
      await expect.poll(() => deviceRequests, { timeout: 10000 }).toBeGreaterThan(beforeDevicePoll);
      await expect.poll(() => recentRequests, { timeout: 5000 }).toBeGreaterThan(beforeTaskPoll);
      await expect(name).toHaveValue("Unsaved first draft");
      await expect(configurationStatus).toHaveCount(0);
      assert.equal(readCommands().length, 1, "background polling must not read configuration again");
      await expect(page.locator("body")).not.toContainText("配置修订已变化");
      await preview();
      await expect(differences).toContainText("Fresh after opening");
      await expect(differences).toContainText("Unsaved first draft");
      await expect(differences).not.toContainText("Background heartbeat drift");
      await general();
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect.poll(() => readCommands().length).toBe(2);
      // The pending read removes the old editor. A count/inputValue pair can
      // race that removal and wait for an input that cannot return until we
      // complete the read below; use the locator's retrying count assertion.
      await expect(name).toHaveCount(0);
      completeRead(snapshot("d", "Fresh after header refresh"));
      await expect(name).toHaveValue("Fresh after header refresh");
      await expect(configurationStatus).toHaveCount(0);
      await preview();
      await noApplicableChanges();
      await expect(configurationStatus).toHaveText(readStatus);
      assert.equal(writes().length, 0);

      await general();
      await name.fill("Saved directly");
      // A concurrent local change is overwritten only when Apply changes is explicitly pressed.
      device.snapshot = snapshot("f", "Concurrent local change", { qp: "40", keyboard: "true" });
      await preview();
      assert.equal(writes().length, 0);
      await apply.click();
      await expect.poll(() => writes().length).toBe(1);
      assert.deepEqual(saves()[0], {
        kind: "save_config", set: { sunshine_name: "Saved directly", qp: 28, controller: true },
        remove: ["keyboard"], restart_policy: "manual",
      });
      await expect(categories.getByRole("button", { name: "预览变更", exact: true })).toHaveAttribute("aria-pressed", "true");
      await expect(configurationStatus).toHaveText("配置保存请求已排队，等待客户端执行。");
      await expect(configurationStatus).toHaveAttribute("role", "status");
      await expect(visibleSavedFeedback).toHaveCount(0);
      await expect(apply).toBeDisabled();
      await expect(differenceRow("Sunshine 名称").getByRole("button", { name: "撤销", exact: true })).toBeDisabled();
      completeWrite("running", null);
      await expect(configurationStatus).toHaveText("正在保存配置，等待客户端执行回执。");
      await expect(differenceRow("Sunshine 名称").getByRole("button", { name: "撤销", exact: true })).toBeDisabled();
      await expect(visibleSavedFeedback).toHaveCount(0);
      assert.equal(writes().length, 1, "waiting for a receipt must not resubmit a save");
      // A response to the submitted values must not erase edits made while waiting for that response.
      await general();
      await name.fill("Edited during apply");
      await preview();
      await expect(differenceRow("Sunshine 名称")).toContainText("Edited during apply");
      const beforeSavedDevicePoll = deviceRequests;
      completeSave();
      await expect(configurationStatus).toHaveText(savedStatus);
      await expect(differenceRow("Sunshine 名称")).toContainText("Saved directly");
      await expect(differenceRow("Sunshine 名称")).toContainText("Edited during apply");
      await expect(name).toHaveValue("Edited during apply");
      await differenceRow("Sunshine 名称").getByRole("button", { name: "撤销", exact: true }).click();
      await noApplicableChanges();
      assert.equal(device.snapshot.fields.qp, "28", "unchanged managed fields overwrite concurrent local changes");
      assert.ok(!("keyboard" in device.snapshot.fields), "absent managed fields remove concurrent explicit values");
      await expect(name).toHaveValue("Saved directly");
      assert.equal(readCommands().length, 2, "saving must not trigger another configuration read");

      // Restart remains available only in General; its acknowledgement is shown in Preview.
      await expect.poll(() => deviceRequests, { timeout: 10000 }).toBeGreaterThan(beforeSavedDevicePoll);
      await general();
      await expect(configurationStatus).toHaveCount(0);
      await page.getByRole("button", { name: "重启 Sunshine", exact: true }).click();
      await page.getByRole("dialog", { name: "确认重启 Sunshine", exact: true }).getByRole("button", { name: "确认", exact: true }).click();
      await expect.poll(() => writes().length).toBe(2);
      assert.deepEqual(writes()[1], { kind: "restart", expected_revision: "e".repeat(64), administrator_confirmed: true });
      await preview();
      await expect(configurationStatus).not.toHaveText("Sunshine 已重启，运行时生效待验证。");
      device.snapshot = { ...device.snapshot, effectiveness: "pending_verification" };
      device.configuration_state = "pending_verification";
      completeWrite("succeeded", { kind: "restart_acknowledged", snapshot: device.snapshot, process_generation: "config-snapshot-fixture-generation" });
      await expect(configurationStatus).toHaveText("Sunshine 已重启，运行时生效待验证。");
      await expect(configurationStatus).toHaveAttribute("role", "status");
      assert.equal(readCommands().length, 2, "explicit restart must not replace the frozen editor snapshot");
      assert.equal(writes().length, 2, "restart acknowledgement must not trigger another restart");

      const applyName = async value => {
        await general();
        await name.fill(value);
        await expect(configurationStatus).toHaveCount(0);
        await expect(apply).toHaveCount(0);
        await preview();
        await expect(differenceRow("Sunshine 名称")).toContainText(value);
        await apply.click();
      };
      await applyName("Rejected save draft");
      await expect.poll(() => saves().length).toBe(2);
      completeWrite("failed", { kind: "rejected", reason: "fixture_save_rejected" });
      await expect(configurationStatus).toHaveText("配置保存失败，请检查日志。");
      await expect(configurationStatus).toHaveAttribute("role", "alert");
      await expect(differenceRow("Sunshine 名称")).toContainText("Rejected save draft");
      await expect(differenceRow("Sunshine 名称").getByRole("button", { name: "撤销", exact: true })).toBeEnabled();
      await expect(name).toHaveValue("Rejected save draft");
      await expect(apply).toBeEnabled();
      assert.equal(writes().length, 3, "a failed save must not retry itself");

      await applyName("Mismatched result draft");
      await expect.poll(() => saves().length).toBe(3);
      completeWrite("succeeded", { kind: "config_read", snapshot: { ...device.snapshot, effectiveness: "awaiting_restart" } });
      await expect(configurationStatus).toHaveText("配置操作回执不匹配，请检查日志核对结果。");
      await expect(configurationStatus).toHaveAttribute("role", "alert");
      await expect(name).toHaveValue("Mismatched result draft");
      await expect(visibleSavedFeedback).toHaveCount(0);
      await expect(apply).toBeEnabled();
      assert.equal(writes().length, 4, "a mismatched save result must not replay the operation");

      await applyName("Mismatched action draft");
      await expect.poll(() => saves().length).toBe(4);
      completeWrite("succeeded", { kind: "config_saved", snapshot: { ...device.snapshot, effectiveness: "awaiting_restart" } }, "sunshine.config.read");
      await expect(configurationStatus).toHaveText("配置操作回执不匹配，请检查日志核对结果。");
      await expect(configurationStatus).toHaveAttribute("role", "alert");
      await expect(name).toHaveValue("Mismatched action draft");
      await expect(visibleSavedFeedback).toHaveCount(0);
      await expect(apply).toBeEnabled();
      assert.equal(writes().length, 5, "a save result from the wrong action must not replay the operation");

      await applyName("Unconfirmed save draft");
      await expect.poll(() => saves().length).toBe(5);
      const unknown = completeWrite("unknown", { kind: "unknown", reason: "side_effect_not_confirmed" });
      await expect(configurationStatus).toHaveText("配置保存结果未确认，请检查日志并核对客户端实际配置。");
      await expect(configurationStatus).toHaveAttribute("role", "alert");
      await expect(name).toHaveValue("Unconfirmed save draft");
      await expect(visibleSavedFeedback).toHaveCount(0);
      await expect(apply).toBeDisabled();
      assert.equal(writes().length, 6, "an unknown save must not retry itself");
      unknown.state = "resolved";
      unknown.resolution = "unable_to_confirm";
      unknown.updated_at_micros = ++clock;
      await expect(configurationStatus).toHaveText("该操作已人工核对，请查看日志中的结论。");
      await expect(apply).toBeEnabled();

      await general();
      await name.fill("Unsaved before browser reload");
      device.configuration_state = "awaiting_restart";
      device.snapshot = { ...device.snapshot, effectiveness: "awaiting_restart" };
      await page.reload();
      await expect.poll(() => readCommands().length).toBe(3);
      await general();
      await expect(name).toHaveCount(0);
      completeRead({ ...snapshot("1", "Fresh after browser reload"), effectiveness: "awaiting_restart" });
      await expect(name).toHaveValue("Fresh after browser reload");
      await expect(configurationStatus).toHaveCount(0);
      await preview();
      await noApplicableChanges();
      await expect(configurationStatus).toHaveText(readStatus);
      await expect(visibleSavedFeedback).toHaveCount(0);
      assert.equal(saves().length, 5);
      assert.equal(writes().length, 6, "browser reload must not repeat a save or restart");
      device.capabilities.configuration_overwrite = false;
      await page.getByRole("button", {name:"刷新", exact:true}).click();
      await expect.poll(() => readCommands().length).toBe(4);
      completeRead({ ...snapshot("1", "Fresh after browser reload"), effectiveness: "awaiting_restart" });
      await general();
      await name.fill("Blocked without capability");
      await preview();
      await expect(page.getByText("客户端尚未报告完整配置保存能力。", {exact:true})).toBeVisible();
      await expect(apply).toBeDisabled();
      assert.equal(writes().length, 6, "a missing capability must not submit or replay a write");
      assert.deepEqual(errors, []);
      console.log(engine.name() + ": automatic preview, per-field undo, explicit apply, frozen snapshot and truthful receipts passed");
    } finally {
      await browser.close();
    }
  }
} finally {
  await new Promise(done => server.httpServer.close(done));
}
