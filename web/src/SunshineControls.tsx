import {useEffect,useRef,useState,type FormEvent} from "react";
import {Button,FormField,TextField} from "@xcss/web/admin-ui";
import {t} from "@xcss/web/admin-ui/i18n";
import type {ApplicationRef,ApplicationSpec,ApplicationsSnapshot,Command,DeviceInfo,DiagnosticSnapshot,LogPage,Operation,PairedClientsSnapshot,Report,VirtualInputStatus} from "./api";
import {ConfigSelect} from "./ConfigSelect";
import {supportsPendingPairingListing} from "./api";

type Confirm={title:string;description:string;run():Promise<unknown>};
export const sunshineManagementCategories=[
 {id:"applications",label:t("应用管理", "Applications")},
 {id:"diagnostics",label:t("诊断与 Sunshine 日志", "Diagnostics and Sunshine logs")},
 {id:"maintenance",label:t("显示与输入维护", "Display and input maintenance")},
] as const;
export type SunshineManagementCategory=typeof sunshineManagementCategories[number]["id"];
type SunshineControlSection=SunshineManagementCategory|"service"|"moonlight";
export function isSunshineManagementCategory(value:string):value is SunshineManagementCategory{return sunshineManagementCategories.some(category=>category.id===value)}
type Props={device:DeviceInfo;operations:Operation[];busy:boolean;blocked:boolean;section:SunshineControlSection|null;submit(command:Command,onAccepted?:(operation:Operation)=>void):Promise<boolean>;confirm(value:Confirm):void};

const emptyApplication:ApplicationSpec={name:"",output:"",cmd:"","working-dir":"","exclude-global-prep-cmd":false,elevated:false,"auto-detach":true,"wait-all":true,"exit-timeout":5,"prep-cmd":[],detached:[],"image-path":""};
function latestOf(operations:Operation[],kinds:string[]):Report|undefined{
 let latest:{time:number;report:Report}|undefined;
 for(const operation of operations){
  // A successful direct result is immutable. A human resolution only changes
  // the operation timestamp; reconciliation has its own receipt timestamp.
  for(const [report,time] of [[operation.result,operation.updated_at_micros],[operation.reconciliation,operation.reconciliation_observed_at_micros]] as const){
   if(report&&kinds.includes(report.kind)&&typeof time==="number"&&Number.isSafeInteger(time)&&(!latest||time>latest.time))latest={time,report};
  }
 }
 return latest?.report;
}
function latestAttempt(operations:Operation[],actions:string[]):Operation|undefined{
 return operations.filter(operation=>actions.includes(operation.action)).reduce<Operation|undefined>((latest,operation)=>!latest||operation.created_at_micros>latest.created_at_micros?operation:latest,undefined);
}
function failedAttempt(operation:Operation|undefined){return !!operation&&["failed","dead_letter","unknown"].includes(operation.state)}
function pendingAttempt(operation:Operation|undefined){return !!operation&&["pending","running"].includes(operation.state)}
function applicationIdentity(value:ApplicationSpec):string{
 // Match the saved specification without relying on JSON object key ordering.
 return JSON.stringify([value.name,value.output,value.cmd,value["working-dir"],value["exclude-global-prep-cmd"],value.elevated,value["auto-detach"],value["wait-all"],value["exit-timeout"],value["prep-cmd"].map(command=>[command.do,command.undo,command.elevated]),value.detached,value["image-path"]]);
}
function latest(operations:Operation[],kind:string):Report|undefined{return latestOf(operations,[kind])}
function applications(value:Report|undefined):ApplicationsSnapshot|undefined{const snapshot=value?.snapshot;return snapshot&&"applications" in snapshot?snapshot:undefined}
function pairedClients(value:Report|undefined):PairedClientsSnapshot|undefined{const snapshot=value?.snapshot;return snapshot&&"clients" in snapshot?snapshot:undefined}
function diagnostics(value:Report|undefined):DiagnosticSnapshot|undefined{const snapshot=value?.snapshot;return snapshot&&"api_reachable" in snapshot?snapshot:undefined}
function appHasHostCommands(value:ApplicationSpec){return Boolean(value.output||value.cmd||value["working-dir"]||value["exclude-global-prep-cmd"]||value.elevated||value["prep-cmd"].length||value.detached.length)}
function fileBase64(file:File):Promise<string>{return file.arrayBuffer().then(buffer=>{const bytes=new Uint8Array(buffer);let binary="";for(let offset=0;offset<bytes.length;offset+=0x8000)binary+=String.fromCharCode(...bytes.subarray(offset,offset+0x8000));return btoa(binary)})}

