import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFile} from 'node:fs/promises';
import {originalModule} from './fixtures/original-module.mjs';
import {HookHost,walk,textContent} from './fixtures/hook-host.mjs';
const {SunshineControls,ApplicationEditor}=await originalModule(new URL('../src/SunshineControls.tsx',import.meta.url),{
 '@xcss/web/admin-ui':'export const Button="button",FormField="field",TextField="input";',
 '@xcss/web/admin-ui/i18n':'export function t(zh,en){return en}',
 './ConfigSelect':'export const ConfigSelect="select"',
 './api':'export function supportsPendingPairingListing(){return true}'
},['ApplicationEditor']);
const spec={name:'Original',output:'',cmd:'','working-dir':'','exclude-global-prep-cmd':false,elevated:false,'auto-detach':false,'wait-all':false,'exit-timeout':5,'prep-cmd':[],detached:[],'image-path':''};
const ref=value=>({fingerprint:value.repeat(64)});
function fixture(){
 const initial={operation_id:'read-1',state:'succeeded',updated_at_micros:1,result:{kind:'applications_read',snapshot:{revision:'a'.repeat(64),applications:[{reference:ref('b'),specification:spec}]}},reconciliation:null};
 const commands=[];let confirmation,delayAcceptance=false,releaseAcceptance;
 const props={device:{id:'fixture',capabilities:{protocol:'xscs-management/1',application_management:true,application_host_commands_allowed:true,service_control:true}},operations:[initial],busy:false,blocked:false,section:'applications',
  submit:async(command,onAccepted)=>{commands.push(command);const operation={operation_id:'save-'+commands.length,state:'pending',updated_at_micros:10+commands.length,result:null,reconciliation:null};if(delayAcceptance)await new Promise(resolve=>{releaseAcceptance=resolve});onAccepted?.(operation);return true},
  confirm:value=>{confirmation=value}
 };
 const host=new HookHost(SunshineControls,props);
 const find=predicate=>walk(host.render(),predicate)[0];
 const editor=()=>find(node=>typeof node.type==='function'&&node.type.name==='ApplicationEditor');
 const button=label=>find(node=>node.type==='button'&&textContent(node)===label);
 const update=operations=>{host.props={...host.props,device:{...host.props.device},operations};host.render()};
 const complete=(index=0,state='succeeded')=>{
  const command=commands[index];const result=state==='succeeded'?{kind:'application_saved',snapshot:{revision:'c'.repeat(64),applications:[{reference:ref('d'),specification:structuredClone(command.application)}]}}:{kind:'rejected',reason:'resource_conflict'};
  const operation={operation_id:'save-'+(index+1),state,updated_at_micros:100+index,result,reconciliation:null};
  update([operation,...host.props.operations.filter(value=>value.operation_id!==operation.operation_id)]);return operation;
 };
 const save=async()=>{editor().props.onSubmit({preventDefault(){}});await Promise.resolve();await Promise.resolve();host.render()};
 return {host,commands,editor,button,save,complete,update,confirmation:()=>confirmation,delay:()=>{delayAcceptance=true},release:()=>releaseAcceptance()};
}
test('successful application edit rebases target without discarding subsequent field or detached-command edits',async()=>{
 const f=fixture();f.button('Edit').props.onClick();
 f.editor().props.setApplication({...spec,name:'First save'});await f.save();
 assert.equal(f.editor().props.blocked,true);
 f.editor().props.setApplication({...spec,name:'Later draft'});f.editor().props.setDetachedText('later command\n');
 f.complete();
 assert.equal(f.editor().props.application.name,'Later draft');assert.equal(f.editor().props.detachedText,'later command\n');
 await f.save();await f.confirmation().run();
 assert.deepEqual(f.commands[1].target,ref('d'));assert.equal(f.commands[1].expected_revision,'c'.repeat(64));assert.equal(f.commands[1].application.name,'Later draft');assert.deepEqual(f.commands[1].application.detached,['later command']);
});
test('successful creation becomes editing and cannot create the same app again',async()=>{
 const f=fixture();f.button('New application').props.onClick();f.editor().props.setApplication({...spec,name:'Created'});await f.save();
 f.complete();assert.equal(f.editor().props.editing,true);await f.save();
 assert.equal(f.commands[0].target,null);assert.deepEqual(f.commands[1].target,ref('d'));
});
test('success stays blocked until its matching identity is adopted, including an inconsistent receipt',async()=>{
 const f=fixture();f.button('New application').props.onClick();f.editor().props.setApplication({...spec,name:'Submitted'});await f.save();
 f.update([{operation_id:'save-1',state:'succeeded',updated_at_micros:100,result:{kind:'application_saved',snapshot:{revision:'c'.repeat(64),applications:[{reference:ref('d'),specification:{...spec,name:'Different receipt'}}]}},reconciliation:null}]);
 assert.equal(f.editor().props.blocked,true);await f.save();assert.equal(f.commands.length,1);
 f.complete();assert.equal(f.editor().props.blocked,false);assert.equal(f.editor().props.editing,true);
});
test('a canceled editor ignores an older accepted request and its later result',async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,name:'Canceled editor save'});f.delay();await f.save();
 f.editor().props.onCancel();f.button('New application').props.onClick();f.editor().props.setApplication({...spec,name:'New editor draft'});
 f.release();await Promise.resolve();await Promise.resolve();f.complete();
 assert.equal(f.editor().props.editing,false);assert.equal(f.editor().props.application.name,'New editor draft');
});
test('an already accepted save cannot rebase a replaced editor when its result arrives',async()=>{
 const f=fixture();f.button('Edit').props.onClick();await f.save();f.editor().props.onCancel();f.button('New application').props.onClick();f.editor().props.setApplication({...spec,name:'Replacement draft'});f.complete();
 assert.equal(f.editor().props.editing,false);assert.equal(f.editor().props.application.name,'Replacement draft');
});
test('failed saves preserve the draft and reference; unrelated reports do not rebase it',async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,name:'Retry draft'});await f.save();f.complete(0,'failed');
 f.update([{operation_id:'unrelated',state:'succeeded',updated_at_micros:200,result:{kind:'application_saved',snapshot:{revision:'e'.repeat(64),applications:[{reference:ref('f'),specification:spec}]}},reconciliation:null},...f.host.props.operations]);
 assert.equal(f.editor().props.application.name,'Retry draft');await f.save();assert.deepEqual(f.commands[1].target,ref('b'));
});
test('canceled host-command confirmation cannot submit against a new editor',async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,cmd:'launch'});await f.save();
 const confirmation=f.confirmation();f.editor().props.onCancel();f.button('New application').props.onClick();await confirmation.run();assert.equal(f.commands.length,0);
});
test('human resolution does not make old evidence newer than a later service read',()=>{
 const f=fixture();f.host.props.section='service';
 const old={operation_id:'old',state:'unknown',updated_at_micros:30,result:{kind:'unknown'},reconciliation:{kind:'service_controlled',action:'restart',state:'running'},reconciliation_observed_at_micros:35};
 const recent={operation_id:'recent',state:'succeeded',updated_at_micros:50,result:{kind:'service_status_read',state:'stopped'},reconciliation:null};
 const state=()=>textContent(walk(f.host.render(),node=>node.type==='section'&&node.props.id==='sunshine-service')[0]);
 f.update([recent,old]);assert.match(state(),/Current state:  stopped/);
 f.update([recent,{...old,state:'resolved',updated_at_micros:60}]);assert.match(state(),/Current state:  stopped/);
 // Fresh inspection evidence really is newer even though operation metadata is older.
 f.update([recent,{...old,reconciliation_observed_at_micros:70}]);assert.match(state(),/Current state:  running/);
 // Never infer evidence freshness from an optional legacy field being absent.
 f.update([recent,{...old,updated_at_micros:90,reconciliation_observed_at_micros:undefined}]);assert.match(state(),/Current state:  stopped/);
});
test('application snapshots also use actual evidence time after resolution',()=>{
 const f=fixture();const latest={operation_id:'read-later',state:'succeeded',updated_at_micros:50,result:{kind:'applications_read',snapshot:{revision:'e'.repeat(64),applications:[{reference:ref('f'),specification:{...spec,name:'Current application'}}]}},reconciliation:null};
 f.update([latest,{operation_id:'old-save',state:'resolved',updated_at_micros:90,result:{kind:'unknown'},reconciliation:{kind:'application_saved',snapshot:{revision:'a'.repeat(64),applications:[{reference:ref('b'),specification:{...spec,name:'Old application'}}]}},reconciliation_observed_at_micros:30}]);
 assert.equal(textContent(f.button('Edit')), 'Edit');f.button('Edit').props.onClick();assert.equal(f.editor().props.application.name,'Current application');
});
test('ordinary parent refresh preserves component identity and forwards accepted operation identity',async()=>{
 const app=await readFile(new URL('../src/App.tsx',import.meta.url),'utf8');const workspace=await readFile(new URL('../src/ClientWorkspace.tsx',import.meta.url),'utf8');
 assert.match(app,/<ClientWorkspace key=\{device.id\}/);assert.match(workspace,/<SunshineControls device=\{device\}/);assert.match(workspace,/onAccepted\?\.\(operation\)/);
});

