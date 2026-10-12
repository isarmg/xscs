import assert from 'node:assert/strict';
import {test} from 'node:test';
import {originalModule} from './fixtures/original-module.mjs';
import {HookHost,walk,textContent} from './fixtures/hook-host.mjs';
const source=name=>new URL('../src/'+name,import.meta.url);
const api=await originalModule(source('api.ts'),{'@xcss/web/admin-web':'export function createAdministratorApiClient(){return {}}'});globalThis.__evictionApi=api;
const controls=await originalModule(source('SunshineControls.tsx'),{
 '@xcss/web/admin-ui':'export const Button="button",FormField="field",TextField="input";',
 '@xcss/web/admin-ui/i18n':'export function t(zh,en){return en}',
 './ConfigSelect':'export const ConfigSelect="select";',
 './api':'export function supportsPendingPairingListing(){return true}'
});globalThis.__evictionControls=controls;
const {ClientWorkspace}=await originalModule(source('ClientWorkspace.tsx'),{
 '@xcss/web/admin-ui':'export const Button="button",ConfirmDangerDialog="confirm",EmptyState="empty",ErrorState="error",FormField="field",LoadingState="loading",Table="table",TextField="input";',
 '@xcss/web/admin-shell':'export function useAdminApplication(){return globalThis.__evictionApplication};export function errorRequestId(){return undefined}',
 '@xcss/web/admin-ui/i18n':'export function t(zh,en){return en}',
 '@xcss/web/admin-ui/date-range':'export const DateRangeField="date-range";',
 '@xcss/web/admin-ui/date-range.css':'',
 './display-labels':'export const operationLabel=value=>value,configValueLabel=value=>value;',
 './ConfigSelect':'export const ConfigSelect="select";',
 './SunshineControls':'export const {SunshineControls,sunshineManagementCategories,isSunshineManagementCategory}=globalThis.__evictionControls;',
 './configuration-feedback':'export function isConfigurationCommand(command){return ["read_config","save_config","restart"].includes(command.kind)};export function configurationFeedback(){return {message:"fixture",error:false}}',
 './config-fields':'export const configCategories=[{id:"general",label:"General"}];export function fieldsForDevice(fields){return fields.map(field=>({...field,category:"general",label:field.key,inputKind:"integer",boolean:false,options:[]}))};export function prerequisiteLabel(){return undefined}',
 './api':'export const {CURRENT_API_PREFIX,isClientAuthorization,isDevice,isOperation,isOperations,isOperationPage,isOperationSummary,isTaskCalendar,isSnapshot,supportsConfigurationOverwrite,currentErrorEnvelope}=globalThis.__evictionApi;'
});
const spec={name:'Original',output:'',cmd:'','working-dir':'','exclude-global-prep-cmd':false,elevated:false,'auto-detach':true,'wait-all':true,'exit-timeout':5,'prep-cmd':[],detached:[],'image-path':''};
const ref=c=>({fingerprint:c.repeat(64)});
const appSnapshot=(c,specification)=>({revision:c.repeat(64),applications:[{reference:ref(c),specification}]});
const config={revision:'a'.repeat(64),sunshine_version:'2026.914.233613',fields:{qp:'28'},effectiveness:'pending_verification'};
function fixture({deviceId='fixture'}={}){
 let timer;globalThis.window={setTimeout:(callback,delay)=>{if(delay===2000)timer=callback;return 0}};let clock=0;const operations=[];const requests=[];const commands=[];let applications=appSnapshot('a',spec);let receiptFailure=false;let receiptChange=value=>value;let holdReceipt=false;const held=[];
 const device={id:deviceId,name:'Fixture',registered:true,revoked:false,configuration_state:'pending_verification',capabilities:{protocol:'xscs-management/1',configuration_overwrite:true,application_management:true}};
 const client={request:async(path,guard,init)=>{
  requests.push({path,method:init?.method??'GET'});
  if(path.endsWith('/authorization'))return {};
  if(path.endsWith('/tasks/summary'))return {blocking_count:operations.filter(v=>['sunshine.applications.save','sunshine.config.save'].includes(v.action)&&['pending','running','unknown'].includes(v.state)).length};
  if(path.includes('/tasks?recent='))return structuredClone(operations.slice(0,50));
  if(path.includes('/sunshine/operations/')){
   if(receiptFailure)throw Error('Receipt transport failure');
   const receipt=receiptChange(structuredClone(operations.find(v=>v.operation_id===path.split('/').at(-1))));
   if(holdReceipt){holdReceipt=false;return new Promise(resolve=>held.push(()=>resolve(receipt)))}
   return receipt;
  }
  if(path.endsWith('/tasks')&&init?.method==='POST'){
   const command=JSON.parse(init.body);commands.push(command);const id='op-'+(++clock);const save=command.kind==='save_application'||command.kind==='save_config';
   const action=command.kind==='read_config'?'sunshine.config.read':save?(command.kind==='save_config'?'sunshine.config.save':'sunshine.applications.save'):'sunshine.applications.list';
   const result=command.kind==='read_config'?{kind:'config_read',snapshot:config}:save?null:{kind:'applications_read',snapshot:structuredClone(applications)};
   const operation={operation_id:id,device_id:device.id,action,state:save?'pending':'succeeded',attempt:1,created_at_micros:clock,created_at_server:'fixture',updated_at_micros:clock,result,reconciliation:null,resolution:null};
   if(command.kind==='save_application')applications=appSnapshot('b',command.application);operations.unshift(operation);return structuredClone(operation);
  }
  throw Error('Unhandled '+path);
 }};globalThis.__evictionApplication={client,notify(){}};
 const host=new HookHost(ClientWorkspace,{device,fieldDefinitions:[{key:'qp',kind:'integer',minimum:0,maximum:51}],refreshSignal:0,configRefreshSignal:0,changed(){},removed(){},ticket:null,page:'details'});let child;
 const render=()=>{const tree=host.render();const node=walk(tree,node=>node.type===controls.SunshineControls)[0];if(!child)child=new HookHost(controls.SunshineControls,node.props);else child.props=node.props;return child.render()};
 const settle=async()=>{for(let i=0;i<8;i++){await Promise.resolve();render()}};
 const button=name=>walk(render(),node=>node.type==='button'&&textContent(node)===name)[0];
 const editor=()=>walk(render(),node=>typeof node.type==='function'&&node.type.name==='ApplicationEditor')[0];
 const refresh=async()=>{host.props={...host.props,refreshSignal:host.props.refreshSignal+1};render();await settle()};
 const workspaceButton=name=>walk(host.render(),node=>node.type==='button'&&textContent(node)===name)[0];
 const qp=()=>walk(host.render(),node=>node.type==='input'&&node.props.id==='sunshine-field-qp')[0];
 const poll=async()=>{assert.ok(timer,'existing poll must be scheduled');const run=timer;timer=undefined;run();await settle()};
 return {host,requests,commands,operations,render,settle,button,workspaceButton,editor,refresh,poll,qp,applicationSnapshot:()=>applications,visibleOperations:()=>{render();return child.props.operations},errors:()=>textContent(walk(host.render(),node=>node.type==='error')),failReceipt:value=>{receiptFailure=value},changeReceipt:fn=>{receiptChange=fn},holdReceipt:()=>{holdReceipt=true},releaseReceipt:()=>{assert.ok(held.length);held.shift()()},unmount:()=>{host.unmount();child?.unmount()}};
}

