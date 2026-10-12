import { t, getLocale } from "@xcss/web/admin-ui/i18n";
import { Button, EmptyState, Table } from "@xcss/web/admin-ui";
import { ErrorState } from "@xcss/web/admin-ui";
import { errorRequestId, useAdminApplication } from "@xcss/web/admin-shell";
import { useState } from "react";
import { CURRENT_API_PREFIX, type DeviceInfo } from "./api";

const configurationStates: Record<string, string> = {
  unknown: t("尚未核对", "Not yet checked"),
  awaiting_restart: t("已保存，等待重启", "Saved; waiting for restart"),
  pending_verification: t("运行时生效待验证", "Runtime effect needs verification"),
  drift_detected: t("配置存在差异", "Configuration differs"),
};
function lastSeen(micros: number | null) {
  if (micros === null) return t("尚未连接", "Never connected");
  const date = new Date(micros / 1000);
  return Number.isFinite(date.getTime()) ? date.toLocaleString(getLocale(), { timeZone: "UTC", timeZoneName: "short" }) : t("未知", "Unknown");
}
export function DeviceInstances({ devices, select, removed }: { devices: DeviceInfo[]; select(id: string): void; removed(id: string): void }) {
  const { client, notify } = useAdminApplication();
  const [candidate, setCandidate] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [failure, setFailure] = useState<{ requestId?: string } | null>(null);
  async function remove(device: DeviceInfo) {
    if (pending) return;
    setPending(device.id); setFailure(null);
    try {
      await client.request(`${CURRENT_API_PREFIX}/sunshine/devices/${device.id}`, (value): value is undefined => value === undefined, { method: "DELETE" });
      setCandidate(null); removed(device.id); notify(t("实例已删除", "Instance deleted"));
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
    finally { setPending(null); }
  }
  if (!devices.length) return <EmptyState>{t("暂无实例，请新建并注册 客户端。", "No instances yet. Create an instance and register a client.")}</EmptyState>;
  return <>{failure && <ErrorState requestId={failure.requestId}>{t("删除未能确认，请刷新实例列表核对。", "Deletion could not be confirmed. Refresh and check the instance list.")}</ErrorState>}<Table aria-label={t("Sunshine 实例列表", "Sunshine instance list")}><thead><tr>
    <th scope="col">{t("实例名称", "Instance name")}</th><th scope="col">{t("注册状态", "Registration status")}</th><th scope="col">{t("客户端 状态", "Client status")}</th>
    <th scope="col">{t("Sunshine 接口", "Sunshine API")}</th><th scope="col">{t("配置状态", "Configuration status")}</th><th scope="col">{t("操作系统/架构", "Operating system / architecture")}</th><th scope="col">{t("最近连接", "Last connection")}</th><th scope="col">{t("删除", "Delete")}</th>
  </tr></thead><tbody>{devices.map(device => <tr key={device.id}>
    <th scope="row"><a className="xcss-instance-link" aria-label={t("选择实例 {0}", "Select instance {0}", [device.name])} href={`#details/${device.id}`} onClick={() => select(device.id)}>{device.name}</a></th>
    <td>{device.revoked ? t("凭据已撤销", "Credentials revoked") : device.registered ? t("已注册", "Registered") : device.pairing_pending ? t("等待配对", "Waiting for pairing") : t("配对已取消", "Pairing cancelled")}</td>
    <td>{device.client_online ? t("在线", "Online") : t("离线", "Offline")}</td>
    <td>{device.sunshine_reachable === null ? t("未知", "Unknown") : device.sunshine_reachable ? t("可访问", "Reachable") : t("不可访问", "Unreachable")}</td>
    <td>{configurationStates[device.configuration_state] ?? t("待核对", "Needs review")}</td>
    <td>{device.capabilities?.os ?? t("尚未上报", "Not yet reported")}</td><td>{lastSeen(device.last_seen_at_micros)}</td><td><div className="xcss-actions">{candidate === device.id ? <><Button disabled={pending !== null} onClick={() => setCandidate(null)}>{t("取消", "Cancel")}</Button><Button className="xcss-danger" disabled={pending !== null} onClick={() => void remove(device)}>{pending === device.id ? t("正在删除…", "Deleting…") : t("确认删除", "Confirm delete")}</Button></> : <Button disabled={pending !== null} onClick={() => setCandidate(device.id)}>{t("删除", "Delete")}</Button>}</div></td>
  </tr>)}</tbody></Table></>;
}