test('administrator DTO checks the additive evidence timestamp without treating omission as freshness',async()=>{
 const {isOperation}=await originalModule(new URL('../src/api.ts',import.meta.url),{'@xcss/web/admin-web':'export function createAdministratorApiClient(){return {}}'});
 const operation={operation_id:'fixture',device_id:'device',action:'sunshine.service.control',state:'resolved',attempt:1,created_at_micros:1,created_at_server:'fixture',updated_at_micros:50,result:{kind:'unknown',reason:'service_transition_not_confirmed'},reconciliation:{kind:'service_controlled',action:'restart',state:'running'},resolution:'confirmed_succeeded'};
 assert.equal(isOperation(operation),true);assert.equal(isOperation({...operation,reconciliation_observed_at_micros:20}),true);
 for(const value of ['20',NaN,Infinity,Number.MAX_SAFE_INTEGER+1,{},[]])assert.equal(isOperation({...operation,reconciliation_observed_at_micros:value}),false);
});

test('new applications use Sunshine launch defaults and existing explicit false values survive editing',()=>{
 const f=fixture();f.button('New application').props.onClick();
 assert.equal(f.editor().props.application['auto-detach'],true);
 assert.equal(f.editor().props.application['wait-all'],true);
 f.editor().props.onCancel();f.button('Edit').props.onClick();
 assert.equal(f.editor().props.application['auto-detach'],false);
 assert.equal(f.editor().props.application['wait-all'],false);
});

