import assert from 'node:assert/strict';
import {test} from 'node:test';
import {originalModule} from './fixtures/original-module.mjs';
import {HookHost,walk,textContent} from './fixtures/hook-host.mjs';
const api=await originalModule(new URL('../src/api.ts',import.meta.url),{'@xcss/web/admin-web':'export function createAdministratorApiClient(){return {}}'});
globalThis.__workspaceApi=api;
const {ClientWorkspace}=await originalModule(new URL('../src/ClientWorkspace.tsx',import.meta.url),{
 '@xcss/web/admin-ui':'export const Button="button",ConfirmDangerDialog="confirm",EmptyState="empty",ErrorState="error",FormField="field",LoadingState="loading",Table="table",TextField="input";',
 '@xcss/web/admin-shell':'export function useAdminApplication(){return globalThis.__workspaceApplication};export function errorRequestId(){return undefined}',
 '@xcss/web/admin-ui/i18n':'export function t(zh,en){return en}',
 '@xcss/web/admin-ui/date-range':'export const DateRangeField="date-range";',
 '@xcss/web/admin-ui/date-range.css':'',
 './display-labels':'export const operationLabel=value=>value,configValueLabel=value=>value;',
 './ConfigSelect':'export const ConfigSelect="select";',
 './SunshineControls':'export const SunshineControls="sunshine-controls",sunshineManagementCategories=[];export function isSunshineManagementCategory(){return false}',
 './configuration-feedback':'export function isConfigurationCommand(command){return ["read_config","save_config","restart"].includes(command.kind)};export function configurationFeedback(){return {message:"fixture",error:false}}',
 './config-fields':'export const configCategories=[{id:"general",label:"General"}];export function fieldsForDevice(fields){return fields.map(field=>({...field,category:"general",label:field.key,inputKind:field.kind,boolean:false,options:field.options??[]}))};export function prerequisiteLabel(){return undefined}',
 './api':'export const {CURRENT_API_PREFIX,isClientAuthorization,isDevice,isOperation,isOperations,isOperationPage,isOperationSummary,isTaskCalendar,isSnapshot,supportsConfigurationOverwrite,currentErrorEnvelope}=globalThis.__workspaceApi;'
});
const snapshot=(revision,qp)=>({revision:revision.repeat(64),sunshine_version:'2026.914.233613',fields:{qp},effectiveness:'awaiting_restart'});
function fixture({readRevision="a",readPending=false}={}){
 globalThis.window={setTimeout:()=>0};
 const initial=snapshot(readRevision,'28');const commands=[];let operations=[];let clock=10;
 const device={id:'00000000-0000-4000-8000-000000000001',name:'Fixture',registered:true,revoked:false,client_online:true,snapshot:snapshot('a','28'),configuration_state:'awaiting_restart',capabilities:{protocol:'xscs-management/1',configuration_overwrite:true,restart_allowed:true}};
 function operation(command,result,state){return {operation_id:`op-${++clock}`,device_id:device.id,action:command==='read_config'?'sunshine.config.read':command==='save_config'?'sunshine.config.save':'sunshine.restart',state,attempt:1,created_at_micros:clock,created_at_server:'fixture',updated_at_micros:clock,result,reconciliation:null,resolution:null}}
 const client={request:async(path,validate,options)=>{
  if(path.endsWith('/authorization'))return {device_id:device.id,manager_id:'fixture',authorization_code:'fixture'};
  if(path.endsWith('/tasks/summary'))return {blocking_count:operations.filter(value=>value.action!=='sunshine.config.read'&&['pending','running','unknown'].includes(value.state)).length};
  if(path.includes('/tasks?recent='))return structuredClone(operations);
  if(path.endsWith('/tasks')&&options?.method==='POST'){
   const command=JSON.parse(options.body);commands.push(command);
   const next=operation(command.kind,command.kind==='read_config'?{kind:'config_read',snapshot:initial}:null,command.kind==='read_config'&&!readPending?'succeeded':'pending');operations.unshift(next);return structuredClone(next);
  }
  throw new Error(`Unexpected fixture request: ${path}`);
 }};
 globalThis.__workspaceApplication={client,notify(){}};
 const host=new HookHost(ClientWorkspace,{device,fieldDefinitions:[{key:'qp',kind:'integer',minimum:0,maximum:51}],refreshSignal:0,configRefreshSignal:0,changed(){},removed(){},ticket:null,page:'details'});
 const find=predicate=>walk(host.render(),predicate)[0];const button=label=>find(node=>node.type==='button'&&textContent(node)===label);
 const settle=async()=>{for(let n=0;n<8;n++){await Promise.resolve();host.render()}};
 const refresh=async()=>{host.props={...host.props,refreshSignal:host.props.refreshSignal+1};host.render();await settle()};
 const setQp=value=>find(node=>node.type==='input'&&node.props.id==='sunshine-field-qp').props.onChange({target:{value}});
 const complete=()=>{const save=operations.find(value=>value.action==='sunshine.config.save');save.state='succeeded';save.result={kind:'config_saved',snapshot:snapshot('b','29')};save.updated_at_micros=++clock};
 const reconcile=(resolution,report={kind:'config_saved',snapshot:snapshot('b','29')})=>{const save=operations.find(value=>value.action==='sunshine.config.save');save.state=resolution===null?'unknown':'resolved';save.result={kind:'unknown',reason:'effect_not_confirmed'};save.reconciliation=report;save.resolution=resolution;save.reconciliation_observed_at_micros=++clock;save.updated_at_micros=++clock};
 const confirm=()=>find(node=>node.type==='confirm');
 return {host,commands,button,setQp,settle,refresh,complete,reconcile,confirm};
}
test('restart immediately after a confirmed save uses its revision before device polling catches up',async()=>{
 const f=fixture();f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();f.setQp('29');f.button('Apply changes').props.onClick();await f.settle();f.complete();await f.refresh();
 assert.equal(f.host.props.device.snapshot.revision,'a'.repeat(64));
 f.button('Restart Sunshine').props.onClick();
 const confirmation=f.confirm();assert.ok(confirmation);await confirmation.props.onConfirm();await f.settle();
 assert.equal(f.commands.at(-1).kind,'restart');assert.equal(f.commands.at(-1).expected_revision,'b'.repeat(64));
});