function ApplicationEditor({application,setApplication,detachedText,setDetachedText,editing,blocked,onSubmit,onCancel}:{application:ApplicationSpec;setApplication(value:ApplicationSpec):void;detachedText:string;setDetachedText(value:string):void;editing:boolean;blocked:boolean;onSubmit(event:FormEvent<HTMLFormElement>):void;onCancel():void}){
 const tooManyDetached=detachedText.split("\n").filter(value=>value.trim()).length>16;
 return <form className="sunshine-application-editor" onSubmit={onSubmit}>
  <h3>{editing?t("编辑应用", "Edit application"):t("新建应用", "New application")}</h3>
  <div className="sunshine-editor-fields">
  <FormField label={t("名称", "Name")}><TextField required maxLength={128} value={application.name} onChange={event=>setApplication({...application,name:event.target.value})}/></FormField>
  <FormField label={t("封面路径", "Image path")}><TextField maxLength={4096} value={application["image-path"]} onChange={event=>setApplication({...application,"image-path":event.target.value})}/></FormField>
  <FormField label={t("启动命令", "Launch command")}><TextField maxLength={4096} value={application.cmd} onChange={event=>setApplication({...application,cmd:event.target.value})}/></FormField>
  <FormField label={t("工作目录", "Working directory")}><TextField maxLength={4096} value={application["working-dir"]} onChange={event=>setApplication({...application,"working-dir":event.target.value})}/></FormField>
  <FormField label={t("输出路径", "Output path")}><TextField maxLength={4096} value={application.output} onChange={event=>setApplication({...application,output:event.target.value})}/></FormField>
  <FormField label={t("退出等待秒数", "Exit timeout seconds")}><TextField type="number" min={0} max={3600} value={String(application["exit-timeout"])} onChange={event=>setApplication({...application,"exit-timeout":Number(event.target.value)})}/></FormField>
  </div>
  <fieldset className="sunshine-run-options"><legend>{t("运行选项", "Run options")}</legend>
  <div className="sunshine-checkbox-grid"><label><input type="checkbox" checked={application["exclude-global-prep-cmd"]} onChange={event=>setApplication({...application,"exclude-global-prep-cmd":event.target.checked})}/>{t("排除全局准备命令", "Exclude global preparation commands")}</label>
  <label><input type="checkbox" checked={application.elevated} onChange={event=>setApplication({...application,elevated:event.target.checked})}/>{t("以提升权限运行", "Run elevated")}</label>
  <label><input type="checkbox" checked={application["auto-detach"]} onChange={event=>setApplication({...application,"auto-detach":event.target.checked})}/>{t("自动分离", "Auto detach")}</label>
  <label><input type="checkbox" checked={application["wait-all"]} onChange={event=>setApplication({...application,"wait-all":event.target.checked})}/>{t("等待全部进程", "Wait for all processes")}</label></div>
  </fieldset>
  <div className="sunshine-detached-commands"><FormField label={t("分离命令（每行一条）", "Detached commands (one per line)")}><textarea aria-label={t("分离命令（每行一条）", "Detached commands (one per line)")} className="sunshine-wrap" rows={4} maxLength={8192} value={detachedText} aria-invalid={tooManyDetached||undefined} onChange={event=>setDetachedText(event.target.value)}/></FormField>
  {tooManyDetached&&<p role="alert">{t("最多允许 16 条分离命令。", "Use at most 16 detached commands.")}</p>}</div>
  <fieldset className="sunshine-preparation"><legend>{t("准备命令", "Preparation commands")}</legend>{application["prep-cmd"].map((command,index)=><div className="sunshine-preparation-row" key={index}><FormField label={t("执行", "Do")}><TextField maxLength={4096} value={command.do} onChange={event=>{const commands=[...application["prep-cmd"]];commands[index]={...command,do:event.target.value};setApplication({...application,"prep-cmd":commands})}}/></FormField><FormField label={t("撤销", "Undo")}><TextField maxLength={4096} value={command.undo} onChange={event=>{const commands=[...application["prep-cmd"]];commands[index]={...command,undo:event.target.value};setApplication({...application,"prep-cmd":commands})}}/></FormField><label><input type="checkbox" checked={command.elevated} onChange={event=>{const commands=[...application["prep-cmd"]];commands[index]={...command,elevated:event.target.checked};setApplication({...application,"prep-cmd":commands})}}/>{t("提升权限", "Elevated")}</label><Button type="button" onClick={()=>setApplication({...application,"prep-cmd":application["prep-cmd"].filter((_,position)=>position!==index)})}>{t("删除准备命令", "Remove preparation command")}</Button></div>)}<Button type="button" disabled={application["prep-cmd"].length>=16} onClick={()=>setApplication({...application,"prep-cmd":[...application["prep-cmd"],{do:"",undo:"",elevated:false}]})}>{t("添加准备命令", "Add preparation command")}</Button></fieldset>
  <div className="xcss-actions sunshine-form-actions"><Button className="sunshine-primary-action" type="submit" disabled={blocked||tooManyDetached||!application.name.trim()||application["prep-cmd"].some(command=>!command.do.trim()&&!command.undo.trim())}>{editing?t("保存应用", "Save application"):t("创建应用", "Create application")}</Button><Button type="button" onClick={onCancel}>{t("取消", "Cancel")}</Button></div>
 </form>
}

