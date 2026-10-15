import { t, getLocale } from "@xcss/web/admin-ui/i18n";
import { Button, EmptyState, ErrorState, Table } from "@xcss/web/admin-ui";
import { errorRequestId, useAdminApplication } from "@xcss/web/admin-shell";
import { useState } from "react";
import { CURRENT_API_PREFIX, type DeviceInfo } from "./api";
export function integrationLabel(device:DeviceInfo):string {
 const state=device.diagnostics?.integration_state;
 return state==="ready"?t("已接入","Integrated"):state==="unavailable"?t("主机暂不可用","Host unavailable"):t("等待 RustDesk 稳定版本接入","Awaiting stable RustDesk integration");
}
export function DeviceInstances({devices,select,removed}:{devices:DeviceInfo[];select(id:string):void;removed(id:string):void}){
 const {client}=useAdminApplication();const[candidate,setCandidate]=useState<string|null>(null);const[pending,setPending]=useState(false);const[failure,setFailure]=useState<{requestId?:string}|null>(null);
 async function remove(device:DeviceInfo){if(pending)return;setPending(true);setFailure(null);try{await client.request(`${CURRENT_API_PREFIX}/devices/${device.id}`,(v):v is undefined=>v===undefined,{method:"DELETE"});setCandidate(null);removed(device.id)}catch(error){setFailure({requestId:errorRequestId(error)})}finally{setPending(false)}}
 if(!devices.length)return <EmptyState>{t("暂无实例。新建实例后，由 RustDesk 内嵌客户端绑定。","No instances. Create one for the embedded RustDesk client to enroll.")}</EmptyState>;
 return <>{failure&&<ErrorState requestId={failure.requestId}>{t("删除未能确认，请刷新核对。","Deletion could not be confirmed. Refresh to check.")}</ErrorState>}<Table aria-label={t("RustDesk 实例","RustDesk instances")}><thead><tr><th>{t("名称","Name")}</th><th>{t("管理连接","Management connection")}</th><th>{t("RustDesk 状态","RustDesk status")}</th><th>{t("平台","Platform")}</th><th>{t("最近连接","Last seen")}</th><th>{t("操作","Actions")}</th></tr></thead><tbody>{devices.map(d=><tr key={d.id}><th><a href={`#details/${d.id}`} onClick={()=>select(d.id)}>{d.name}</a></th><td>{d.revoked?t("已撤销","Revoked"):d.client_online?t("在线","Online"):d.registered?t("离线","Offline"):t("等待绑定","Awaiting enrollment")}</td><td>{integrationLabel(d)}</td><td>{d.capabilities?.os??"—"}</td><td>{d.last_seen_at_micros===null?"—":new Date(d.last_seen_at_micros/1000).toLocaleString(getLocale(),{timeZone:"UTC",timeZoneName:"short"})}</td><td>{candidate===d.id?<div className="xcss-actions"><Button disabled={pending} onClick={()=>setCandidate(null)}>{t("取消","Cancel")}</Button><Button disabled={pending} onClick={()=>void remove(d)}>{t("确认删除","Confirm delete")}</Button></div>:<Button disabled={pending} onClick={()=>setCandidate(d.id)}>{t("删除","Delete")}</Button>}</td></tr>)}</tbody></Table></>;
}