test('a reconciled successful save adopts its application reference after manual resolution',async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,name:'Reconciled save'});await f.save();
 const snapshot={revision:'c'.repeat(64),applications:[{reference:ref('d'),specification:structuredClone(f.commands[0].application)}]};
 const uncertain={operation_id:'save-1',state:'unknown',updated_at_micros:100,result:{kind:'unknown',reason:'effect_not_confirmed'},reconciliation:{kind:'application_saved',snapshot},reconciliation_observed_at_micros:110,resolution:null};
 f.update([uncertain]);assert.equal(f.editor().props.blocked,true);
 f.editor().props.setApplication({...f.editor().props.application,name:'Later draft'});
 f.update([{...uncertain,state:'resolved',updated_at_micros:120,resolution:'confirmed_succeeded'}]);
 assert.equal(f.editor().props.application.name,'Later draft');assert.equal(f.editor().props.blocked,false);
 await f.save();assert.deepEqual(f.commands[1].target,ref('d'));assert.equal(f.commands[1].expected_revision,'c'.repeat(64));
});

test('manual failed resolution does not adopt a reconciliation reference',async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,name:'Retry original target'});await f.save();
 f.update([{operation_id:'save-1',state:'resolved',updated_at_micros:120,result:{kind:'unknown'},reconciliation:{kind:'application_saved',snapshot:{revision:'c'.repeat(64),applications:[{reference:ref('d'),specification:structuredClone(f.commands[0].application)}]}},reconciliation_observed_at_micros:110,resolution:'confirmed_failed'}]);
 await f.save();assert.deepEqual(f.commands[1].target,ref('b'));
});