function MoonlightPairing({device,operations,busy,blocked,submit,confirm}:Pick<Props,"device"|"operations"|"busy"|"blocked"|"submit"|"confirm">){
 const pairings=latest(operations,"pending_pairings_read")?.pairings;
 const clients=pairedClients(latestOf(operations,["paired_clients_read","paired_client_updated"]));
 const latestPendingRead=operations.filter(operation=>operation.action==="sunshine.pairing.pending.list").reduce<Operation|undefined>((current,operation)=>!current||operation.created_at_micros>current.created_at_micros?operation:current,undefined);
 const pendingReadFailed=latestPendingRead&&["failed","cancelled","unknown"].includes(latestPendingRead.state);
 const latestSubmit=operations.filter(operation=>operation.action==="sunshine.pairing.pin.submit").reduce<Operation|undefined>((current,operation)=>!current||operation.created_at_micros>current.created_at_micros?operation:current,undefined);
 const [selected,setSelected]=useState("");const [pin,setPin]=useState("");const [name,setName]=useState("");const [submitted,setSubmitted]=useState(false);
 const [refreshQueue,setRefreshQueue]=useState<Command[]>([]);
 const [refreshing,setRefreshing]=useState(false);const [refreshFailed,setRefreshFailed]=useState(false);const [pairSubmitting,setPairSubmitting]=useState(false);const [submitFailed,setSubmitFailed]=useState(false);const refreshInFlight=useRef(false);
 const initialLoad=useRef(false);const previousSubmit=useRef<string|undefined>(undefined);const refreshedSubmission=useRef<string|undefined>(undefined);const submitRef=useRef(submit);submitRef.current=submit;
 const outcome=submitted&&latestSubmit?.operation_id!==previousSubmit.current?latestSubmit:undefined;
 const reading=busy||refreshing||refreshQueue.length>0||pairSubmitting;
 useEffect(()=>{if(initialLoad.current||busy||!device.registered||device.revoked)return;initialLoad.current=true;void submitRef.current({kind:"list_pending_pairings"}).then(accepted=>{if(!accepted)setRefreshFailed(true)})},[busy,device.registered,device.revoked]);
 useEffect(()=>{if(pairings?.some(pairing=>pairing.id===selected))return;const first=pairings?.[0];setSelected(first?.id??"");setName(first?.name||"Moonlight");setPin("")},[pairings,selected]);
 useEffect(()=>{if(outcome?.result?.kind!=="pairing_pin_submitted"||refreshedSubmission.current===outcome.operation_id)return;refreshedSubmission.current=outcome.operation_id;setRefreshFailed(false);setRefreshQueue([{kind:"list_pending_pairings"},{kind:"list_paired_clients"}])},[outcome]);
 useEffect(()=>{if(busy||!refreshQueue.length||refreshInFlight.current)return;const [command,...rest]=refreshQueue;refreshInFlight.current=true;setRefreshing(true);void submitRef.current(command).then(accepted=>{if(accepted)setRefreshQueue(rest);else{setRefreshQueue([]);setRefreshFailed(true)}}).catch(()=>{setRefreshQueue([]);setRefreshFailed(true)}).finally(()=>{refreshInFlight.current=false;setRefreshing(false)})},[busy,refreshQueue,refreshing]);
 function refresh(command:Command){setRefreshFailed(false);void submit(command).then(accepted=>{if(!accepted)setRefreshFailed(true)})}
 function choose(id:string){setSelected(id);setName(pairings?.find(pairing=>pairing.id===id)?.name||"Moonlight");setPin("");setSubmitted(false)}
 async function pair(event:FormEvent<HTMLFormElement>){event.preventDefault();if(!selected||blocked||reading||!pairings?.some(pairing=>pairing.id===selected))return;previousSubmit.current=latestSubmit?.operation_id;setSubmitted(false);setSubmitFailed(false);setPairSubmitting(true);try{const accepted=await submit({kind:"submit_pairing_pin",pairing_id:selected,pin,name:name.trim()});if(accepted){setSubmitted(true);setPin("")}else setSubmitFailed(true)}finally{setPairSubmitting(false)}}
 return <section id="sunshine-moonlight-pairing" className="xcss-content-panel sunshine-management-panel" aria-label={t("Moonlight 配对", "Moonlight pairing")}>
  <header className="sunshine-section-header"><div className="xcss-actions"><Button disabled={reading} onClick={()=>refresh({kind:"list_pending_pairings"})}>{t("刷新配对请求", "Refresh pairing requests")}</Button><Button disabled={reading} onClick={()=>refresh({kind:"list_paired_clients"})}>{t("刷新已配对客户端", "Refresh paired clients")}</Button><Button className="sunshine-danger-action" disabled={blocked||reading||!clients?.clients.length} onClick={()=>confirm({title:t("取消全部 Moonlight 配对", "Unpair all Moonlight clients"),description:t("所有 Moonlight 客户端都需要重新配对，活动会话可能中断。", "Every Moonlight client will need to pair again and active sessions may be interrupted."),run:()=>submit({kind:"unpair_all_clients",administrator_confirmed:true})})}>{t("全部取消配对", "Unpair all")}</Button></div></header>
  <form className="sunshine-pairing-form" onSubmit={event=>void pair(event)}><p>{t("在 Moonlight 中添加这台 Sunshine 主机并发起配对，然后刷新配对请求，选择对应客户端并输入 Moonlight 显示的 PIN。", "Add this Sunshine host in Moonlight and start pairing. Refresh pairing requests, select the client, and enter the PIN shown in Moonlight.")}</p>
   {refreshFailed&&<p role="alert">{t("列表刷新未完成，请手动刷新；已确认的配对结果保持不变。", "The lists were not refreshed. Refresh them manually; the confirmed pairing result is unchanged.")}</p>}
   {pendingReadFailed&&<p role="alert">{t("未能读取配对请求，请确认 Sunshine 可访问后再次刷新。", "Could not read pairing requests. Check that Sunshine is reachable and refresh again.")}</p>}
   {!pairings&&!pendingReadFailed?<p className="sunshine-muted">{t("正在读取配对请求…", "Loading pairing requests…")}</p>:pairings&&!pairings.length?<p className="sunshine-muted">{t("暂无配对请求，请先在 Moonlight 中发起配对。", "No pending requests. Start pairing in Moonlight first.")}</p>:null}
   <div className="sunshine-pairing-fields"><div className="xcss-form-field"><label id="moonlight-pairing-client-label" htmlFor="moonlight-pairing-client">{t("待配对客户端", "Pending client")}</label><ConfigSelect id="moonlight-pairing-client" value={selected} disabled={reading||!pairings?.length} options={pairings?.length?pairings.map(pairing=>({value:pairing.id,label:`${pairing.name||"Moonlight"} · ${pairing.address}`})):[{value:"",label:t("暂无配对请求", "No pending requests")}]} onChange={choose}/></div>
    <FormField label={t("PIN（4 位数字）", "PIN (4 digits)")}><TextField name="pin" inputMode="numeric" autoComplete="one-time-code" required pattern="[0-9]{4}" maxLength={4} value={pin} disabled={reading||!selected} onChange={event=>{setPin(event.target.value);setSubmitted(false);setSubmitFailed(false)}}/></FormField>
    <FormField label={t("客户端名称", "Client name")}><TextField name="name" required maxLength={128} value={name} disabled={reading||!selected} onChange={event=>{setName(event.target.value);setSubmitted(false);setSubmitFailed(false)}}/></FormField>
    <Button className="sunshine-primary-action" type="submit" disabled={blocked||reading||!selected||!/^\d{4}$/.test(pin)||!name.trim()||new TextEncoder().encode(name.trim()).length>128}>{t("配对", "Pair")}</Button>
   </div>
   {submitFailed&&<p role="alert">{t("配对请求未获确认，PIN 已保留。请先检查日志中的任务结果。", "The pairing request was not acknowledged and the PIN was kept. Check the task result in Logs first.")}</p>}
   {outcome?.result?.kind==="pairing_pin_submitted"?<p role="status">{t("Moonlight 已配对。", "Moonlight paired.")}</p>:outcome?.result?.reason==="pairing_failed"?<p role="alert">{t("配对未完成。请在 Moonlight 中重新发起配对，刷新请求并核对 PIN。", "Pairing did not complete. Start pairing again in Moonlight, refresh requests, and check the PIN.")}</p>:outcome?.state==="unknown"?<p role="alert">{t("配对结果未确认，请核对 Moonlight 的实际状态；本次 PIN 不会自动重复提交。", "Pairing is unconfirmed. Check Moonlight; this PIN will not be submitted again automatically.")}</p>:outcome&&["pending","running"].includes(outcome.state)?<p role="status">{t("正在等待 Moonlight 完成配对…", "Waiting for Moonlight to complete pairing…")}</p>:null}
  </form>
  <h3>{t("已配对客户端", "Paired clients")}</h3>{!clients?<p className="sunshine-muted">{t("刷新列表以查看已配对客户端。", "Refresh the list to view paired clients.")}</p>:!clients.clients.length?<p className="sunshine-muted">{t("暂无已配对客户端。", "No paired clients yet.")}</p>:<div className="sunshine-management-list">{clients.clients.map(value=><article className="sunshine-management-row" key={value.uuid}><div className="sunshine-row-summary"><div className="xscc-title"><h4>{value.name}</h4><span className="xscc-status" data-enabled={value.enabled}>{value.enabled?t("已启用", "Enabled"):t("已禁用", "Disabled")}</span></div><code className="sunshine-muted">{value.uuid}</code></div><div className="xcss-actions"><Button disabled={blocked||reading} onClick={()=>{const command:Command={kind:"set_paired_client_enabled",uuid:value.uuid,enabled:!value.enabled,administrator_confirmed:true};if(value.enabled)confirm({title:t("禁用 Moonlight 客户端", "Disable Moonlight client"),description:t("禁用会终止该客户端对应的活动会话。", "Disabling terminates active sessions for this client."),run:()=>submit(command)});else void submit(command)}}>{value.enabled?t("禁用", "Disable"):t("启用", "Enable")}</Button><Button className="sunshine-danger-action" disabled={blocked||reading} onClick={()=>confirm({title:t("取消 Moonlight 配对", "Unpair Moonlight client"),description:t("该客户端需要重新配对，活动会话可能中断。", "This client will need to pair again and its active session may be interrupted."),run:()=>submit({kind:"unpair_client",uuid:value.uuid,administrator_confirmed:true})})}>{t("取消配对", "Unpair")}</Button></div></article>)}</div>}
 </section>;
}