const receiptQueries=f=>f.requests.filter(value=>value.path.includes('/sunshine/operations/'));
async function initialize(){const f=fixture();f.render();await f.settle();await f.refresh();f.button('Refresh applications').props.onClick();await f.settle();return f}
async function startApplicationSave(f){f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,name:'Submitted'});f.editor().props.onSubmit({preventDefault(){}});await f.settle();return f.operations.find(value=>value.action==='sunshine.applications.save')}
async function evict(f){for(let index=0;index<50;index++){assert.equal(f.button('Refresh applications').props.disabled,false);f.button('Refresh applications').props.onClick();await f.settle()}}
function finishApplication(f,operation,state){
 operation.state=state==='confirmed_succeeded'||state==='confirmed_failed'||state==='unable_to_confirm'?'resolved':state;
 operation.updated_at_micros=1000;
 operation.result=state==='succeeded'?{kind:'application_saved',snapshot:f.applicationSnapshot()}:state==='failed'||state==='dead_letter'?{kind:'rejected',reason:'resource_conflict'}:{kind:'unknown',reason:'effect_not_confirmed'};
 if(operation.state==='resolved')operation.resolution=state;
 if(state==='confirmed_succeeded'){operation.reconciliation={kind:'application_saved',snapshot:f.applicationSnapshot()};operation.reconciliation_observed_at_micros=999}
}