test('reconciled creation becomes an edit instead of repeating creation',async()=>{
 const f=fixture();f.button('New application').props.onClick();f.editor().props.setApplication({...f.editor().props.application,name:'Created after inspection'});await f.save();
 assert.equal(f.commands[0].application['auto-detach'],true);assert.equal(f.commands[0].application['wait-all'],true);
 f.update([{operation_id:'save-1',state:'resolved',updated_at_micros:120,result:{kind:'unknown'},reconciliation:{kind:'application_saved',snapshot:{revision:'c'.repeat(64),applications:[{reference:ref('d'),specification:structuredClone(f.commands[0].application)}]}},reconciliation_observed_at_micros:110,resolution:'confirmed_succeeded'}]);
 assert.equal(f.editor().props.editing,true);await f.save();assert.deepEqual(f.commands[1].target,ref('d'));
});

for(const [field,value] of [['name','界'.repeat(43)],['name',' Name'],['cmd',' launch'],['image-path','x'.repeat(4097)],['exit-timeout',1.5]])test(`application ${field} invalid protocol input is visible and cannot submit a task`,async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,[field]:value});await f.save();assert.equal(f.commands.length,0);assert.equal(f.confirmation(),undefined);assert.equal(f.editor().props.application[field],value);
 const editorHost=new HookHost(ApplicationEditor,f.editor().props);editorHost.render();assert.ok(walk(editorHost.tree,node=>node.props?.role==='alert').length>0);assert.equal(walk(editorHost.tree,node=>node.type==='input'&&String(node.props.value)===String(value))[0].props.maxLength,undefined);editorHost.unmount();
});
test('detached command whitespace is rejected without being trimmed; valid command contents are sent unchanged',async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setDetachedText(' echo original \n');await f.save();assert.equal(f.commands.length,0);assert.equal(f.confirmation(),undefined);assert.equal(f.editor().props.detachedText,' echo original \n');
 f.editor().props.setDetachedText('echo  \"internal spaces\"\n\necho 第二条\n');await f.save();assert.ok(f.confirmation());await f.confirmation().run();assert.deepEqual(f.commands[0].application.detached,['echo  \"internal spaces\"','echo 第二条']);
});
test('large detached command groups are accepted beyond the old textarea limit but the full task envelope is bounded',async()=>{
 const f=fixture();f.button('Edit').props.onClick();f.editor().props.setDetachedText(Array.from({length:3},(_,i)=>String(i)+'x'.repeat(4095)).join('\n'));await f.save();assert.ok(f.confirmation());await f.confirmation().run();assert.equal(f.commands[0].application.detached.length,3);
 const g=fixture();g.button('Edit').props.onClick();g.editor().props.setDetachedText(Array.from({length:16},()=> 'x'.repeat(4096)).join('\n'));await g.save();assert.equal(g.commands.length,0);assert.equal(g.confirmation(),undefined);assert.match(g.editor().props.failure,/task size limit/);
});
test('unapproved host commands show a permission error and cannot reach confirmation',async()=>{
 const f=fixture();f.host.props.device.capabilities.application_host_commands_allowed=false;f.button('Edit').props.onClick();f.editor().props.setApplication({...spec,cmd:'launch'});await f.save();assert.equal(f.commands.length,0);assert.equal(f.confirmation(),undefined);assert.match(f.editor().props.failure,/not allowed application host commands/);
});