export function SunshineControls({device,operations,busy,blocked,section,submit,confirm}:Props){
 const caps=device.capabilities;
 const appSnapshot=applications(latestOf(operations,["applications_read","application_saved","application_deleted"]));
 const applicationAttempt=latestAttempt(operations.filter(operation=>{
  if(operation.action==="sunshine.applications.list")return true;
  const report=operation.state==="succeeded"?operation.result:operation.state==="resolved"&&operation.resolution==="confirmed_succeeded"?operation.reconciliation:undefined;
  return (report?.kind==="application_saved"||report?.kind==="application_deleted")&&!!applications(latestOf([operation],[report.kind]));
 }),["sunshine.applications.list","sunshine.applications.save","sunshine.applications.delete"]);
 const applicationReadAttempt=applicationAttempt?.action==="sunshine.applications.list"?applicationAttempt:undefined;
 const applicationReadFailed=failedAttempt(applicationReadAttempt)||(applicationReadAttempt?.state==="resolved"&&(applicationReadAttempt.resolution!=="confirmed_succeeded"||applicationReadAttempt.reconciliation?.kind!=="applications_read"||!applications(latest([applicationReadAttempt],"applications_read"))));
 const logPage=latest(operations,"logs_read")?.page as LogPage|undefined;
 const diagnostic=diagnostics(latest(operations,"diagnostics_read"));
 const virtualStatus=latest(operations,"virtual_input_status_read")?.status as VirtualInputStatus|undefined;
 const serviceState=latestOf(operations,["service_controlled","service_status_read"])?.state;
 const serviceAttempt=latestAttempt(operations,["sunshine.service.read","sunshine.service.control"]);
 const diagnosticAttempt=latestAttempt(operations,["sunshine.diagnostics.read"]);

 const uploadedCover=latest(operations,"cover_uploaded")?.path;
 const[editing,setEditing]=useState<ApplicationRef|null|undefined>(undefined);const[application,setApplication]=useState<ApplicationSpec>(emptyApplication);const[detachedText,setDetachedText]=useState("");
 const editorGeneration=useRef(0);const submitting=useRef(false);
 const[submission,setSubmission]=useState<{operation:Operation;application:ApplicationSpec;generation:number}|null>(null);
 const submittedOperation=submission&&(operations.find(operation=>operation.operation_id===submission.operation.operation_id&&operation.updated_at_micros>=submission.operation.updated_at_micros)??submission.operation);
 const savedApplicationReport=submittedOperation?.state==="succeeded"?submittedOperation.result:submittedOperation?.state==="resolved"&&submittedOperation.resolution==="confirmed_succeeded"?submittedOperation.reconciliation:undefined;
 // Keep submit disabled through the success render until its new target is adopted.
 const applicationPending=!!submittedOperation&&(["pending","running","unknown","succeeded"].includes(submittedOperation.state)||savedApplicationReport?.kind==="application_saved");
 useEffect(()=>{
  if(!submission||!submittedOperation||submission.generation!==editorGeneration.current)return;
  if(savedApplicationReport?.kind!=="application_saved")return;
  const snapshot=applications(savedApplicationReport);
  const matches=snapshot?.applications.filter(value=>applicationIdentity(value.specification)===applicationIdentity(submission.application));
  if(matches?.length!==1)return;
  // Rebase the identity only. Text edited while execution was pending belongs
  // to the user and must survive this earlier save's completion.
  setEditing(matches[0].reference);setSubmission(null);
 },[submission,submittedOperation,savedApplicationReport]);
 function edit(reference:ApplicationRef|null){editorGeneration.current+=1;setSubmission(null);const value=reference?appSnapshot?.applications.find(value=>value.reference.fingerprint===reference.fingerprint)?.specification??emptyApplication:emptyApplication;setEditing(reference);setApplication(value);setDetachedText(value.detached.join("\n"))}
 function cancelApplication(){editorGeneration.current+=1;setSubmission(null);setEditing(undefined)}
 async function saveApplication(event:FormEvent<HTMLFormElement>){
  event.preventDefault();if(!appSnapshot||blocked||applicationPending||submitting.current)return;
  const value={...application,detached:detachedText.split("\n").map(line=>line.trim()).filter(Boolean)};if(value.detached.length>16)return;
  const generation=editorGeneration.current;
  const command:Command={kind:"save_application",expected_revision:appSnapshot.revision,target:editing??null,application:value,administrator_confirmed_host_commands:appHasHostCommands(value)};
  const save=async()=>{
   if(generation!==editorGeneration.current||submitting.current)return false;
   submitting.current=true;
   try{return await submit(command,operation=>{if(generation===editorGeneration.current)setSubmission({operation,application:value,generation})})}
   finally{submitting.current=false}
  };
  if(appHasHostCommands(value))confirm({title:t("确认保存主机命令", "Confirm host commands"),description:t("应用的启动、准备或分离命令会在 Sunshine 主机上执行程序。", "Application launch, preparation or detached commands execute programs on the Sunshine host."),run:save});else await save();
 }
 const sectionLabel=section==="service"?t("Sunshine 服务", "Sunshine service"):section==="moonlight"?t("Moonlight 配对", "Moonlight pairing"):sunshineManagementCategories.find(category=>category.id===section)?.label;
 if(!caps||caps.protocol!=="xscs-management/1")return section?<section id={section==="moonlight"?"sunshine-moonlight-pairing":`sunshine-${section}`} className="xcss-content-panel" aria-label={sectionLabel}><p>{t("客户端尚未上报当前协议 能力。升级并重新连接客户端后可使用应用、配对、诊断和服务控制。", "The client has not reported the current protocol capabilities. Upgrade and reconnect it to use applications, pairing, diagnostics and service controls.")}</p></section>:null;
 const sectionAvailable=section==="applications"?caps.application_management:section==="diagnostics"?caps.diagnostics:section==="maintenance"?caps.maintenance:section==="service"?caps.service_control:true;
 return <>
 {section==="moonlight"&&(caps.moonlight_pairing_management&&supportsPendingPairingListing(caps)?<MoonlightPairing key={device.id} device={device} operations={operations} busy={busy} blocked={blocked} submit={submit} confirm={confirm}/>:<section id="sunshine-moonlight-pairing" className="xcss-content-panel" aria-label={sectionLabel}><p>{t("客户端尚未报告待配对请求查询能力。", "The client has not reported pending pairing request support.")}</p></section>)}
 {section&&!sectionAvailable&&<section id={`sunshine-${section}`} className="xcss-content-panel" aria-label={sectionLabel}><p>{t("客户端尚未启用此项管理能力。", "This management capability is not enabled on the client.")}</p></section>}
 {caps.application_management&&<section id="sunshine-applications" className="xcss-content-panel sunshine-management-panel" hidden={section!=="applications"} aria-label={t("应用管理", "Applications")}><header className="sunshine-section-header"><div className="xcss-actions"><Button disabled={busy} onClick={()=>void submit({kind:"list_applications"})}>{t("刷新应用", "Refresh applications")}</Button><Button disabled={blocked||!appSnapshot} onClick={()=>edit(null)}>{t("新建应用", "New application")}</Button><Button disabled={blocked} onClick={()=>confirm({title:t("关闭当前应用", "Close current application"),description:t("这只会调用 Sunshine 关闭当前应用，可能中断活动串流。", "This asks Sunshine to close its current application and may interrupt an active stream."),run:()=>submit({kind:"close_application",administrator_confirmed:true})})}>{t("关闭当前应用", "Close current application")}</Button></div></header>
 {applicationReadFailed&&<p role="alert">{t("未能刷新应用列表。当前显示的应用来自较早的回执，请检查日志后重试。", "Could not refresh applications. Any applications shown are from an earlier result. Check Logs and try again.")}</p>}
 {pendingAttempt(applicationReadAttempt)&&<p role="status">{t("正在等待应用列表回执；显示的已有列表尚未刷新。", "Waiting for applications; any existing list has not been refreshed yet.")}</p>}
 {!appSnapshot?<p>{t("刷新应用列表后，可新建或编辑应用。", "Refresh the application list to create or edit applications.")}</p>:!appSnapshot.applications.length?<p className="sunshine-muted">{t("暂无应用，可以新建应用。", "No applications yet. Create an application to get started.")}</p>:<div className="sunshine-management-list">{appSnapshot.applications.map(value=><article className="sunshine-management-row" key={value.reference.fingerprint}><div className="sunshine-row-summary"><h3>{value.specification.name}</h3><p>{value.specification.cmd||t("无启动命令", "No launch command")}</p></div><div className="xcss-actions"><Button disabled={blocked} onClick={()=>edit(value.reference)}>{t("编辑", "Edit")}</Button><Button className="sunshine-danger-action" disabled={blocked} onClick={()=>confirm({title:t("删除应用", "Delete application"),description:t("删除会按当前列表修订和应用内容引用重新核对目标。", "Deletion rechecks the target using the current list revision and content reference."),run:()=>submit({kind:"delete_application",expected_revision:appSnapshot.revision,target:value.reference,administrator_confirmed:true})})}>{t("删除", "Delete")}</Button></div></article>)}</div>}
 {editing!==undefined&&appSnapshot&&<ApplicationEditor application={application} setApplication={setApplication} detachedText={detachedText} setDetachedText={setDetachedText} editing={editing!==null} blocked={blocked||applicationPending} onSubmit={event=>void saveApplication(event)} onCancel={cancelApplication}/>}
 <form className="sunshine-cover-form" onSubmit={event=>{event.preventDefault();const data=new FormData(event.currentTarget);const file=data.get("cover");const key=String(data.get("cover_key")??"").trim();const input=event.currentTarget.elements.namedItem("cover");if(!(file instanceof File)||!file.size)return;if(file.size>30*1024||file.type!=="image/png"){if(input instanceof HTMLInputElement){input.setCustomValidity(t("封面必须是 30 KiB 以内的 PNG。", "The cover must be a PNG no larger than 30 KiB."));input.reportValidity()}return}if(input instanceof HTMLInputElement)input.setCustomValidity("");void fileBase64(file).then(png_base64=>submit({kind:"upload_cover",key,png_base64,administrator_confirmed:true}))}}><h3>{t("上传封面", "Upload cover")}</h3><p className="sunshine-muted">{t("PNG 格式，最大 30 KiB。", "PNG format, up to 30 KiB.")}</p><div className="sunshine-upload-fields"><FormField label={t("封面键", "Cover key")}><TextField name="cover_key" required pattern="[A-Za-z0-9._-]{1,64}"/></FormField><FormField label={t("PNG 封面", "PNG cover")}><input name="cover" className="sunshine-file-input" type="file" accept="image/png" required onChange={event=>event.currentTarget.setCustomValidity("")}/></FormField><Button type="submit" disabled={blocked}>{t("上传 PNG", "Upload PNG")}</Button></div>{uploadedCover&&<p>{t("已保存路径：", "Saved path: ")}<code>{uploadedCover}</code></p>}</form>
 </section>}
 {section==="diagnostics"&&caps.diagnostics&&<section id="sunshine-diagnostics" className="xcss-content-panel" aria-label={t("诊断与 Sunshine 日志", "Diagnostics and Sunshine logs")}><p>{t("这里读取的是本机 Sunshine 日志；管理操作记录位于“日志”页面。日志按字节游标分页并在客户端脱敏。", "This reads local Sunshine logs. Manager operation records are on the Logs page. Logs use byte cursors and are redacted by the client.")}</p><div className="xcss-actions"><Button disabled={busy} onClick={()=>void submit({kind:"read_logs",cursor:null,limit_bytes:16384})}>{t("读取最新日志", "Read latest logs")}</Button><Button disabled={busy||!logPage?.previous} onClick={()=>{if(logPage?.previous)void submit({kind:"read_logs",cursor:logPage.previous,limit_bytes:16384})}}>{t("读取上一页", "Read previous page")}</Button><Button disabled={busy} onClick={()=>void submit({kind:"read_diagnostics"})}>{t("读取诊断", "Read diagnostics")}</Button><Button disabled={busy} onClick={()=>void submit({kind:"read_virtual_input_status"})}>{t("查询虚拟输入", "Read virtual input status")}</Button></div>{logPage&&<pre className="sunshine-logs">{logPage.text||t("此页没有日志内容", "This page has no log content")}</pre>}{failedAttempt(diagnosticAttempt)&&<p role="alert">{t("未能刷新诊断。当前显示的值来自较早的回执，请检查日志后重试。", "Could not refresh diagnostics. Any values shown are from an earlier result. Check Logs and try again.")}</p>}{pendingAttempt(diagnosticAttempt)&&<p role="status">{t("正在等待诊断回执；显示的已有结果尚未刷新。", "Waiting for diagnostics; any existing result has not been refreshed yet.")}</p>}{diagnostic&&<dl><dt>{t("API 可达", "API reachable")}</dt><dd>{String(diagnostic.api_reachable)}</dd><dt>{t("认证通过", "Authentication accepted")}</dt><dd>{String(diagnostic.authentication_accepted)}</dd><dt>{t("Sunshine 版本", "Sunshine version")}</dt><dd>{diagnostic.sunshine_version??t("未知", "Unknown")}</dd><dt>{t("平台", "Platform")}</dt><dd>{diagnostic.platform??t("未知", "Unknown")}</dd><dt>{t("服务状态", "Service state")}</dt><dd>{diagnostic.service_state}</dd></dl>}{virtualStatus&&<p>{t("VirtualHID", "VirtualHID")}: {virtualStatus.virtualhid.installed?t("已安装", "installed"):t("未安装", "not installed")} · ViGEmBus: {virtualStatus.vigembus.installed?t("已安装", "installed"):t("未安装", "not installed")}</p>}</section>}
 {section==="maintenance"&&caps.maintenance&&<section id="sunshine-maintenance" className="xcss-content-panel" aria-label={t("显示与输入维护", "Display and input maintenance")}><div className="xcss-actions"><Button disabled={blocked} onClick={()=>confirm({title:t("重置显示设备持久状态", "Reset display device persistence"),description:t("Sunshine 将清除已保存的显示设备选择；下次串流可能重新选择显示器。", "Sunshine will clear its saved display selection and may choose a display again on the next stream."),run:()=>submit({kind:"run_maintenance",action:"reset_display_persistence",administrator_confirmed:true})})}>{t("重置显示设备", "Reset display device")}</Button><Button disabled={blocked} onClick={()=>confirm({title:t("重置 Portal token", "Reset Portal token"),description:t("现有 Portal token 将失效。", "The current Portal token will be invalidated."),run:()=>submit({kind:"run_maintenance",action:"reset_portal_token",administrator_confirmed:true})})}>{t("重置 Portal token", "Reset Portal token")}</Button></div></section>}
 {section==="service"&&caps.service_control&&<section id="sunshine-service" className="xcss-content-panel" aria-label={t("Sunshine 服务", "Sunshine service")}><h2>{t("Sunshine 服务", "Sunshine service")}</h2><p>{t("客户端只操作本机配置中固定的 Sunshine 服务；不能指定服务名、路径或 Shell 命令。", "The client only controls the fixed Sunshine service selected in its local configuration. Service names, paths and shell commands cannot be supplied remotely.")}</p>{failedAttempt(serviceAttempt)&&<p role="alert">{t("最新服务操作未获确认。显示的是最近一次上报的状态，请检查日志后刷新。", "The latest service operation was not confirmed. Any state shown is the last reported state. Check Logs and refresh.")}</p>}{pendingAttempt(serviceAttempt)&&<p role="status">{t("正在等待服务状态回执…", "Waiting for a service state receipt…")}</p>}<p>{failedAttempt(serviceAttempt)||pendingAttempt(serviceAttempt)?t("最近上报状态：", "Last reported state: "):t("当前状态：", "Current state: ")}{serviceState??t("尚未读取", "not read")}</p><div className="xcss-actions"><Button disabled={busy} onClick={()=>void submit({kind:"read_service_status"})}>{t("刷新服务状态", "Refresh service status")}</Button>{(["start","stop","restart"] as const).map(action=><Button key={action} disabled={blocked} onClick={()=>confirm({title:t(`确认${action==="start"?"启动":action==="stop"?"停止":"重启"} Sunshine 服务`, `Confirm Sunshine service ${action}`),description:t("客户端将调用固定的本机服务适配器并核对状态转换。", "The client will invoke its fixed local service adapter and verify the state transition."),run:()=>submit({kind:"control_service",action,administrator_confirmed:true})})}>{action==="start"?t("启动", "Start"):action==="stop"?t("停止", "Stop"):t("重启", "Restart")}</Button>)}</div></section>}
 </>;
}