for(const state of ['succeeded','failed','dead_letter','confirmed_succeeded','confirmed_failed','unable_to_confirm'])test(`an evicted application save with ${state} receipt settles the editor and survives subsequent polls`,async()=>{
 const f=await initialize();const save=await startApplicationSave(f);
 if(state.startsWith('confirmed')||state==='unable_to_confirm'){save.state='unknown';save.result={kind:'unknown',reason:'effect_not_confirmed'};await f.poll()}
 f.editor().props.setApplication({...spec,name:'Later unsaved edit'});await evict(f);finishApplication(f,save,state);await f.poll();
 assert.equal(f.operations.slice(0,50).some(value=>value.operation_id===save.operation_id),false);
 assert.equal(f.editor().props.blocked,false);assert.equal(f.editor().props.application.name,'Later unsaved edit');assert.equal(receiptQueries(f).length,1);
 await f.poll();await f.poll();assert.equal(f.editor().props.blocked,false);assert.equal(receiptQueries(f).length,1,'terminal receipt must be retained without another GET');
 const before=f.commands.length;f.editor().props.onSubmit({preventDefault(){}});await f.settle();assert.equal(f.commands.length,before+1);
 assert.deepEqual(f.commands.at(-1).target,ref(state==='succeeded'||state==='confirmed_succeeded'?'b':'a'));
});

test('a single-receipt request failure exposes the error and preserves the draft and uncertain lock until retry succeeds',async()=>{
 const f=await initialize();const save=await startApplicationSave(f);save.state='unknown';save.result={kind:'unknown',reason:'effect_not_confirmed'};await f.poll();
 f.editor().props.setApplication({...spec,name:'Do not discard'});await evict(f);finishApplication(f,save,'confirmed_succeeded');f.failReceipt(true);await f.poll();
 assert.match(f.errors(),/save receipt is temporarily unavailable/);assert.equal(f.editor().props.blocked,true);assert.equal(f.editor().props.application.name,'Do not discard');
 const before=f.commands.length;f.editor().props.onSubmit({preventDefault(){}});await f.settle();assert.equal(f.commands.length,before);
 f.failReceipt(false);await f.poll();assert.doesNotMatch(f.errors(),/save receipt/);assert.equal(f.editor().props.blocked,false);assert.equal(f.editor().props.application.name,'Do not discard');
});

for(const field of ['operation_id','device_id','action'])test(`a mismatched ${field} on a single receipt cannot unlock or rebase the editor`,async()=>{
 const f=await initialize();const save=await startApplicationSave(f);await evict(f);finishApplication(f,save,'succeeded');f.changeReceipt(value=>({...value,[field]:'wrong'}));await f.poll();
 assert.match(f.errors(),/save receipt is temporarily unavailable/);assert.equal(f.editor().props.blocked,true);assert.equal(f.editor().props.application.name,'Submitted');
});

test('an old load cannot roll back a newer single-operation receipt',async()=>{
 const f=await initialize();const save=await startApplicationSave(f);await evict(f);f.holdReceipt();await f.poll();
 finishApplication(f,save,'succeeded');await f.refresh();assert.equal(f.editor().props.blocked,false);
 f.releaseReceipt();await f.settle();assert.equal(f.editor().props.blocked,false);assert.equal(f.visibleOperations().find(value=>value.operation_id===save.operation_id).state,'succeeded');
 await f.poll();assert.equal(f.editor().props.blocked,false);
});

