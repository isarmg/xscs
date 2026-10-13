import { t } from "@xcss/web/admin-ui/i18n";

/** Display only: protocol values in requests, stored tasks and snapshots stay exact. */
const labels: Record<string, readonly [string, string]> = {
  read_config: ["读取配置", "Read configuration"], patch_config: ["保存配置", "Save configuration"], save_config: ["保存配置", "Save configuration"], "config.save": ["保存配置", "Save configuration"], restart: ["重启", "Restart"],
  config_read: ["配置已读取", "Configuration read"], config_saved: ["配置已保存", "Configuration saved"],
  restart_acknowledged: ["Sunshine 进程已重启，运行时生效待验证", "Sunshine process restarted; runtime effect awaits verification"], conflict: ["修订冲突", "Revision conflict"],
  rejected: ["执行被拒绝", "Execution rejected"], unknown: ["结果不确定", "Outcome unknown"],
  invalid_task: ["任务无效", "Invalid task"], binding_mismatch: ["设备绑定不匹配", "Device binding mismatch"],
  permission_denied: ["权限不足", "Permission denied"], operation_id_reused: ["操作标识被不同内容复用", "Operation ID reused with different content"],
  unsupported_version: ["版本不受支持", "Unsupported version"], unsafe_configuration: ["配置不安全", "Unsafe configuration"],
  sunshine_unavailable: ["Sunshine 不可访问", "Sunshine unavailable"], journal_full: ["执行日志已满", "Execution journal full"],
  execution_deadline_expired: ["任务投递期限已过，未执行", "Task dispatch deadline expired without execution"],
  execution_expired: ["任务已过期，未执行", "Task expired without execution"], result_too_large: ["执行结果超出传输上限", "Execution result exceeds the message limit"], operation_already_completed: ["操作已收存，不重复执行", "Operation already accepted; no reexecution"],
  no_execution_record: ["没有可核对的执行记录", "No execution record"], effect_not_confirmed: ["生效状态未确认", "Effect not confirmed"],
  restart_not_confirmed: ["重启结果未确认", "Restart not confirmed"], persistence_failure: ["执行记录保存失败", "Execution record persistence failed"],
  "applications.list": ["读取应用", "Read applications"], "applications.save": ["保存应用", "Save application"],
  "applications.delete": ["删除应用", "Delete application"], "applications.close": ["关闭当前应用", "Close current application"],
  "applications.cover.upload": ["上传应用封面", "Upload application cover"], "pairing.pin.submit": ["提交 Moonlight PIN", "Submit Moonlight PIN"],
  "pairing.clients.list": ["读取 Moonlight 客户端", "Read Moonlight clients"], "pairing.clients.update": ["更新 Moonlight 客户端", "Update Moonlight client"],
  "pairing.pending.list": ["待配对请求", "Pending pairing requests"], pending_pairings_read: ["待配对请求已更新", "Pending pairing requests updated"], pairing_failed: ["Moonlight 配对未完成", "Moonlight pairing did not complete"],
  "pairing.clients.unpair": ["取消 Moonlight 配对", "Unpair Moonlight client"], "pairing.clients.unpair_all": ["取消全部 Moonlight 配对", "Unpair all Moonlight clients"],
  "logs.read": ["读取 Sunshine 日志", "Read Sunshine logs"], "diagnostics.read": ["读取 Sunshine 诊断", "Read Sunshine diagnostics"],
  "virtual_input.read": ["读取虚拟输入状态", "Read virtual input status"], "maintenance.run": ["执行 Sunshine 维护", "Run Sunshine maintenance"],
  "service.read": ["读取 Sunshine 服务状态", "Read Sunshine service status"], "service.control": ["控制 Sunshine 服务", "Control Sunshine service"],
  applications_read: ["应用已读取", "Applications read"], application_saved: ["应用已保存", "Application saved"], application_deleted: ["应用已删除", "Application deleted"],
  application_closed: ["当前应用已关闭", "Current application closed"], cover_uploaded: ["封面已上传", "Cover uploaded"], pairing_pin_submitted: ["配对 PIN 已提交", "Pairing PIN submitted"],
  paired_clients_read: ["Moonlight 客户端已读取", "Moonlight clients read"], paired_client_updated: ["Moonlight 客户端已更新", "Moonlight client updated"],
  logs_read: ["Sunshine 日志已读取", "Sunshine logs read"], diagnostics_read: ["诊断已读取", "Diagnostics read"], virtual_input_status_read: ["虚拟输入状态已读取", "Virtual input status read"],
  maintenance_completed: ["维护已完成", "Maintenance completed"], service_status_read: ["服务状态已读取", "Service status read"], service_controlled: ["服务控制已完成", "Service control completed"],
  resource_conflict: ["资源修订冲突", "Resource revision conflict"], unsupported_capability: ["客户端能力不支持", "Client capability unsupported"],
  side_effect_not_confirmed: ["副作用结果未确认", "Side effect not confirmed"], service_transition_not_confirmed: ["服务状态转换未确认", "Service transition not confirmed"],
  device_revoked: ["实例已撤销", "Instance revoked"],
  authorization_rotated: ["实例密码已更换", "Instance password changed"],
  true: ["启用", "Enabled"], false: ["禁用", "Disabled"], info: ["信息", "Information"], warning: ["警告", "Warning"],
  error: ["错误", "Error"], fatal: ["致命错误", "Fatal"],
  ultrafast: ["极快", "Ultrafast"], superfast: ["超快", "Superfast"], veryfast: ["很快", "Very fast"], faster: ["较快", "Faster"],
  fast: ["快速", "Fast"], medium: ["中等", "Medium"], slow: ["慢速", "Slow"], slower: ["较慢", "Slower"], veryslow: ["很慢", "Very slow"],
};
export function operationLabel(value: string): string {
  const key = value.replace(/^sunshine\./, "");
  const label = Object.hasOwn(labels, key) ? labels[key] : undefined;
  return label ? t(...label) : t("未识别的状态", "Unrecognized state");
}
export function configValueLabel(value: string): string {
  const label = Object.hasOwn(labels, value) ? labels[value] : undefined;
  return label ? t(...label) : value;
}
