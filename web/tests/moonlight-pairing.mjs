import assert from "node:assert/strict";
import {randomUUID} from "node:crypto";
import {chromium,firefox,expect} from "@playwright/test";
import {preview} from "vite";

const server=await preview({preview:{host:"127.0.0.1",port:0,strictPort:true}});
const session={authenticated:true,user_id:"A".repeat(43),username:"admin",role:"admin",csrf_token:"A".repeat(43)};
const engines=process.env.MOONLIGHT_FIREFOX_REPETITIONS?Array.from({length:Number(process.env.MOONLIGHT_FIREFOX_REPETITIONS)},()=>firefox):[chromium,firefox];
try {for(const engine of engines){
 const browser=await engine.launch();
 try {
  const page=await browser.newPage({locale:"zh-CN"});const commands=[];const operations=[];const errors=[];let empty=false,clock=Date.now()*1000,holdFollowupPending=true,releasePending=null,holdPairedRefresh=true,releasePaired=null,failPairedRefresh=false;
  await page.addInitScript(()=>{
   window.__operationNotices=[];const seen=new WeakSet();
   new MutationObserver(()=>{for(const toast of document.querySelectorAll(".xcss-toast")){if(seen.has(toast))continue;seen.add(toast);window.__operationNotices.push(toast.querySelector("span")?.textContent)}}).observe(document,{childList:true,subtree:true});
  });
  const notices=()=>page.evaluate(()=>window.__operationNotices);
  page.on("pageerror",error=>errors.push(error.message));
  const requests=[{id:"a".repeat(32),name:"Moonlight TV",address:"192.168.1.20"},{id:"b".repeat(32),name:"Moonlight 手机",address:"192.168.1.21"}];
  const snapshot={revision:"c".repeat(64),sunshine_version:"2026.914.233613",fields:{sunshine_name:"Original"},effectiveness:"pending_verification"};
  const device={id:randomUUID(),name:"配对测试主机",registered:true,pairing_pending:false,revoked:false,client_online:true,sunshine_reachable:true,configuration_state:"pending_verification",last_seen_at_micros:clock,snapshot,capabilities:{protocol:"xscs-management/1",client_version:"0.1.0",os:"linux_x86_64",sunshine_version:"2026.914.233613",restart_allowed:false,managed_fields:["sunshine_name"],configuration_overwrite:true,application_management:false,application_host_commands_allowed:false,pending_pairing_listing:true,moonlight_pairing_management:true,diagnostics:false,maintenance:false,service_control:false}};
  await page.route("**/api/v1/**",async route=>{
   const req=route.request(),url=new URL(req.url()),path=url.pathname;
   if(path.endsWith("/sunshine/config-fields"))return route.fulfill({json:[{key:"sunshine_name",kind:"text",maximum_length:32,requires_restart:true,supported_sunshine_versions:["2026.914.233613"],operating_systems:["linux_x86_64"]}]});
   if(path.endsWith("/sunshine/devices"))return route.fulfill({json:[device]});
   if(path.endsWith("/authorization"))return route.fulfill({json:{manager_id:randomUUID(),device_id:device.id,authorization_code:"d".repeat(36)}});
   if(path.endsWith("/tasks/calendar"))return route.fulfill({json:{today:new Date().toISOString().slice(0,10)}});
   if(path.endsWith("/tasks/summary"))return route.fulfill({json:{blocking_count:operations.filter(operation=>operation.state==="unknown").length}});
   if(path.endsWith("/tasks")&&req.method()==="POST"){
    const command=req.postDataJSON();commands.push(command);assert.equal(req.headers()["x-csrf-token"],session.csrf_token);assert.ok(req.headers()["idempotency-key"]);
    let result,state="succeeded",action;
    if(command.kind==="list_pending_pairings"){if(holdFollowupPending&&commands.some(value=>value.kind==="submit_pairing_pin")){await new Promise(resolve=>{releasePending=resolve});holdFollowupPending=false}action="sunshine.pairing.pending.list";result={kind:"pending_pairings_read",pairings:empty?[]:requests}}
    else if(command.kind==="list_paired_clients"){if(failPairedRefresh){failPairedRefresh=false;return route.fulfill({status:503,json:{code:"database_unavailable",message:"try again",retryable:true}})}if(holdPairedRefresh){await new Promise(resolve=>{releasePaired=resolve});holdPairedRefresh=false}action="sunshine.pairing.clients.list";result={kind:"paired_clients_read",snapshot:{revision:"e".repeat(64),clients:[{uuid:randomUUID(),name:"客厅 TV",enabled:true}]}}}
    else if(command.kind==="submit_pairing_pin"){
     action="sunshine.pairing.pin.submit";
     if(command.pin==="1234"){result={kind:"pairing_pin_submitted"};requests.splice(requests.findIndex(pairing=>pairing.id===command.pairing_id),1)}
     else if(command.pin==="5678"){state="failed";result={kind:"rejected",reason:"pairing_failed"}}
     else {state="unknown";result={kind:"unknown",reason:"side_effect_not_confirmed"}}
    } else {assert.equal(command.kind,"read_config");action="sunshine.config.read";result={kind:"config_read",snapshot}}
    const created=++clock;const operation={operation_id:"op_"+randomUUID(),device_id:device.id,action,state,attempt:1,created_at_micros:created,created_at_server:new Date(Math.floor(created/1000)).toISOString().slice(0,19).replace("T"," ")+".000000 UTC",updated_at_micros:created,result,reconciliation:null,resolution:null};operations.unshift(operation);return route.fulfill({json:operation});
   }
   if(path.endsWith("/tasks"))return route.fulfill({json:new URL(req.url()).searchParams.has("date")?{operations:operations.slice(0,50),next_cursor:null,previous_cursor:null}:operations.slice(0,50)});
   return route.fulfill({json:session});
  });
  await page.goto("http://127.0.0.1:"+server.httpServer.address().port);
  await page.evaluate(()=>{window.__pairingEvents=[];for(const kind of ["pointerdown","pointerup","click","submit","invalid"])document.addEventListener(kind,event=>{if(!event.target.closest?.("#sunshine-moonlight-pairing"))return;window.__pairingEvents.push({kind,target:event.target.tagName,name:event.target.name,text:event.target.tagName==="BUTTON"?event.target.textContent:undefined,disabled:event.target.disabled,time:performance.now()});if(window.__pairingEvents.length>30)window.__pairingEvents.shift()},true)});
  await page.getByRole("link",{name:"选择实例 配对测试主机",exact:true}).click();
  const categories=page.getByRole("navigation",{name:"Sunshine 配置分类"});
  assert.equal(commands.filter(command=>command.kind==="list_pending_pairings").length,0);
  await categories.getByRole("button",{name:"Moonlight 配对",exact:true}).click();
  const panel=page.getByRole("region",{name:"Moonlight 配对",exact:true});
  await expect.poll(()=>commands.filter(command=>command.kind==="list_pending_pairings").length).toBe(1);
  await expect(panel.getByRole("combobox",{name:"待配对客户端",exact:true})).toContainText("Moonlight TV · 192.168.1.20");
  assert.deepEqual(await notices(),[],"opening Moonlight must read requests without announcing an operation");
  await panel.getByRole("button",{name:"刷新配对请求",exact:true}).click();
  await expect.poll(()=>commands.filter(command=>command.kind==="list_pending_pairings").length).toBe(2);
  await expect(panel.getByRole("button",{name:"刷新配对请求",exact:true})).toBeEnabled();
  assert.deepEqual(await notices(),[],"manual list reads must also remain silent on success");
  await expect(panel.locator('input[name="pairing_id"]')).toHaveCount(0);
  await expect(panel.getByLabel("客户端名称",{exact:true})).toHaveValue("Moonlight TV");
  await panel.getByRole("combobox",{name:"待配对客户端",exact:true}).click();
  await panel.getByRole("option",{name:"Moonlight 手机 · 192.168.1.21"}).click();
  await expect(panel.getByLabel("客户端名称",{exact:true})).toHaveValue("Moonlight 手机");
  await panel.getByRole("combobox",{name:"待配对客户端",exact:true}).click();
  await panel.getByRole("option",{name:"Moonlight TV · 192.168.1.20"}).click();
  await panel.getByLabel("客户端名称",{exact:true}).fill("客厅 TV");
  await panel.getByLabel("PIN（4 位数字）",{exact:true}).fill("1234");
  await panel.getByRole("button",{name:"配对",exact:true}).click();
  await expect(panel.getByRole("status")).toContainText("Moonlight 已配对");
  await expect.poll(async()=>(await notices()).length).toBe(1);
  assert.match((await notices())[0],/操作已写入日志/);
  assert.deepEqual(commands.find(command=>command.kind==="submit_pairing_pin"),{kind:"submit_pairing_pin",pairing_id:"a".repeat(32),pin:"1234",name:"客厅 TV"});
  await expect.poll(()=>typeof releasePending).toBe("function");
  await expect(panel.getByLabel("PIN（4 位数字）",{exact:true})).toBeDisabled();
  await expect(panel.getByRole("button",{name:"刷新配对请求",exact:true})).toBeDisabled();
  releasePending();
  await expect.poll(()=>typeof releasePaired).toBe("function");
  await expect(panel.getByLabel("PIN（4 位数字）",{exact:true})).toBeDisabled();
  await expect(panel.getByRole("button",{name:"配对",exact:true})).toBeDisabled();
  assert.equal(commands.filter(command=>command.kind==="submit_pairing_pin").length,1);
  releasePaired();
  await expect(panel.getByRole("combobox",{name:"待配对客户端",exact:true})).toContainText("Moonlight 手机");
  await expect(panel).toContainText("客厅 TV");
  await expect(panel.getByLabel("PIN（4 位数字）",{exact:true})).toHaveValue("");
  assert.equal((await notices()).length,1,"automatic follow-up reads must not duplicate the explicit PIN operation notification");
  const pairedReads=commands.filter(command=>command.kind==="list_paired_clients").length;
  await panel.getByRole("button",{name:"刷新已配对客户端",exact:true}).click();
  await expect.poll(()=>commands.filter(command=>command.kind==="list_paired_clients").length).toBe(pairedReads+1);
  await expect(panel.getByRole("button",{name:"刷新已配对客户端",exact:true})).toBeEnabled();
  assert.equal((await notices()).length,1,"a paired-client refresh must not announce a mutation");
  await panel.getByLabel("PIN（4 位数字）",{exact:true}).fill("5678");
  await panel.getByRole("button",{name:"配对",exact:true}).click();
  try {await expect(panel.getByRole("alert")).toContainText("配对未完成")} catch(error){console.error("Moonlight command trace",JSON.stringify(commands));console.error("Pairing event trace",await page.evaluate(()=>window.__pairingEvents));console.error("Pairing form validity",await panel.locator("form").evaluate(form=>Array.from(form.elements).map(element=>({name:element.name,value:element.value,valid:element.validity?.valid,message:element.validationMessage}))));throw error}
  await panel.getByLabel("PIN（4 位数字）",{exact:true}).fill("9999");
  await panel.getByRole("button",{name:"配对",exact:true}).click();
  await expect(panel.getByRole("alert")).toContainText("配对结果未确认");
  await expect(panel.getByRole("button",{name:"配对",exact:true})).toBeDisabled();
  empty=true;await panel.getByRole("button",{name:"刷新配对请求",exact:true}).click();
  await expect(panel).toContainText("暂无配对请求，请先在 Moonlight 中发起配对。");
  assert.equal(commands.filter(command=>command.kind==="submit_pairing_pin").length,3);
  // A failed automatic list refresh must not undo an already confirmed pairing.
  const unknown=operations.find(operation=>operation.state==="unknown");unknown.state="resolved";unknown.resolution="unable_to_confirm";
  empty=false;await page.getByRole("button",{name:"刷新",exact:true}).click();
  await panel.getByRole("button",{name:"刷新配对请求",exact:true}).click();
  await expect(panel.getByRole("combobox",{name:"待配对客户端",exact:true})).toContainText("Moonlight 手机");
  await panel.getByLabel("PIN（4 位数字）",{exact:true}).fill("1234");
  failPairedRefresh=true;await panel.getByRole("button",{name:"配对",exact:true}).click();
  await expect(panel.getByRole("status")).toContainText("Moonlight 已配对");
  await expect(panel.getByText("列表刷新未完成，请手动刷新；已确认的配对结果保持不变。",{exact:true})).toBeVisible();
  await expect(page.getByText("读取暂时不可用，请稍后重试。",{exact:true})).toBeVisible();
  await expect(panel.getByRole("button",{name:"刷新已配对客户端",exact:true})).toBeEnabled();
  assert.equal(commands.filter(command=>command.kind==="submit_pairing_pin").length,4);
  assert.equal((await notices()).length,4,"each explicit PIN submission should notify once, with no notifications for reads or read failures");
  device.capabilities.pending_pairing_listing=false;
  await page.getByRole("button",{name:"刷新",exact:true}).click();
  await expect(panel).toContainText("客户端尚未报告待配对请求查询能力。");
  await expect(panel.getByRole("button",{name:"配对",exact:true})).toHaveCount(0);
  assert.equal(commands.filter(command=>command.kind==="submit_pairing_pin").length,4);
  assert.deepEqual(errors,[]);
  console.log(`PASS ${engine.name()} Moonlight discovery, selected request, PIN handshake feedback and no automatic replay`);
 } finally {await browser.close()}
}} finally {await new Promise(resolve=>server.httpServer.close(resolve))}