test('a completed configuration save outside the page rebases only its submitted values',async()=>{
 const f=await initialize();f.workspaceButton('General').props.onClick();f.qp().props.onChange({target:{value:'29'}});f.workspaceButton('Apply changes').props.onClick();await f.settle();
 const save=f.operations.find(value=>value.action==='sunshine.config.save');assert.ok(save);f.qp().props.onChange({target:{value:'31'}});await evict(f);
 save.state='succeeded';save.updated_at_micros=1000;save.result={kind:'config_saved',snapshot:{...config,revision:'b'.repeat(64),fields:{qp:'29'},effectiveness:'awaiting_restart'}};await f.poll();
 assert.equal(f.qp().props.value,'31');assert.equal(f.workspaceButton('Apply changes').props.disabled,false);assert.equal(receiptQueries(f).length,1);
 await f.poll();assert.equal(f.qp().props.value,'31');assert.equal(f.workspaceButton('Apply changes').props.disabled,false);assert.equal(receiptQueries(f).length,1);
});

test('a configuration editor generation change discards a late single-operation receipt',async()=>{
 const f=await initialize();f.workspaceButton('General').props.onClick();f.qp().props.onChange({target:{value:'29'}});f.workspaceButton('Apply changes').props.onClick();await f.settle();
 const save=f.operations.find(value=>value.action==='sunshine.config.save');await evict(f);save.state='succeeded';save.updated_at_micros=1000;save.result={kind:'config_saved',snapshot:{...config,revision:'b'.repeat(64),fields:{qp:'29'},effectiveness:'awaiting_restart'}};f.holdReceipt();await f.poll();
 f.host.props={...f.host.props,configRefreshSignal:1};f.render();await f.settle();assert.equal(f.qp().props.value,'28');f.qp().props.onChange({target:{value:'33'}});
 f.releaseReceipt();await f.settle();assert.equal(f.qp().props.value,'33');await f.poll();assert.equal(f.qp().props.value,'33');
 assert.equal(receiptQueries(f).length,1,'retired configuration generation must not keep requesting its receipt');
});

test('receipt storage is bounded to the current save for each editor',async()=>{
 const f=await initialize();f.workspaceButton('General').props.onClick();f.qp().props.onChange({target:{value:'29'}});f.workspaceButton('Apply changes').props.onClick();await f.settle();
 const configSave=f.operations.find(value=>value.action==='sunshine.config.save');configSave.state='succeeded';configSave.updated_at_micros=1000;configSave.result={kind:'config_saved',snapshot:{...config,revision:'b'.repeat(64),fields:{qp:'29'},effectiveness:'awaiting_restart'}};await f.poll();
 const first=await startApplicationSave(f);finishApplication(f,first,'succeeded');await f.poll();f.editor().props.setApplication({...spec,name:'Second save'});f.editor().props.onSubmit({preventDefault(){}});await f.settle();
 const second=f.operations.find(value=>value.action==='sunshine.applications.save');assert.notEqual(second.operation_id,first.operation_id);finishApplication(f,second,'succeeded');await f.poll();await evict(f);await f.poll();
 assert.equal(f.visibleOperations().length,52);assert.equal(f.visibleOperations().some(value=>value.operation_id===first.operation_id),false);assert.equal(f.visibleOperations().some(value=>value.operation_id===second.operation_id),true);assert.equal(f.visibleOperations().some(value=>value.operation_id===configSave.operation_id),true);
 assert.equal(receiptQueries(f).length,0,'receipts already settled in the page need no single-operation request');
});

test('a receipt from an unmounted device cannot change the next device editor',async()=>{
 const old=await initialize();const save=await startApplicationSave(old);await evict(old);finishApplication(old,save,'succeeded');old.holdReceipt();await old.poll();old.unmount();
 const current=fixture({deviceId:'another-device'});current.render();await current.settle();await current.refresh();current.button('Refresh applications').props.onClick();await current.settle();current.button('Edit').props.onClick();current.editor().props.setApplication({...spec,name:'Next device draft'});
 old.releaseReceipt();await current.settle();
 assert.equal(current.editor().props.application.name,'Next device draft');assert.equal(current.editor().props.blocked,false);assert.equal(receiptQueries(current).length,0);
 assert.equal(current.visibleOperations().every(value=>value.device_id==='another-device'),true);
});
