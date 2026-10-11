import assert from 'node:assert/strict';
import {test} from 'node:test';
import {originalModule} from './fixtures/original-module.mjs';
import {HookHost,walk,textContent} from './fixtures/hook-host.mjs';
const {SunshineControls}=await originalModule(new URL('../src/SunshineControls.tsx',import.meta.url),{
 '@xcss/web/admin-ui':'export const Button="button",FormField="field",TextField="input";',
 '@xcss/web/admin-ui/i18n':'export function t(zh,en){return en}',
 './ConfigSelect':'export const ConfigSelect="select"',
 './api':'export function supportsPendingPairingListing(){return true}'
});
const spec={name:'Existing game',output:'',cmd:'','working-dir':'','exclude-global-prep-cmd':false,elevated:false,'auto-detach':true,'wait-all':true,'exit-timeout':5,'prep-cmd':[],detached:[],'image-path':''};
const snapshot={revision:'a'.repeat(64),applications:[{reference:{fingerprint:'b'.repeat(64)},specification:spec}]};
const old={operation_id:'old',action:'sunshine.applications.list',state:'succeeded',created_at_micros:1,updated_at_micros:2,result:{kind:'applications_read',snapshot},reconciliation:null};
function fixture(operations=[old]){
 const commands=[];
 const host=new HookHost(SunshineControls,{device:{id:'fixture',capabilities:{protocol:'xscs-management/1',application_management:true}},operations,busy:false,blocked:false,section:'applications',submit:async command=>{commands.push(command);return true},confirm(){}});
 const tree=()=>walk(host.render(),node=>node.type==='section'&&node.props.id==='sunshine-applications')[0];
 const alerts=()=>walk(tree(),node=>node.props.role==='alert');
 return {host,commands,tree,alerts};
}
const attempt=state=>({...old,operation_id:'refresh',created_at_micros:10,updated_at_micros:11,state,result:state==='failed'||state==='dead_letter'?{kind:'rejected',reason:'sunshine_unavailable'}:null});
for(const state of ['failed','dead_letter','unknown'])test(`application refresh ${state} retains old data and exposes its failure`,()=>{
 const f=fixture([old,attempt(state)]);
 assert.match(textContent(f.tree()),/Existing game/);
 assert.equal(f.alerts().length,1);
 assert.match(textContent(f.alerts()),/Could not refresh applications/);
});
test('first failed application read shows failure instead of silently returning the initial refresh instruction',()=>{
 const f=fixture([attempt('failed')]);
 assert.match(textContent(f.alerts()),/Could not refresh applications/);
});
test('application refresh pending and successful completion are distinguished without overwriting an open draft',()=>{
 const f=fixture();
 walk(f.tree(),node=>node.type==='button'&&textContent(node)==='Edit')[0].props.onClick();
 const editor=()=>walk(f.tree(),node=>typeof node.type==='function'&&node.type.name==='ApplicationEditor')[0];
 editor().props.setApplication({...spec,name:'Unsaved draft'});
 f.host.props.operations=[attempt('running'),old];
 assert.match(textContent(f.tree()),/Waiting for applications/);
 assert.equal(editor().props.application.name,'Unsaved draft');
 f.host.props.operations=[{...attempt('succeeded'),result:{kind:'applications_read',snapshot:{...snapshot,applications:[]}}},old];
 assert.doesNotMatch(textContent(f.tree()),/Waiting for applications|Could not refresh applications/);
 assert.equal(editor().props.application.name,'Unsaved draft');
});
test('newer read supersedes older failures by creation order and unrelated failures stay out of the applications section',()=>{
 const f=fixture([{...attempt('failed'),updated_at_micros:100},{...old,operation_id:'new',created_at_micros:20,updated_at_micros:21},{...attempt('failed'),action:'sunshine.diagnostics.read',created_at_micros:30}]);
 assert.equal(f.alerts().length,0);
 assert.match(textContent(f.tree()),/Existing game/);
});
test('refresh button still dispatches exactly one ordinary list request',async()=>{
 const f=fixture([old,attempt('failed')]);
 const button=walk(f.tree(),node=>node.type==='button'&&textContent(node)==='Refresh applications')[0];
 assert.equal(button.props.disabled,false);button.props.onClick();await Promise.resolve();
 assert.deepEqual(f.commands,[{kind:'list_applications'}]);
});
for(const [action,kind] of [['save','application_saved'],['delete','application_deleted']])test(`a newer successful ${action} snapshot clears the older list failure`,()=>{
 const f=fixture([old,attempt('failed'),{...old,operation_id:'mutation',action:`sunshine.applications.${action}`,created_at_micros:20,updated_at_micros:21,result:{kind,snapshot}}]);
 assert.equal(f.alerts().length,0);
 assert.match(textContent(f.tree()),/Existing game/);
});
for(const action of ['save','delete'])for(const state of ['pending','running','failed','unknown'])test(`a newer ${state} ${action} cannot clear an earlier list failure without a confirmed snapshot`,()=>{
 const f=fixture([old,attempt('failed'),{...old,operation_id:'mutation',action:`sunshine.applications.${action}`,state,created_at_micros:20,updated_at_micros:21,result:null}]);
 assert.equal(f.alerts().length,1);
});
for(const resolution of ['confirmed_failed','unable_to_confirm','confirmed_succeeded'])test(`resolved read ${resolution} without read evidence still requires refresh`,()=>{
 const f=fixture([old,{...attempt('unknown'),state:'resolved',resolution}]);
 assert.equal(f.alerts().length,1);
});
test('confirmed successful read reconciliation clears failure with its actual snapshot',()=>{
 const f=fixture([old,{...attempt('unknown'),state:'resolved',resolution:'confirmed_succeeded',reconciliation:{kind:'applications_read',snapshot},reconciliation_observed_at_micros:12}]);
 assert.equal(f.alerts().length,0);
});
test('confirmed successful save reconciliation supersedes the older list failure',()=>{
 const f=fixture([old,attempt('failed'),{...old,operation_id:'mutation',action:'sunshine.applications.save',state:'resolved',resolution:'confirmed_succeeded',created_at_micros:20,updated_at_micros:21,result:null,reconciliation:{kind:'application_saved',snapshot},reconciliation_observed_at_micros:21}]);
 assert.equal(f.alerts().length,0);
});
for(const timestamp of [undefined,null,Number.MAX_SAFE_INTEGER+1])test(`save reconciliation with untrusted evidence time ${timestamp} cannot clear list failure`,()=>{
 const f=fixture([old,attempt('failed'),{...old,operation_id:'mutation',action:'sunshine.applications.save',state:'resolved',resolution:'confirmed_succeeded',created_at_micros:20,updated_at_micros:21,result:null,reconciliation:{kind:'application_saved',snapshot},reconciliation_observed_at_micros:timestamp}]);
 assert.equal(f.alerts().length,1);
});
test('read reconciliation without evidence time cannot clear the need to refresh',()=>{
 const f=fixture([old,{...attempt('unknown'),state:'resolved',resolution:'confirmed_succeeded',reconciliation:{kind:'applications_read',snapshot}}]);
 assert.equal(f.alerts().length,1);
});
