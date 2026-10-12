import { operationLabel, configValueLabel } from "./display-labels";
import { t } from "@xcss/web/admin-ui/i18n";
import {useEffect,useRef,useState} from "react";
import {Button,ConfirmDangerDialog,EmptyState,ErrorState,FormField,LoadingState,Table,TextField} from "@xcss/web/admin-ui";
import {useAdminApplication,errorRequestId} from "@xcss/web/admin-shell";
import {CURRENT_API_PREFIX,isClientAuthorization,isDevice,isOperation,isOperations,isOperationPage,isOperationSummary,isTaskCalendar,isSnapshot,supportsConfigurationOverwrite,type ClientAuthorization,type Command,type ConfigFieldDefinition,type DeviceInfo,type Operation,type Snapshot,type Ticket,currentErrorEnvelope} from "./api";
import {configCategories,fieldsForDevice,prerequisiteLabel,type ConfigCategory} from "./config-fields";
import {SunshineControls,isSunshineManagementCategory,sunshineManagementCategories,type SunshineManagementCategory} from "./SunshineControls";
import {ConfigSelect} from "./ConfigSelect";
import {configurationFieldError} from "./input-validation";
import {configurationFeedback,isConfigurationCommand,type ConfigurationActivity} from "./configuration-feedback";
import { DateRangeField, type CalendarDateRange } from "@xcss/web/admin-ui/date-range";
import "@xcss/web/admin-ui/date-range.css";
const states:Record<string,string>={unknown:t("尚未核对", "Not yet checked"),awaiting_restart:t("配置已保存，等待管理员重启", "Configuration saved; waiting for an administrator to restart"),pending_verification:t("配置已读取，运行时生效待验证", "Configuration read; runtime effect needs verification"),drift_detected:t("实际配置与已保存修订不同，请核对冲突", "Actual configuration differs from the saved revision; review the conflict")};
const tasks:Record<string,string>={pending:t("排队中", "Queued"),running:t("已下发，等待执行回执", "Dispatched; waiting for execution receipt"),succeeded:t("执行已完成", "Execution completed"),failed:t("执行失败", "Execution failed"),unknown:t("结果不确定", "Outcome unknown"),resolved:t("已人工核对", "Manually reconciled"),dead_letter:t("执行已拒绝", "Execution rejected")};
const instanceNameError=t("实例名称须为 1–32 个字符，不能包含控制字符或首尾空白。", "Use 1–32 characters without control characters or surrounding whitespace.");
function validInstanceName(value:string):boolean{
 const length=[...value].length;
 return length>0&&length<=32&&!/^\p{White_Space}|\p{White_Space}$/u.test(value)&&!/[\u0000-\u001f\u007f-\u009f\ud800-\udfff]/u.test(value);
}
function logPageRequest(signal:AbortSignal):RequestInit&{maxResponseBytes:number;timeoutMs:number}{
 // The administrator client passes these bounded JSON transport options through to requestJson.
 return {signal,maxResponseBytes:8*1024*1024,timeoutMs:5_000};
}
type WorkspaceFailure={message:string;requestId?:string};
type SubmittedConfiguration={operation:Operation;draft:Record<string,string>;remove:string[]};
function newerOperation(current:Operation|undefined,incoming:Operation):Operation{
 if(!current)return incoming;
 return incoming.updated_at_micros>current.updated_at_micros||(incoming.updated_at_micros===current.updated_at_micros&&(incoming.reconciliation_observed_at_micros??-1)>=(current.reconciliation_observed_at_micros??-1))?incoming:current;
}
function withSaveReceipts(operations:Operation[],receipts:Iterable<Operation>):Operation[]{
 const values=new Map(operations.map(operation=>[operation.operation_id,operation]));
 for(const receipt of receipts)values.set(receipt.operation_id,newerOperation(values.get(receipt.operation_id),receipt));
 return [...values.values()];
}
export function ClientWorkspace({device,fieldDefinitions,refreshSignal,configRefreshSignal,changed,removed,ticket:initialTicket,page}:{device:DeviceInfo;fieldDefinitions:ConfigFieldDefinition[]|null;refreshSignal:number;configRefreshSignal:number;changed():void;removed():void;ticket:Ticket|null;page:"details"|"logs"}){
 const{client,notify}=useAdminApplication();const[operations,setOperations]=useState<Operation[]>([]);
 const[blockingCount,setBlockingCount]=useState(0);const[summaryDevice,setSummaryDevice]=useState<string|null>(null);const[logOperations,setLogOperations]=useState<Operation[]>([]);const[logRange,setLogRange]=useState<CalendarDateRange|null>(null);const[loadedLogRange,setLoadedLogRange]=useState<CalendarDateRange|null>(null);const[logDraftValid,setLogDraftValid]=useState(false);const[calendarLoading,setCalendarLoading]=useState(false);const[calendarRefresh,setCalendarRefresh]=useState(0);const[logLoading,setLogLoading]=useState(false);const[logRefresh,setLogRefresh]=useState(0);const[logCursor,setLogCursor]=useState<string|null>(null);const[logNext,setLogNext]=useState<string|null>(null);const[logPrevious,setLogPrevious]=useState<string|null>(null);
 const[authorization,setAuthorization]=useState<ClientAuthorization|null>(initialTicket?{manager_id:initialTicket.manager_id,device_id:initialTicket.device.id,authorization_code:initialTicket.token}:null);
 const[busy,setBusy]=useState(false);const[authorizationFailure,setAuthorizationFailure]=useState<WorkspaceFailure|null>(null);const[taskFailure,setTaskFailure]=useState<WorkspaceFailure|null>(null);const[actionFailure,setActionFailure]=useState<WorkspaceFailure|null>(null);const[confirm,setConfirm]=useState<{title:string;description:string;run():Promise<unknown>}|null>(null);
 const[name,setName]=useState(device.name);const persistedName=useRef(device.name);
 const nameInvalid=!validInstanceName(name);
 const[baseline,setBaseline]=useState<Snapshot|null>(null);const[draft,setDraft]=useState<Record<string,string>>({});const[remove,setRemove]=useState<string[]>([]);
 const[submittedConfiguration,setSubmittedConfiguration]=useState<SubmittedConfiguration|null>(null);
 const[configLoading,setConfigLoading]=useState(false);const[configFailure,setConfigFailure]=useState<WorkspaceFailure|null>(null);
 const[configActivity,setConfigActivity]=useState<ConfigurationActivity|null>(null);
 const configurationGeneration=useRef(0);
 const configurationRetry=useRef<{body:string;key:string}|null>(null);
 const saveReceipts=useRef(new Map<"save_config"|"save_application",Operation>());
 const configReadKey=useRef<{refresh:number;key:string}|null>(null);
 const idempotencyKeys=useRef(new Map<string,string>());const operationRevision=useRef(0);const calendarReady=useRef(false);
 const[configCategory,setConfigCategory]=useState<"overview"|"preview"|"moonlight"|ConfigCategory|SunshineManagementCategory>("overview");
 const configForm=useRef<HTMLFormElement>(null);
 const configFields=fieldsForDevice(fieldDefinitions??[],device);
 const visibleCategories=configCategories.filter(category=>!("encoder" in category)||configFields.some(field=>field.category===category.id));
 const activeCategory=configCategory==="overview"||configCategory==="preview"||configCategory==="moonlight"||isSunshineManagementCategory(configCategory)||visibleCategories.some(category=>category.id===configCategory)?configCategory:"overview";
 const controlSection=activeCategory==="general"?"service":activeCategory==="moonlight"||isSunshineManagementCategory(activeCategory)?activeCategory:null;
 const base=`${CURRENT_API_PREFIX}/sunshine/devices/${device.id}`;
 useEffect(()=>{
  // Only page entry and an explicit header/browser refresh replace the editor snapshot.
  // Status polling and completed writes use a separate refresh signal.
  configurationGeneration.current+=1;configurationRetry.current=null;saveReceipts.current.delete("save_config");
  if(page!=="details"){configReadKey.current=null;return}
  const controller=new AbortController();let active=true;let timer:number|undefined;
  setBaseline(null);setDraft({});setRemove([]);setSubmittedConfiguration(null);setConfigFailure(null);setConfigActivity(null);
  if(device.revoked||!device.registered||device.capabilities?.protocol!=="xscs-management/1"){configReadKey.current=null;setConfigLoading(false);return}
  setConfigLoading(true);
  const key=configReadKey.current?.refresh===configRefreshSignal?configReadKey.current.key:crypto.randomUUID();
  configReadKey.current={refresh:configRefreshSignal,key};
  const fail=(error:unknown)=>{if(active){setConfigLoading(false);setConfigFailure({message:t("配置暂时不可用，请刷新页面后重试。", "Configuration is temporarily unavailable. Refresh the page to try again."),requestId:errorRequestId(error)})}};
  async function accept(operation:Operation){
   if(!active)return;
   setConfigActivity({command:"read_config",operation});
   if(operation.device_id!==device.id||operation.action!=="sunshine.config.read"){fail(undefined);return}
   if(["pending","running"].includes(operation.state)){
    timer=window.setTimeout(()=>{void client.request(`${CURRENT_API_PREFIX}/sunshine/operations/${operation.operation_id}`,isOperation,{signal:controller.signal}).then(accept).catch(fail)},1000);return;
   }
   operationRevision.current+=1;setOperations(values=>withSaveReceipts([newerOperation(values.find(value=>value.operation_id===operation.operation_id),operation),...values.filter(value=>value.operation_id!==operation.operation_id)].slice(0,50),saveReceipts.current.values()));
   if(operation.state==="succeeded"&&operation.result?.kind==="config_read"&&isSnapshot(operation.result.snapshot)){
    const snapshot=operation.result.snapshot;setBaseline(snapshot);setDraft({...snapshot.fields});setRemove([]);setConfigLoading(false);
   }else fail(undefined);
  }
  void client.request(base+"/tasks",isOperation,{method:"POST",headers:{"Idempotency-Key":key},body:JSON.stringify({kind:"read_config"}),signal:controller.signal}).then(accept).catch(fail);
  return()=>{active=false;controller.abort();if(timer!==undefined)clearTimeout(timer)};
 },[base,client,page,configRefreshSignal,device.registered,device.revoked,device.capabilities?.protocol]);
 useEffect(()=>{
  const previous=persistedName.current;persistedName.current=device.name;
  // Refresh clean fields without replacing an administrator's unsaved draft.
  setName(current=>current===previous?device.name:current);
 },[device.name]);
 useEffect(()=>{const controller=new AbortController();let active=true;void client.request(base+"/authorization",isClientAuthorization,{signal:controller.signal}).then(value=>{if(active){setAuthorization(value);setAuthorizationFailure(null)}}).catch(error=>{if(active)setAuthorizationFailure({message:t("无法读取实例密码", "Unable to load the instance password"),requestId:errorRequestId(error)})});return()=>{active=false;controller.abort()}},[base,client,refreshSignal]);
 useEffect(()=>{if(page!=="details")return;const controller=new AbortController();let active=true;let timer:number|undefined;setSummaryDevice(null);
  async function load(){
   const revision=operationRevision.current;const generation=configurationGeneration.current;const tracked=[...saveReceipts.current.entries()];let readingReceipts=false;
   const current=()=>active&&revision===operationRevision.current&&generation===configurationGeneration.current;
   try{
    const[values,summary]=await Promise.all([client.request(base+"/tasks?recent=50",isOperations,{signal:controller.signal}),client.request(base+"/tasks/summary",isOperationSummary,{signal:controller.signal})]);
    if(!current())return;
    // The history page is creation-ordered. A submitted save can leave it before its receipt arrives.
    // Reuse this poll for at most the current configuration save and application save.
    const receipts=await Promise.all(tracked.map(async([kind,previous])=>{
     let receipt=values.find(value=>value.operation_id===previous.operation_id);
     if(!receipt&&["pending","running","unknown"].includes(previous.state)){
      readingReceipts=true;
      receipt=await client.request(`${CURRENT_API_PREFIX}/sunshine/operations/${previous.operation_id}`,isOperation,{signal:controller.signal});
     }
     if(receipt&&(receipt.operation_id!==previous.operation_id||receipt.device_id!==device.id||receipt.action!==previous.action))throw new Error("Save operation receipt does not match");
     return [kind,receipt?newerOperation(previous,receipt):previous] as const;
    }));
    if(!current())return;
    for(const[kind,receipt]of receipts)if(saveReceipts.current.get(kind)?.operation_id===receipt.operation_id)saveReceipts.current.set(kind,newerOperation(saveReceipts.current.get(kind),receipt));
    // Keep terminal receipts in their two bounded slots; a later page must not restore pending state.
    setOperations(withSaveReceipts(values,saveReceipts.current.values()));setBlockingCount(summary.blocking_count);setSummaryDevice(device.id);setTaskFailure(null);
   }catch(error){if(current()){setSummaryDevice(null);setTaskFailure({message:readingReceipts?t("保存任务回执暂时不可用，草稿已保留。请稍后重试或检查日志。", "The save receipt is temporarily unavailable. Your draft is preserved. Try again shortly or check Logs."):t("任务状态暂时不可用", "Task status is temporarily unavailable"),requestId:errorRequestId(error)})}}
   finally{if(active)timer=window.setTimeout(load,2000)}
  }
  void load();return()=>{active=false;controller.abort();if(timer!==undefined)clearTimeout(timer)}
 },[base,client,device.id,refreshSignal,page]);
 useEffect(()=>{if(page!=="logs")return;const controller=new AbortController();let active=true;calendarReady.current=false;setLogCursor(null);setLogNext(null);setLogPrevious(null);setLogOperations([]);setLogRange(null);setLoadedLogRange(null);setLogLoading(false);setCalendarLoading(true);setTaskFailure(null);void client.request(base+"/tasks/calendar",isTaskCalendar,{signal:controller.signal}).then(value=>{if(active){calendarReady.current=true;setLogRange({start:value.today,end:value.today})}}).catch(error=>{if(active)setTaskFailure({message:t("无法读取服务器当天日期", "Could not read the server's current date"),requestId:errorRequestId(error)})}).finally(()=>{if(active)setCalendarLoading(false)});return()=>{active=false;calendarReady.current=false;controller.abort()}},[base,client,page,calendarRefresh]);
 useEffect(()=>{if(page!=="logs"||!calendarReady.current||logRange===null)return;setTaskFailure(null);const controller=new AbortController();let active=true;setLogLoading(true);const query=logRange.start===logRange.end?new URLSearchParams({date:logRange.start}):new URLSearchParams({start_date:logRange.start,end_date:logRange.end});if(logCursor!==null)query.set("cursor",logCursor);setLogOperations([]);setLoadedLogRange(null);setLogNext(null);setLogPrevious(null);void client.request(base+"/tasks?"+query,isOperationPage,logPageRequest(controller.signal)).then(values=>{if(active){setLogOperations(values.operations);setLogNext(values.next_cursor);setLogPrevious(values.previous_cursor);setLoadedLogRange(logRange);setTaskFailure(null)}}).catch(error=>{if(active)setTaskFailure({message:t("日志暂时不可用", "Logs are temporarily unavailable"),requestId:errorRequestId(error)})}).finally(()=>{if(active)setLogLoading(false)});return()=>{active=false;controller.abort()}},[base,client,refreshSignal,page,logRange,logRefresh,logCursor]);
 async function perform(action:()=>Promise<void>,after:()=>void=changed,failureMessage?:string):Promise<boolean>{if(busy)return false;setBusy(true);setActionFailure(null);try{await action();after();return true}catch(error){setActionFailure({message:currentErrorEnvelope(error)?.code==="history_capacity_exhausted"?t("任务历史容量已满，新任务未接受；已有任务仍可查询。", "Task history capacity is full. The new task was not accepted; existing tasks remain available."):currentErrorEnvelope(error)?.code==="conflict"?t("配置或设备状态发生冲突，请刷新后核对。", "Configuration or device state conflict. Refresh and review the latest state."):failureMessage??t("操作未能确认，请检查日志，勿直接重复重启。", "The operation could not be confirmed. Check the logs; do not blindly repeat a restart."),requestId:errorRequestId(error)});return false}finally{setBusy(false)}}
 function randomAuthorizationCode(){const alphabet="abcdefghijklmnopqrstuvwxyz0123456789";let result="";while(result.length<36){for(const value of crypto.getRandomValues(new Uint8Array(64))){if(value<252)result+=alphabet[value%alphabet.length];if(result.length===36)break}}return result}
 const readActions=new Set(["sunshine.config.read","sunshine.applications.list","sunshine.pairing.clients.list","sunshine.pairing.pending.list","sunshine.logs.read","sunshine.diagnostics.read","sunshine.virtual_input.read","sunshine.service.read"]);
 function isReadCommand(command:Command){return ["read_config","list_applications","list_paired_clients","list_pending_pairings","read_logs","read_diagnostics","read_virtual_input_status","read_service_status"].includes(command.kind)}
 async function submit(command:Command,onAccepted?:(operation:Operation)=>void):Promise<boolean>{
  const readOnly=isReadCommand(command);
  if(!readOnly&&blocked)return false;
  const generation=configurationGeneration.current;
  const body=JSON.stringify(command);
  // A retry belongs to the unchanged editor intent, not every future save with the same values.
  // Other commands keep their existing retry identities, including uncertain restarts.
  const configurationSave=command.kind==="save_config";
  const key=(configurationSave?(configurationRetry.current?.body===body?configurationRetry.current.key:undefined):idempotencyKeys.current.get(body))??crypto.randomUUID();
  if(configurationSave)configurationRetry.current={body,key};else idempotencyKeys.current.set(body,key);
  return perform(async()=>{
   const operation=await client.request(base+"/tasks",isOperation,{method:"POST",headers:{"Idempotency-Key":key},body});
   onAccepted?.(operation);
   if(command.kind==="save_application"||(command.kind==="save_config"&&generation===configurationGeneration.current)){
    const previous=saveReceipts.current.get(command.kind);saveReceipts.current.set(command.kind,newerOperation(previous?.operation_id===operation.operation_id?previous:undefined,operation));
   }
   if(configurationSave){if(configurationRetry.current?.key===key)configurationRetry.current=null}else idempotencyKeys.current.delete(body);operationRevision.current+=1;setOperations(values=>withSaveReceipts([newerOperation(values.find(value=>value.operation_id===operation.operation_id),operation),...values.filter(value=>value.operation_id!==operation.operation_id)].slice(0,50),saveReceipts.current.values()));
   if(generation===configurationGeneration.current){
    if(isConfigurationCommand(command))setConfigActivity({command:command.kind,operation});
    if(command.kind==="save_config")setSubmittedConfiguration({operation,draft:{...draft},remove:[...remove]});
   }
   // Read tasks update their own results without announcing a mutation.
   if(!readOnly){
    if(["pending","running","unknown"].includes(operation.state))setBlockingCount(value=>Math.max(value,1));
    notify(t("操作已写入日志，等待客户端执行。", "The operation was written to the log and is waiting for the client."));
   }
  },readOnly?()=>{}:changed,readOnly?t("读取暂时不可用，请稍后重试。", "Reading is temporarily unavailable. Try again shortly."):undefined);
 }
 const polledConfigOperation=configActivity&&operations.find(operation=>operation.operation_id===configActivity.operation.operation_id&&operation.updated_at_micros>=configActivity.operation.updated_at_micros);
 const configFeedback=configActivity?configurationFeedback({...configActivity,operation:polledConfigOperation||configActivity.operation}):null;
 const submittedOperation=submittedConfiguration&&(operations.find(value=>value.operation_id===submittedConfiguration.operation.operation_id&&value.updated_at_micros>=submittedConfiguration.operation.updated_at_micros)??submittedConfiguration.operation);
 const awaitingConfigurationSave=!!submittedOperation&&["pending","running","unknown"].includes(submittedOperation.state);
 useEffect(()=>{
  if(!submittedConfiguration||!submittedOperation)return;
  const operation=submittedOperation;
  const report=operation.state==="succeeded"?operation.result:operation.state==="resolved"&&operation.resolution==="confirmed_succeeded"?operation.reconciliation:undefined;
  if(operation.device_id!==device.id||operation.action!=="sunshine.config.save"||report?.kind!=="config_saved"||!isSnapshot(report.snapshot))return;
  const snapshot=report.snapshot;
  const nextDraft={...draft};const nextRemove=new Set(remove);
  // A successful receipt confirms only the submitted draft. Preserve edits made while it ran.
  for(const key of new Set([...Object.keys(draft),...Object.keys(snapshot.fields),...Object.keys(submittedConfiguration.draft),...remove])){
   if((draft[key]??"")!==(submittedConfiguration.draft[key]??"")||remove.includes(key)!==submittedConfiguration.remove.includes(key))continue;
   if(key in snapshot.fields)nextDraft[key]=snapshot.fields[key];else delete nextDraft[key];
   nextRemove.delete(key);
  }
  setBaseline(snapshot);setDraft(nextDraft);setRemove([...nextRemove]);setSubmittedConfiguration(null);
 },[submittedConfiguration,submittedOperation,device.id,draft,remove]);
 const blocked=busy||device.revoked||!device.registered||summaryDevice!==device.id||blockingCount>0||operations.some(operation=>!readActions.has(operation.action)&&["pending","running","unknown"].includes(operation.state));
 const logDateValid=logRange!==null;
 const visibleLogOperations=logRange!==null&&loadedLogRange===logRange?logOperations:[];
 function selectConfigCategory(category:ConfigCategory){setConfigCategory(category)}
 function saveCommand():Extract<Command,{kind:"save_config"}>|null{
  if(!baseline||fieldDefinitions===null||configLoading||!supportsConfigurationOverwrite(device.capabilities))return null;
  for(const field of configFields){
   const issue=configurationFieldError(field,remove.includes(field.key)?"":draft[field.key]??"");
   if(issue){selectConfigCategory(field.category);requestAnimationFrame(()=>document.getElementById(`sunshine-field-${field.key}`)?.focus());return null;}
  }
  const invalid=configForm.current?.querySelector<HTMLInputElement|HTMLSelectElement>("input:invalid, select:invalid");
  if(invalid){const category=invalid.closest<HTMLElement>("[data-config-category]")?.dataset.configCategory as ConfigCategory|undefined;if(category)selectConfigCategory(category);requestAnimationFrame(()=>invalid.reportValidity());return null}
  const set:Record<string,string|number|boolean>={};const deleted:string[]=[];
  for(const field of configFields){const value=draft[field.key]??"";
   if(remove.includes(field.key)||value==="")deleted.push(field.key);
   else set[field.key]=field.kind==="integer"?Number(value):field.boolean?value==="true":value;
  }
  return{kind:"save_config",set,remove:deleted,restart_policy:"manual"};
 }
 const previewFields=baseline?configFields.filter(field=>{
  const value=remove.includes(field.key)?"":draft[field.key]??"";
  if(value==="")return field.key in baseline.fields;
  return field.kind==="integer"?String(Number(value))!==baseline.fields[field.key]:value!==baseline.fields[field.key];
 }):[];
 function changeConfiguration(key:string,value:string){if((draft[key]??"")===value)return;configurationRetry.current=null;setDraft(values=>({...values,[key]:value}))}
 function resetConfiguration(key:string,checked:boolean){if(remove.includes(key)===checked)return;configurationRetry.current=null;setRemove(values=>checked?[...values,key]:values.filter(value=>value!==key))}
 function undoConfiguration(key:string){
  if(!baseline||busy||awaitingConfigurationSave)return;
  configurationRetry.current=null;
  setDraft(values=>{const restored={...values};if(key in baseline.fields)restored[key]=baseline.fields[key];else delete restored[key];return restored});
  setRemove(values=>values.filter(value=>value!==key));
 }
 function applyConfiguration(){if(blocked||!previewFields.length)return;const command=saveCommand();if(command)void submit(command)}
 const overviewContent=<section className="xcss-content-stack sunshine-overview" aria-label={t("实例概览", "Instance overview")}><section className="sunshine-overview-section" aria-label={t("配对账户信息", "Pairing account information")}><dl className="sunshine-detail-list"><dt>{t("账户名", "Account name")}</dt><dd>{device.name}</dd><dt>{t("账户", "Account")}</dt><dd><code>{authorization?.device_id??device.id}</code></dd><dt>{t("密码", "Password")}</dt><dd>{authorization?<code className="sunshine-token">{authorization.authorization_code}</code>:t("正在读取…", "Loading…")}</dd><dt>{t("配对状态", "Pairing status")}</dt><dd>{device.revoked?t("凭据已撤销", "Credentials revoked"):!device.registered?(device.pairing_pending?t("等待配对", "Waiting for pairing"):t("配对已取消", "Pairing cancelled")):t("已配对", "Paired")}</dd></dl></section>

 <section className="sunshine-overview-section" aria-label={t("Sunshine 状态", "Sunshine status")}><dl className="sunshine-detail-list"><dt>{t("客户端", "Client")}</dt><dd>{device.client_online?t("在线", "Online"):t("离线", "Offline")}</dd><dt>{t("Sunshine 接口", "Sunshine API")}</dt><dd>{device.sunshine_reachable===null?t("未知", "Unknown"):device.sunshine_reachable?t("可访问", "Reachable"):t("不可访问", "Unreachable")}</dd><dt>{t("配置状态", "Configuration status")}</dt><dd>{states[device.configuration_state]??t("未知", "Unknown")}</dd></dl><p>{t("客户端 在线不等于 Sunshine 正常；配置保存或重启响应不证明运行时已生效。", "An online client does not mean Sunshine is healthy. Saving or restarting does not prove runtime effect.")}</p></section>

<section className="sunshine-overview-section sunshine-instance-settings" aria-label={t("实例设置", "Instance settings")}><form onSubmit={event=>{event.preventDefault();if(nameInvalid)return;void perform(async()=>{const updated=await client.request(base,isDevice,{method:"PATCH",body:JSON.stringify({name})});setName(updated.name);persistedName.current=updated.name;notify(t("实例名称已保存", "Instance name saved"))})}} aria-busy={busy}><FormField label={t("实例名称", "Instance name")}><TextField name="name" value={name} onChange={event=>setName(event.target.value)} onInput={event=>event.currentTarget.setCustomValidity(validInstanceName(event.currentTarget.value)?"":instanceNameError)} aria-invalid={nameInvalid||undefined} aria-describedby={nameInvalid?"sunshine-instance-name-error":undefined} required readOnly={busy}/></FormField>{nameInvalid&&<p id="sunshine-instance-name-error" role="alert">{instanceNameError}</p>}<div className="xcss-actions"><Button type="submit" disabled={busy||nameInvalid||name===device.name}>{busy?t("正在保存…", "Saving…"):t("保存名称", "Save name")}</Button></div></form></section>
 <section className="sunshine-overview-section sunshine-instance-actions" aria-label={t("实例操作", "Instance actions")}><div className="xcss-actions">{!device.revoked&&<Button disabled={busy} onClick={()=>setConfirm({title:t("更换密码", "Change password"),description:t("这会立即撤销当前客户端凭据。客户端必须使用新的密码重新配对后才能连接。", "This immediately revokes the current client credential. The client must pair again with the new password before it can reconnect."),run:()=>perform(async()=>{const value=await client.request(base+"/authorization",isClientAuthorization,{method:"PUT",body:JSON.stringify({authorization_code:randomAuthorizationCode()})});setAuthorization(value);notify(t("密码已更换，请在客户端使用新密码重新配对。", "Password changed. Pair the client again with the new password."))})})}>{t("更换密码", "Change password")}</Button>}
 {device.pairing_pending&&!device.registered&&!device.revoked&&<Button disabled={busy} onClick={()=>setConfirm({title:t("取消配对", "Cancel pairing"),description:t("当前密码将无法用于配对；取消后可永久删除该实例，也可通过“更换密码”恢复。", "The current password can no longer be used for pairing. Afterwards the instance can be deleted permanently or reactivated with Change password."),run:()=>perform(async()=>{await client.request(base+"/pairing",(value):value is undefined=>value===undefined,{method:"DELETE"})})})}>{t("取消配对", "Cancel pairing")}</Button>}
 {device.registered&&!device.revoked&&<Button disabled={busy} onClick={()=>setConfirm({title:t("撤销设备凭据", "Revoke device credentials"),description:t("将断开 客户端 并永久禁止该设备凭据。执行中的任务可能变为不确定；如需重新注册，请新建设备。", "This disconnects the client and permanently revokes its credentials. Running tasks may have an unknown outcome. Create a new device to register again."),run:()=>perform(async()=>{await client.request(base+"/revoke",(value):value is undefined=>value===undefined,{method:"POST"})})})}>{t("撤销设备凭据", "Revoke device credentials")}</Button>}
 <Button disabled={busy} onClick={()=>setConfirm({title:t("删除实例", "Delete instance"),description:t("永久删除该实例。当前密码和客户端凭据会立即失效；操作日志仍保留用于审计。", "Permanently delete this instance. Its password and client credential are invalidated immediately; operation logs remain for audit."),run:()=>perform(async()=>{await client.request(base,(value):value is undefined=>value===undefined,{method:"DELETE"})},removed)})}>{t("删除实例", "Delete instance")}</Button></div><p>{t("这是该实例的长期密码，服务端会加密保存并允许之后查看或更换；更换后客户端必须重新配对。", "This is the instance's long-lived password. The server stores it encrypted and keeps it available to view or change; changing it requires the client to pair again.")}</p></section>
 </section>;
 return <div className="xcss-content-stack sunshine-workspace" aria-busy={busy}>
 {page==="details"&&<nav className="sunshine-config-navigation xcss-secondary-navigation" aria-label={t("Sunshine 配置分类", "Sunshine configuration categories")}><Button type="button" aria-pressed={activeCategory==="overview"} aria-controls="sunshine-instance-overview" onClick={()=>setConfigCategory("overview")}>{t("实例概览", "Instance overview")}</Button>{visibleCategories.map(category=><Button key={category.id} type="button" aria-pressed={activeCategory===category.id} aria-controls={baseline&&fieldDefinitions!==null?`sunshine-config-${category.id}`:undefined} onClick={()=>selectConfigCategory(category.id)}>{category.label}</Button>)}{sunshineManagementCategories.map(category=><Button key={category.id} type="button" aria-pressed={activeCategory===category.id} aria-controls={activeCategory===category.id?`sunshine-${category.id}`:undefined} onClick={()=>setConfigCategory(category.id)}>{category.label}</Button>)}<Button type="button" aria-pressed={activeCategory==="moonlight"} aria-controls={activeCategory==="moonlight"?"sunshine-moonlight-pairing":undefined} onClick={()=>setConfigCategory("moonlight")}>{t("Moonlight 配对", "Moonlight pairing")}</Button><Button type="button" aria-pressed={activeCategory==="preview"} aria-controls="sunshine-config-preview" onClick={()=>setConfigCategory("preview")}>{t("预览变更", "Preview changes")}</Button></nav>}
 {authorizationFailure&&<ErrorState requestId={authorizationFailure.requestId}>{authorizationFailure.message}</ErrorState>}
 {page!=="logs"&&taskFailure&&<ErrorState requestId={taskFailure.requestId}>{taskFailure.message}</ErrorState>}
 {configFailure&&<ErrorState requestId={configFailure.requestId}>{configFailure.message}</ErrorState>}
 {actionFailure&&<ErrorState requestId={actionFailure.requestId}>{actionFailure.message}</ErrorState>}

 {page==="details"&&summaryDevice===device.id&&blockingCount>0&&<section className="xcss-content-panel" aria-label={t("任务状态", "Task status")}><p role="status">{t("该实例有 {0} 个未完成任务。请在日志页选择相关日期核对；若无记录，请联系原操作者。", "This instance has {0} unfinished tasks. Check the relevant date in Logs; if no task appears, contact its original administrator.", [String(blockingCount)])}</p></section>}
 {page==="details"&&summaryDevice!==device.id&&!taskFailure&&<section className="xcss-content-panel" aria-label={t("任务状态", "Task status")}><p role="status">{t("正在确认该实例的任务状态，确认前暂停新操作。", "Checking this instance's tasks. New operations are paused until the check completes.")}</p></section>}
 {page==="details"&&<section className="xcss-content-panel sunshine-config-panel" hidden={activeCategory==="moonlight"||isSunshineManagementCategory(activeCategory)} aria-label={t("配置设置", "Configuration settings")}>
 <div id="sunshine-instance-overview" hidden={activeCategory!=="overview"}>{overviewContent}</div>
 <div className="sunshine-config-content" hidden={activeCategory==="overview"||activeCategory==="preview"||activeCategory==="moonlight"}>
 {baseline&&fieldDefinitions===null?<p>{t("正在加载配置字段定义…", "Loading configuration field definitions…")}</p>:baseline?<form ref={configForm} onSubmit={event=>event.preventDefault()} noValidate><p>{t("未列出的配置保留不变；命令、文件路径和敏感网络设置不开放远程修改。", "Unlisted settings are preserved. Commands, file paths and sensitive network settings cannot be changed remotely.")}</p>
 {visibleCategories.map(category=><section key={category.id} id={`sunshine-config-${category.id}`} data-config-category={category.id} hidden={activeCategory!==category.id} aria-label={category.label}>
 {!configFields.some(field=>field.category===category.id)&&<p>{t("当前设备没有可远程修改的此类配置。", "This device has no remotely editable settings in this category.")}</p>}
 {configFields.filter(field=>field.category===category.id).map(field=>{
 const inputId=`sunshine-field-${field.key}`;
 const fieldError=configurationFieldError(field,remove.includes(field.key)?"":draft[field.key]??"");
 const errorId=`${inputId}-error`;
 const helpId=field.prerequisite?`${inputId}-help`:undefined;
 const describedBy=[helpId,fieldError?errorId:undefined].filter(Boolean).join(" ")||undefined;
 return <div key={field.key} className="sunshine-config-field">
 <div className="sunshine-config-label"><label id={`${inputId}-label`} htmlFor={inputId}>{field.label}</label>{field.prerequisite&&<small id={helpId}>{prerequisiteLabel(field.prerequisite)}</small>}</div>
 {field.inputKind==="select"?<ConfigSelect id={inputId} describedBy={describedBy} value={draft[field.key]??""} disabled={busy||remove.includes(field.key)} options={[{value:"",label:t("未显式设置", "Not explicitly set")},...field.options.map(value=>({value,label:configValueLabel(value)}))]} onChange={value=>changeConfiguration(field.key,value)}/>:<TextField id={inputId} aria-describedby={describedBy} aria-invalid={!!fieldError||undefined} type={field.inputKind==="integer"?"number":"text"} min={field.minimum} max={field.maximum} step={1} value={draft[field.key]??""} disabled={busy||remove.includes(field.key)} onChange={event=>changeConfiguration(field.key,event.target.value)}/>}
 {fieldError&&<p id={errorId} role="alert">{fieldError}</p>}
 <label className="sunshine-config-reset" title={t("恢复默认（删除显式设置）", "Restore default (remove explicit setting)")}><input type="checkbox" aria-label={`${field.label}：${t("恢复默认（删除显式设置）", "Restore default (remove explicit setting)")}`} checked={remove.includes(field.key)} disabled={busy||!(field.key in baseline.fields)} onChange={event=>resetConfiguration(field.key,event.target.checked)}/>{t("恢复默认", "Restore default")}</label>
 </div>})}
 </section>)}

 </form>:<p role="status">{configLoading?t("正在加载配置…", "Loading configuration…"):t("客户端连接后可显示配置。", "Configuration is available after the client connects.")}</p>}
 {activeCategory==="general"&&<><Button disabled={blocked||!baseline||!device.capabilities?.restart_allowed} onClick={()=>setConfirm({title:t("确认重启 Sunshine", "Confirm Sunshine restart"),description:t("重启可能中断正在进行的串流。必须由管理员明确授权；客户端 管理进程保持独立。重启后仍可能需要人工验证运行时生效。", "Restarting may interrupt an active stream and requires explicit administrator authorization. The client remains independent. Runtime effect may still need manual verification after restarting."),run:async()=>{if(baseline)await submit({kind:"restart",expected_revision:baseline.revision,administrator_confirmed:true})}})}>{t("重启 Sunshine", "Restart Sunshine")}</Button>{!device.capabilities?.restart_allowed&&<p>{t("客户端 本机尚未授权受控重启。", "Controlled restarts have not been authorized locally on the client.")}</p>}</>}</div>
 <section id="sunshine-config-preview" className="sunshine-config-content" hidden={activeCategory!=="preview"} aria-label={t("预览变更", "Preview changes")}>
 {!baseline?<p>{configLoading?t("正在加载配置…", "Loading configuration…"):t("配置加载后可编辑并预览变更。", "Edit and preview changes after configuration loads.")}</p>:fieldDefinitions===null?<p>{t("正在加载配置字段定义…", "Loading configuration field definitions…")}</p>:previewFields.length?<section className="xcss-content-stack" aria-label={t("变更差异预览", "Change preview")}>
 <table className="xcss-table"><thead><tr><th>{t("字段", "Field")}</th><th>{t("原值", "Previous value")}</th><th>{t("新值", "New value")}</th><th>{t("撤销", "Undo")}</th></tr></thead><tbody>{previewFields.map(field=><tr key={field.key}><td>{field.label}</td><td>{baseline.fields[field.key]===undefined?t("默认", "Default"):configValueLabel(baseline.fields[field.key])}</td><td>{remove.includes(field.key)||!draft[field.key]?t("恢复默认", "Restore default"):configValueLabel(draft[field.key])}</td><td><Button type="button" disabled={busy||awaitingConfigurationSave} onClick={()=>undoConfiguration(field.key)}>{t("撤销", "Undo")}</Button></td></tr>)}</tbody></table>
 <p>{t("应用更改后保存配置，不自动重启。", "Applying changes saves the configuration without restarting.")}</p><div className="xcss-actions"><Button type="button" disabled={blocked||configLoading||!supportsConfigurationOverwrite(device.capabilities)} onClick={applyConfiguration}>{t("应用更改", "Apply changes")}</Button></div>
 {!supportsConfigurationOverwrite(device.capabilities)&&<p>{t("客户端尚未报告完整配置保存能力。", "The client has not reported complete configuration save support.")}</p>}
 </section>:<p>{t("没有需要应用的更改", "No changes to apply")}</p>}
 {configFeedback&&<p aria-label={t("配置操作状态", "Configuration operation status")} role={configFeedback.error?"alert":"status"}>{configFeedback.message}</p>}
 </section></section>}
 {page==="details"&&<SunshineControls device={device} operations={operations} busy={busy} blocked={blocked} section={controlSection} submit={submit} confirm={setConfirm}/>}
 {page==="logs"&&<section className="xcss-content-stack sunshine-logs-panel" aria-label={t("操作日志", "Operation logs")}>
  <div className="xcss-content-panel xcss-content-stack">
   <p>{t("实例：{0}", "Instance: {0}", [device.name])}</p>
   <div className="xcss-log-date-controls">
    <label htmlFor="sunshine-log-date-start-year">{t("日志日期范围（UTC）", "Log date range (UTC)")}</label>
    <div className="xcss-actions"><Button disabled={calendarLoading||logLoading||(logRange!==null&&!logDraftValid)} onClick={()=>{setLogCursor(null);if(logRange===null)setCalendarRefresh(value=>value+1);else setLogRefresh(value=>value+1)}}>{t("刷新日志", "Refresh logs")}</Button></div>
    <DateRangeField id="sunshine-log-date" value={logRange} disabled={calendarLoading} onValidityChange={setLogDraftValid} onApply={range=>{calendarReady.current=true;setTaskFailure(null);setLogCursor(null);setLogNext(null);setLogPrevious(null);setLogRange(range);setLogRefresh(value=>value+1)}}/>
   </div>
  </div>
  {taskFailure&&<ErrorState requestId={taskFailure.requestId} onRetry={()=>logRange===null?setCalendarRefresh(value=>value+1):setLogRefresh(value=>value+1)}>{taskFailure.message}</ErrorState>}
  {calendarLoading&&<LoadingState>{t("正在读取 UTC 当天日期…", "Loading the current UTC date…")}</LoadingState>}
  {logLoading&&<LoadingState>{t("正在读取所选日期的日志页…", "Loading a log page for the selected date…")}</LoadingState>}
  {!logLoading&&logDateValid&&!taskFailure&&loadedLogRange===logRange&&(visibleLogOperations.length===0?<EmptyState>{t("暂无日志", "No logs yet")}</EmptyState>:<Table aria-label={t("操作日志", "Operation logs")}>
   <thead><tr><th>{t("时间（UTC）", "Time (UTC)")}</th><th>{t("操作", "Action")}</th><th>{t("状态", "Status")}</th><th>{t("详情", "Details")}</th></tr></thead>
   <tbody>{visibleLogOperations.map(operation=><tr key={operation.operation_id}>
    <td><time>{operation.created_at_server}</time></td><td>{operationLabel(operation.action)}</td><td>{tasks[operation.state]}</td>
    <td><article className="xcss-content-stack"><code>{operation.operation_id}</code>
     {operation.resolution&&<p>{t("人工结论：", "Manual resolution:")}{operationLabel(operation.resolution)}</p>}
     {operation.status_reason&&<p>{t("状态原因：", "State reason:")}{operationLabel(operation.status_reason)}</p>}
     {operation.result&&<p>{t("结果：", "Result:")}{operationLabel(operation.result.kind)} {operation.result.reason?operationLabel(operation.result.reason):""}</p>}
     {(operation.reconciliation?.process_generation??operation.result?.process_generation)&&<p>{t("Sunshine 进程证据：", "Sunshine process evidence: ")}<code>{operation.reconciliation?.process_generation??operation.result?.process_generation}</code></p>}
     {operation.result?.kind==="conflict"&&<p role="alert">{t("配置冲突，请重新读取并核对差异。", "Configuration conflict. Read the configuration again and review the differences.")}</p>}
     {operation.reconciliation&&<p>{t("客户端 核对证据：", "Client reconciliation evidence:")}{operationLabel(operation.reconciliation.kind)} {operation.reconciliation.reason?operationLabel(operation.reconciliation.reason):""}{t("。证据不会自动代替管理员确认。", ". Evidence does not replace administrator confirmation.")}</p>}
     {operation.state==="unknown"&&<div className="xcss-actions">{[["confirmed_succeeded",t("确认已成功", "Confirm success")],["confirmed_failed",t("确认未成功", "Confirm failure")],["unable_to_confirm",t("仍无法确认", "Still unable to confirm")]].map(([resolution,label])=><Button key={resolution} disabled={busy} onClick={()=>setConfirm({title:label,description:t("请先核对实际设备状态。此操作只记录人工结论，不会重复执行原操作。", "Check the actual device state first. This records a manual conclusion only; it does not rerun the original operation."),run:()=>perform(async()=>{const updated=await client.request(`${CURRENT_API_PREFIX}/sunshine/operations/${operation.operation_id}/resolve`,isOperation,{method:"POST",body:JSON.stringify({resolution})});operationRevision.current+=1;setLogOperations(values=>values.map(value=>value.operation_id===updated.operation_id?updated:value))})})}>{label}</Button>)}</div>}
    </article></td>
   </tr>)}</tbody>
  </Table>)}
  <nav className="xcss-actions" aria-label={t("日志分页", "Log pages")}>
   <Button disabled={logLoading||calendarLoading||loadedLogRange!==logRange||!logDateValid||!logDraftValid||taskFailure!==null||logCursor===null} onClick={()=>setLogCursor(null)}>{t("首页", "First page")}</Button>
   <Button disabled={logLoading||calendarLoading||loadedLogRange!==logRange||!logDateValid||!logDraftValid||taskFailure!==null||logPrevious===null} onClick={()=>setLogCursor(logPrevious)}>{t("上一页", "Previous page")}</Button>
   <Button disabled={logLoading||calendarLoading||loadedLogRange!==logRange||!logDateValid||!logDraftValid||taskFailure!==null||logNext===null} onClick={()=>setLogCursor(logNext)}>{t("下一页", "Next page")}</Button>
  </nav>
 </section>}

 {confirm&&<ConfirmDangerDialog title={confirm.title} description={confirm.description} pending={busy} onClose={()=>setConfirm(null)} onConfirm={()=>{const action=confirm;setConfirm(null);void action.run()}}/>}
 </div>;
}
