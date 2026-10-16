import {useEffect,useState} from "react";
import {Button,ErrorState,LoadingState} from "@xcss/web/admin-ui";
import {t} from "@xcss/web/admin-ui/i18n";
import {errorRequestId,useAdminApplication} from "@xcss/web/admin-shell";
import {CURRENT_API_PREFIX,isAccessRules,type AccessRules,type DeviceInfo} from "./api";

export function AccessEditor({device,devices,changed}:{device:DeviceInfo;devices:DeviceInfo[];changed():void}){
 const {client}=useAdminApplication();
 const [rules,setRules]=useState<AccessRules|null>(null);
 const [selected,setSelected]=useState<string[]>([]);
 const [busy,setBusy]=useState(false);
 const [reload,setReload]=useState(0);
 const [failure,setFailure]=useState<{requestId?:string}|null>(null);
 const [saved,setSaved]=useState(false);
 useEffect(()=>{let active=true;const controller=new AbortController();setRules(null);setSaved(false);setFailure(null);
  void client.request(`${CURRENT_API_PREFIX}/devices/${device.id}/access`,isAccessRules,{signal:controller.signal}).then(value=>{if(active){setRules(value);setSelected(value.peer_ids)}}).catch(error=>{if(active)setFailure({requestId:errorRequestId(error)})});
  return()=>{active=false;controller.abort()};
 },[client,device.id,reload]);
 async function save(){if(busy||!rules)return;setBusy(true);setFailure(null);setSaved(false);try{
  const value=await client.request(`${CURRENT_API_PREFIX}/devices/${device.id}/access`,isAccessRules,{method:"PUT",body:JSON.stringify({peer_ids:selected,expected_revision:rules.revision})});
  setRules(value);setSelected(value.peer_ids);setSaved(true);changed();
 }catch(error){setFailure({requestId:errorRequestId(error)})}finally{setBusy(false)}}
 const candidates=devices.filter(peer=>peer.role!==device.role&&(!peer.revoked||selected.includes(peer.id)));
 return <section className="xcss-content-stack" aria-label={t("控制权限","Control permissions")}>
  <h3>{device.role==="controller"?t("允许控制的被控主机","Hosts this controller may control"):t("允许连接的控制端","Controllers allowed to connect")}</h3>
  <p>{t("两侧编辑同一份授权关系。未勾选即不允许，每个实例最多 32 个对端。","Both sides edit the same permission. Unselected peers are denied; up to 32 peers per instance.")}</p>
  {failure&&<ErrorState requestId={failure.requestId}>{t("授权保存或读取未能确认；请重新读取后核对。并发修改时需重新选择。","Could not confirm permissions. Reload and review; concurrent edits require a new selection.")}</ErrorState>}
  {rules===null?<LoadingState>{t("正在读取授权…","Loading permissions…")}</LoadingState>:<fieldset disabled={busy||device.revoked}><legend>{t("授权对端","Allowed peers")}</legend>{candidates.length===0?<p>{t("请先创建另一角色的实例。","Create an instance with the opposite role first.")}</p>:candidates.map(peer=><label className="rustdesk-access-peer" key={peer.id}><input type="checkbox" checked={selected.includes(peer.id)} disabled={!selected.includes(peer.id)&&(selected.length>=32||peer.revoked)} onChange={event=>{setSelected(ids=>event.target.checked?[...ids,peer.id]:ids.filter(id=>id!==peer.id));setSaved(false)}}/>{peer.name} {peer.revoked?t("（已撤销，请移除）"," (revoked; remove)"):!peer.registered?t("（等待绑定）"," (awaiting enrollment)"):""}</label>)}</fieldset>}
  <div className="xcss-actions"><Button disabled={busy||!rules||device.revoked} onClick={()=>void save()}>{t("保存控制权限","Save control permissions")}</Button><Button disabled={busy} onClick={()=>setReload(v=>v+1)}>{t("重新读取授权","Reload permissions")}</Button></div>
  {saved&&<p role="status">{t("控制权限已保存。在线客户端将在 5 秒内更新。","Permissions saved. Connected clients update within 5 seconds.")}</p>}
 </section>;
}
