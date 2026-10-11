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
const page={revision:'a'.repeat(64),text:'Previous log page',start_offset:100,end_offset:117,total_bytes:117,previous:{revision:'a'.repeat(64),before_offset:100},redacted:false};
const old={operation_id:'old',action:'sunshine.logs.read',state:'succeeded',created_at_micros:1,updated_at_micros:2,result:{kind:'logs_read',page},reconciliation:null};
const attempt=state=>({...old,operation_id:'refresh',created_at_micros:10,updated_at_micros:11,state,result:state==='failed'?{kind:'rejected',reason:'resource_conflict'}:null});
function fixture(operations=[old]){
 const commands=[];
 const host=new HookHost(SunshineControls,{device:{id:'fixture',capabilities:{protocol:'xscs-management/1',diagnostics:true}},operations,busy:false,blocked:false,section:'diagnostics',submit:async command=>{commands.push(command);return true},confirm(){}});
 const tree=()=>host.render();const alerts=()=>walk(tree(),node=>node.props.role==='alert');
 return {host,commands,tree,alerts};
}
for(const state of ['failed','dead_letter','unknown'])test(`log read ${state} retains prior page but explicitly requires a fresh read`,()=>{
 const f=fixture([old,attempt(state)]);
 assert.match(textContent(f.tree()),/Previous log page/);
 assert.match(textContent(f.alerts()),/Could not read Sunshine logs/);
 assert.match(textContent(f.alerts()),/Read latest logs/);
});
test('first failed log read exposes failure even without an old page',()=>{
 const f=fixture([attempt('failed')]);assert.match(textContent(f.alerts()),/Could not read Sunshine logs/);
});
for(const state of ['pending','running'])test(`log read ${state} distinguishes retained evidence from a new result`,()=>{
 const f=fixture([old,attempt(state)]);assert.match(textContent(f.tree()),/Waiting for Sunshine logs/);assert.match(textContent(f.tree()),/Previous log page/);
});
for(const resolution of ['confirmed_failed','unable_to_confirm','confirmed_succeeded'])test(`resolved log read ${resolution} with no evidence still needs a new read`,()=>{
 const f=fixture([old,{...attempt('resolved'),resolution}]);assert.match(textContent(f.alerts()),/Could not read Sunshine logs/);
});
test('newer successful empty page clears feedback and disables previous-page navigation',()=>{
 const f=fixture([old,attempt('failed')]);assert.equal(f.alerts().length,1);
 f.host.props.operations=[{...attempt('failed'),updated_at_micros:100},{...old,operation_id:'new',created_at_micros:20,updated_at_micros:21,result:{kind:'logs_read',page:{...page,text:'',start_offset:0,end_offset:0,total_bytes:0,previous:null}}}];
 assert.equal(f.alerts().length,0);assert.match(textContent(f.tree()),/This page has no log content/);
 assert.equal(walk(f.tree(),node=>node.type==='button'&&textContent(node)==='Read previous page')[0].props.disabled,true);
});
test('unrelated failed reads do not label successful logs as failed and cursor commands remain unchanged',async()=>{
 const f=fixture([old,{...attempt('failed'),action:'sunshine.diagnostics.read'}]);
 assert.doesNotMatch(textContent(f.alerts()),/Could not read Sunshine logs/);
 for(const name of ['Read previous page','Read latest logs'])walk(f.tree(),node=>node.type==='button'&&textContent(node)===name)[0].props.onClick();
 await Promise.resolve();assert.deepEqual(f.commands,[{kind:'read_logs',cursor:page.previous,limit_bytes:16384},{kind:'read_logs',cursor:null,limit_bytes:16384}]);
});
test('valid log reconciliation clears failure and shows its receipt',()=>{
 const f=fixture([old,{...attempt('resolved'),resolution:'confirmed_succeeded',reconciliation:{kind:'logs_read',page:{...page,text:'Reconciled page'}},reconciliation_observed_at_micros:12}]);
 assert.equal(f.alerts().length,0);assert.match(textContent(f.tree()),/Reconciled page/);
});
const virtualReport={kind:'virtual_input_status_read',status:{virtualhid:{installed:true},vigembus:{installed:false}}};
for(const state of ['failed','dead_letter','unknown','pending','running'])test(`virtual input ${state} independently marks its prior result`,()=>{
 const previous={...old,action:'sunshine.virtual_input.read',result:virtualReport};
 const f=fixture([previous,{...attempt(state),action:'sunshine.virtual_input.read'}]);
 assert.match(textContent(f.tree()),state==='pending'||state==='running'?/Waiting for virtual input status/:/Could not refresh virtual input status/);
 assert.match(textContent(f.tree()),/VirtualHID :  installed/);
 assert.doesNotMatch(textContent(f.tree()),/Could not read Sunshine logs|Waiting for Sunshine logs|Could not refresh diagnostics/);
});
test('first virtual-input failure is visible and a newer successful receipt clears it',()=>{
 const f=fixture([{...attempt('failed'),action:'sunshine.virtual_input.read'}]);
 assert.match(textContent(f.alerts()),/Could not refresh virtual input status/);
 f.host.props.operations.push({...old,operation_id:'new',action:'sunshine.virtual_input.read',created_at_micros:20,updated_at_micros:21,result:virtualReport});
 assert.equal(f.alerts().length,0);assert.match(textContent(f.tree()),/VirtualHID :  installed/);
});
for(const kind of ['logs_read','virtual_input_status_read'])test(`${kind} reconciliation without evidence time cannot clear feedback`,()=>{
 const action=kind==='logs_read'?'sunshine.logs.read':'sunshine.virtual_input.read';
 const f=fixture([{...attempt('resolved'),action,resolution:'confirmed_succeeded',reconciliation:kind==='logs_read'?{kind,page}:virtualReport}]);
 assert.equal(f.alerts().length,1);
});
test('resolved virtual input without a successful status remains stale until its valid reconciliation',()=>{
 const resolved={...attempt('resolved'),action:'sunshine.virtual_input.read',resolution:'confirmed_succeeded'};
 const f=fixture([resolved]);assert.equal(f.alerts().length,1);
 f.host.props.operations=[{...resolved,reconciliation:virtualReport,reconciliation_observed_at_micros:12}];
 assert.equal(f.alerts().length,0);assert.match(textContent(f.tree()),/VirtualHID :  installed/);
});