test('restart uses the freshly read baseline while unsaved field edits remain only a draft',async()=>{
 const f=fixture({readRevision:'b'});f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();f.setQp('31');
 f.button('Restart Sunshine').props.onClick();await f.confirm().props.onConfirm();await f.settle();
 assert.equal(f.commands.at(-1).kind,'restart');assert.equal(f.commands.at(-1).expected_revision,'b'.repeat(64));
 assert.equal(f.commands.filter(command=>command.kind==='save_config').length,0);
 assert.equal(walk(f.host.render(),node=>node.type==='input'&&node.props.id==='sunshine-field-qp')[0].props.value,'31');
});

test('a cached device snapshot alone cannot enable configuration restart before the requested read',async()=>{
 const f=fixture({readPending:true});f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();
 assert.equal(f.button('Restart Sunshine').props.disabled,true);assert.equal(f.commands.filter(command=>command.kind==='restart').length,0);
});


test('confirmed successful recovery rebases restart while preserving subsequent draft edits',async()=>{
 const f=fixture();f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();f.setQp('29');f.button('Apply changes').props.onClick();await f.settle();
 f.setQp('31');f.reconcile(null);await f.refresh();
 assert.equal(f.button('Restart Sunshine').props.disabled,true);
 f.reconcile('confirmed_succeeded');await f.refresh();
 assert.equal(walk(f.host.render(),node=>node.type==='input'&&node.props.id==='sunshine-field-qp')[0].props.value,'31');
 f.button('Restart Sunshine').props.onClick();await f.confirm().props.onConfirm();await f.settle();
 assert.equal(f.commands.at(-1).kind,'restart');assert.equal(f.commands.at(-1).expected_revision,'b'.repeat(64));
});

test('successful reconciliation clears the submitted change from the preview',async()=>{
 const f=fixture();f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();f.setQp('29');f.button('Apply changes').props.onClick();await f.settle();
 f.reconcile('confirmed_succeeded');await f.refresh();
 assert.equal(f.button('Apply changes'),undefined);
 assert.equal(walk(f.host.render(),node=>node.type==='input'&&node.props.id==='sunshine-field-qp')[0].props.value,'29');
});

for(const [label,resolution,report] of [
 ['failed resolution','confirmed_failed',undefined],
 ['unconfirmed resolution','unable_to_confirm',undefined],
 ['missing evidence','confirmed_succeeded',null],
 ['wrong evidence kind','confirmed_succeeded',{kind:'config_read',snapshot:snapshot('b','29')}],
 ['invalid snapshot','confirmed_succeeded',{kind:'config_saved',snapshot:{revision:'invalid'}}],
])test(`${label} does not adopt a recovered configuration`,async()=>{
 const f=fixture();f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();f.setQp('29');f.button('Apply changes').props.onClick();await f.settle();
 f.reconcile(resolution,report);await f.refresh();
 assert.equal(f.button('Apply changes').props.disabled,false);
 f.button('Restart Sunshine').props.onClick();await f.confirm().props.onConfirm();await f.settle();
 assert.equal(f.commands.at(-1).expected_revision,'a'.repeat(64));
 assert.equal(walk(f.host.render(),node=>node.type==='input'&&node.props.id==='sunshine-field-qp')[0].props.value,'29');
});

for(const value of ['A#B','A=B','A[B]',' Name','Name\u0085','😀'.repeat(33)])test(`invalid configuration text stays visible and cannot send SaveConfig (${JSON.stringify(value)})`,async()=>{
 const f=fixture();f.host.props.fieldDefinitions=[{key:'sunshine_name',kind:'text',maximum_length:32}];f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();
 const field=()=>walk(f.host.render(),node=>node.type==='input'&&node.props.id==='sunshine-field-sunshine_name')[0];field().props.onChange({target:{value}});f.host.render();assert.equal(field().props.value,value);assert.equal(field().props.maxLength,undefined);assert.equal(field().props.pattern,undefined);assert.equal(field().props['aria-invalid'],true);assert.ok(walk(f.host.tree,node=>node.props?.role==='alert').length);
 const before=f.commands.length;globalThis.requestAnimationFrame=()=>{};f.button('Apply changes').props.onClick();await f.settle();assert.equal(f.commands.length,before);assert.equal(field().props.value,value);delete globalThis.requestAnimationFrame;
});
test('configuration text counts Unicode scalar characters and sends 32 astral characters unchanged',async()=>{
 const f=fixture();f.host.props.fieldDefinitions=[{key:'sunshine_name',kind:'text',maximum_length:32}];f.host.render();await f.settle();await f.refresh();f.button('General').props.onClick();const value='😀'.repeat(32);walk(f.host.render(),node=>node.type==='input'&&node.props.id==='sunshine-field-sunshine_name')[0].props.onChange({target:{value}});f.host.render();f.button('Apply changes').props.onClick();await f.settle();assert.equal(f.commands.at(-1).set.sunshine_name,value);
});
