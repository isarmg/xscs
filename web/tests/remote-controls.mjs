import { rangeEditor, checkDateRangeValidation } from "./date-range.mjs";
import assert from "node:assert/strict";
import {chromium,firefox,expect} from "@playwright/test";
import {preview} from "vite";
import AxeBuilder from "@axe-core/playwright";
import {randomUUID} from "node:crypto";
import {createAdministratorApiClient} from "@xcss/web/admin-web";
const largeJson=JSON.stringify({payload:"x".repeat(2_200_000)});
const budgetClient=createAdministratorApiClient({baseUrl:"http://localhost:43210",fetchImpl:async()=>new Response(largeJson,{headers:{"content-type":"application/json","content-length":String(Buffer.byteLength(largeJson))}})});
const isLargePayload=value=>value!==null&&typeof value==="object"&&typeof value.payload==="string";
await assert.rejects(()=>budgetClient.request("/api/v1/log-budget",isLargePayload),error=>error.code==="response_too_large");
assert.equal((await budgetClient.request("/api/v1/log-budget",isLargePayload,{maxResponseBytes:64*1024*1024})).payload.length,2_200_000);
const session={authenticated:true,user_id:"A".repeat(43),username:"admin",role:"admin",csrf_token:"A".repeat(43)};
const configFields=[
 {key:"sunshine_name",kind:"text",maximum_length:32,requires_restart:true,supported_sunshine_versions:["2026.914.233613"],operating_systems:["linux_x86_64"]},
 {key:"qp",kind:"integer",minimum:0,maximum:51,requires_restart:true,supported_sunshine_versions:["2026.914.233613"],operating_systems:["linux_x86_64"]},
];
configFields.push({key:"nvenc_preset",kind:"integer",minimum:1,maximum:7,requires_restart:true,supported_sunshine_versions:["2026.914.233613"],operating_systems:["linux_x86_64"]});
for(const key of ["controller","keyboard"])configFields.push({key,kind:"boolean",requires_restart:true,supported_sunshine_versions:["2026.914.233613"],operating_systems:["linux_x86_64"]});
const server=await preview({preview:{host:"127.0.0.1",port:0,strictPort:true}});
try {for(const engine of process.argv.includes("--browser=firefox") ? [firefox] : [chromium,firefox]){
 const browser=await engine.launch();
 try {
  const context=await browser.newContext({ locale: "zh-CN", hasTouch:true });const page=await context.newPage();const commands=[];const configReads=[];const errors=[];const operations=[];const logDates=[];const serverToday="2030-01-02",serverYesterday="2030-01-01",serverStart=Date.parse("2030-01-02T00:00:00Z")*1000,serverEnd=Date.parse("2030-01-03T00:00:00Z")*1000;let operationClock=0;let emptyApplications=false,holdInitialSummary=true,releaseInitialSummary=null,holdNextCalendar=false,releaseCalendar=null,failCalendarOnce=false,largeLogPayloadOnce=false,failNextLog=false,externalBlocking=0,fullLogRequests=0,summaryResponses=0,failSummaryOnce=false,failedSummaryResponses=0;
  const serverTimestamp=micros=>new Date(Math.floor(micros/1000)).toISOString().slice(0,19).replace("T"," ")+"."+String(micros%1_000_000).padStart(6,"0")+" UTC";
  page.on("pageerror",e=>errors.push(e.message));
  const app={reference:{fingerprint:"c".repeat(64)},specification:{name:"Steam",output:"",cmd:"","working-dir":"","exclude-global-prep-cmd":false,elevated:false,"auto-detach":false,"wait-all":false,"exit-timeout":5,"prep-cmd":[],detached:[],"image-path":""}};
  const device={id:randomUUID(),name:"游戏主机",registered:true,pairing_pending:false,revoked:false,client_online:true,sunshine_reachable:true,configuration_state:"pending_verification",last_seen_at_micros:Date.now()*1000,capabilities:{protocol:"xscs-management/1",client_version:"0.3.1",os:"linux_x86_64",sunshine_version:"2026.914.233613",restart_allowed:true,managed_fields:configFields.map(field=>field.key),configuration_overwrite:true,application_management:true,application_host_commands_allowed:true,pending_pairing_listing:true,moonlight_pairing_management:true,diagnostics:true,maintenance:true,service_control:true},snapshot:{revision:"a".repeat(64),sunshine_version:"2026.914.233613",fields:{sunshine_name:"Original",qp:"28",nvenc_preset:"1",controller:"true",keyboard:"false"},effectiveness:"pending_verification"}};
  await page.route("**/api/v1/**",async route=>{
   const req=route.request();const path=new URL(req.url()).pathname;
   if(path.endsWith("/sunshine/config-fields"))return route.fulfill({json:configFields});
   if(path.endsWith("/sunshine/devices"))return route.fulfill({json:[device]});
   if(path.endsWith("/authorization"))return route.fulfill({json:{manager_id:randomUUID(),device_id:device.id,authorization_code:"b".repeat(36)}});
   if(path.endsWith("/tasks/calendar")){
    if(holdNextCalendar){await new Promise(resolve=>{releaseCalendar=resolve});holdNextCalendar=false}
    if(failCalendarOnce){failCalendarOnce=false;return route.fulfill({status:503,json:{code:"database_unavailable",message:"try again",retryable:true}})}
    return route.fulfill({json:{today:serverToday}});
   }
   if(path.endsWith("/tasks/summary")){
    if(holdInitialSummary)await new Promise(resolve=>{releaseInitialSummary=resolve});
    if(failSummaryOnce){failSummaryOnce=false;failedSummaryResponses++;return route.fulfill({status:503,json:{code:"database_unavailable",message:"try again",retryable:true}})}
    summaryResponses++;
    return route.fulfill({json:{blocking_count:externalBlocking+operations.filter(operation=>["pending","running","unknown"].includes(operation.state)).length}});
   }
   if(path.endsWith("/tasks")){
    if(req.method()==="POST"){
     const command=req.postDataJSON();if(command.kind==="read_config")configReads.push(command);else if(command.kind!=="list_pending_pairings")commands.push(command);
     assert.equal(req.headers()["x-csrf-token"],session.csrf_token);assert.ok(req.headers()["idempotency-key"]);
     const action={read_config:"sunshine.config.read",patch_config:"sunshine.config.patch",save_config:"sunshine.config.save",list_pending_pairings:"sunshine.pairing.pending.list",restart:"sunshine.restart",list_applications:"sunshine.applications.list",save_application:"sunshine.applications.save",list_paired_clients:"sunshine.pairing.clients.list",read_logs:"sunshine.logs.read",read_diagnostics:"sunshine.diagnostics.read",read_virtual_input_status:"sunshine.virtual_input.read",read_service_status:"sunshine.service.read"}[command.kind]??`sunshine.${command.kind}`;
     const snapshot={...device.snapshot,effectiveness:["patch_config","save_config"].includes(command.kind)?"awaiting_restart":"pending_verification"};
     const result=command.kind==="upload_cover"?{kind:"cover_uploaded",path:"covers/test.png"}:command.kind==="control_service"?{kind:"service_controlled",action:command.action,state:"stopped"}:command.kind==="restart"?{kind:"restart_acknowledged",snapshot,process_generation:"fixture-process-generation"}:command.kind==="read_config"?{kind:"config_read",snapshot}:["patch_config","save_config"].includes(command.kind)?{kind:"config_saved",snapshot}:command.kind==="list_pending_pairings"?{kind:"pending_pairings_read",pairings:[]}:command.kind==="list_applications"?{kind:"applications_read",snapshot:{revision:"d".repeat(64),applications:emptyApplications?[]:[app]}}:command.kind==="save_application"?{kind:"application_saved",snapshot:{revision:"e".repeat(64),applications:[{reference:{fingerprint:"f".repeat(64)},specification:command.application}]}}:command.kind==="list_paired_clients"?{kind:"paired_clients_read",snapshot:{revision:"1".repeat(64),clients:[{uuid:"123e4567-e89b-12d3-a456-426614174000",name:"Moonlight TV",enabled:true}]}}:command.kind==="read_logs"?{kind:"logs_read",page:{revision:"2".repeat(64),text:"Sunshine ready\n",start_offset:0,end_offset:15,total_bytes:15,previous:null,redacted:false}}:command.kind==="read_diagnostics"?{kind:"diagnostics_read",snapshot:{sunshine_version:"2026.914.233613",platform:"linux",api_reachable:true,authentication_accepted:true,service_state:"running",configuration_revision:"a".repeat(64)}}:command.kind==="read_virtual_input_status"?{kind:"virtual_input_status_read",status:{virtualhid:{installed:false,version:null,required_version:null,error:null},vigembus:{installed:false,version:null,required_version:null,error:null}}}:command.kind==="read_service_status"?{kind:"service_status_read",state:"running"}:{kind:"maintenance_completed",action:command.action};
     const created=serverStart+1_000_000+operationClock++;
     const operation={operation_id:"op_"+randomUUID(),device_id:device.id,action,state:"succeeded",attempt:1,created_at_micros:created,created_at_server:serverTimestamp(created),updated_at_micros:created,result,reconciliation:null,resolution:null};
     operations.unshift(operation);return route.fulfill({json:operation});
    }
    const params=new URL(req.url()).searchParams;
    if(params.get("recent")==="50"){
     return route.fulfill({json:operations.slice(0,50)});
    }
    const date=params.get("date")??params.get("start_date");assert.ok(date,"dated logs require a server date");assert.ok([...params.keys()].every(key=>["date","start_date","end_date","cursor"].includes(key)));
    const from=Date.parse(date+"T00:00:00Z")*1000,to=Date.parse((params.get("end_date")??date)+"T00:00:00Z")*1000+86_400_000_000;
    fullLogRequests++;logDates.push(date);
    if(failNextLog){failNextLog=false;return route.fulfill({status:503,json:{code:"database_unavailable",message:"try again",retryable:true}})}
    let rows=operations.filter(operation=>operation.created_at_micros>=from&&operation.created_at_micros<to).sort((a,b)=>b.created_at_micros-a.created_at_micros||b.operation_id.localeCompare(a.operation_id));
    const cursor=params.get("cursor"),anchor=cursor?JSON.parse(Buffer.from(cursor,"base64url").toString("utf8")):null;
    if(anchor)rows=rows.filter(row=>anchor.newer?(row.created_at_micros>anchor.time||(row.created_at_micros===anchor.time&&row.operation_id>anchor.id)):(row.created_at_micros<anchor.time||(row.created_at_micros===anchor.time&&row.operation_id<anchor.id)));
    if(anchor?.newer)rows.reverse();
    const more=rows.length>50;rows=rows.slice(0,50);if(anchor?.newer)rows.reverse();
    const encode=(row,newer)=>Buffer.from(JSON.stringify({id:row.operation_id,time:row.created_at_micros,newer})).toString("base64url");
    const result={operations:rows,next_cursor:rows.length&&(!anchor?.newer?more:true)?encode(rows.at(-1),false):null,previous_cursor:rows.length&&(anchor?.newer?more:!!anchor)?encode(rows[0],true):null};
    if(largeLogPayloadOnce){largeLogPayloadOnce=false;const expanded=[{...rows[0],transport_padding:"x".repeat(2_200_000)},...rows.slice(1)];assert.ok(Buffer.byteLength(JSON.stringify(expanded))>2*1024*1024);return route.fulfill({json:{...result,operations:expanded}})}
    return route.fulfill({json:result});
   }
   if(path.endsWith("/resolve")&&req.method()==="POST"){
    const id=path.split("/").at(-2);const operation=operations.find(item=>item.operation_id===id);
    assert.ok(operation);operation.state="resolved";operation.resolution=req.postDataJSON().resolution;
    return route.fulfill({json:operation});
   }
   return route.fulfill({json:session});
  });
  await page.goto("http://127.0.0.1:"+server.httpServer.address().port);
  await page.getByRole("link",{name:"选择实例 游戏主机",exact:true}).click();
  await expect(page.getByRole("button",{name:"详细信息",exact:true})).toHaveAttribute("aria-pressed","true");
  const categories=page.getByRole("navigation",{name:"Sunshine 配置分类"});
  await expect(categories.getByRole("button",{name:"实例概览",exact:true})).toHaveAttribute("aria-pressed","true");
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  const saveConfig=page.getByRole("button",{name:"应用更改",exact:true});
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toHaveValue("Original");
  assert.equal(configReads.length,1);
  await expect(page.getByRole("button",{name:/读取配置|最新配置/})).toHaveCount(0);
  await expect.poll(()=>typeof releaseInitialSummary).toBe("function");
  await expect(saveConfig).toHaveCount(0);
  await expect(page.getByRole("button",{name:"保存配置",exact:true})).toHaveCount(0);
  await page.getByLabel("Sunshine 名称",{exact:true}).fill("Before task summary");
  await categories.getByRole("button",{name:"预览变更",exact:true}).click();
  await expect(saveConfig).toBeDisabled();
  await expect(page.getByText("正在确认该实例的任务状态，确认前暂停新操作。",{exact:true})).toBeVisible();
  holdInitialSummary=false;releaseInitialSummary();
  await expect(saveConfig).toBeEnabled();
  const saveConfigOffset=await saveConfig.evaluate(button=>{
   const text=document.createRange();text.selectNodeContents(button);
   return text.getBoundingClientRect().left-button.parentElement.getBoundingClientRect().left;
  });
  assert.ok(Math.abs(saveConfigOffset)<1,`Apply changes text is offset by ${saveConfigOffset}px`);
  await page.getByRole("region",{name:"变更差异预览"}).getByRole("button",{name:"撤销",exact:true}).click();
  await expect(saveConfig).toHaveCount(0);
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await expect(categories.getByRole("button",{name:"概况",exact:true})).toHaveAttribute("aria-pressed","true");
  assert.ok(await page.locator("[data-config-category]").count()>0);
  await expect(page.locator("[data-config-category] > h3")).toHaveCount(0);
  await expect(categories.getByRole("button",{name:"预览变更",exact:true})).toHaveAttribute("aria-controls","sunshine-config-preview");
  await expect(page.locator("form").getByRole("button",{name:"预览变更",exact:true})).toHaveCount(0);
  const managementCategories=[
   {name:"应用管理",id:"sunshine-applications"},
   {name:"诊断与 Sunshine 日志",id:"sunshine-diagnostics"},
   {name:"显示与输入维护",id:"sunshine-maintenance"},
  ];
  const passiveCommands=commands.length,passiveConfigReads=configReads.length;
  for(const name of await categories.getByRole("button").allTextContents()){
   await categories.getByRole("button",{name,exact:true}).click();
   await expect(page.getByRole("button",{name:"重启 Sunshine",exact:true})).toHaveCount(name==="概况"?1:0);
   await expect(page.getByRole("button",{name:"重启",exact:true})).toHaveCount(name==="概况"?1:0);
   for(const category of managementCategories){
    if(name===category.name)await expect(categories.getByRole("button",{name:category.name,exact:true})).toHaveAttribute("aria-controls",category.id);
    await expect(page.locator(`#${category.id}`).filter({visible:true})).toHaveCount(name===category.name?1:0);
    await expect(page.getByRole("heading",{name:category.name,exact:true})).toHaveCount(0);
   }
   await expect(page.getByRole("region",{name:"Sunshine 服务",exact:true})).toHaveCount(name==="概况"?1:0);
   const managementPage=name==="Moonlight 配对"||managementCategories.some(category=>category.name===name);
   await expect(page.locator(".sunshine-config-panel").filter({visible:true})).toHaveCount(managementPage?0:1);
   await expect(page.getByLabel("配置操作状态",{exact:true}).filter({visible:true})).toHaveCount(name==="预览变更"?1:0);
   await expect(page.getByRole("button",{name:"保存配置",exact:true})).toHaveCount(0);
   assert.equal(commands.length,passiveCommands,"changing a category must not submit a command");
   assert.equal(configReads.length,passiveConfigReads,"changing a category must preserve the initial configuration snapshot");
   await expect(page.getByText("操作已写入日志，等待客户端执行。",{exact:true})).toHaveCount(0);
  }
  await categories.getByRole("button",{name:"预览变更",exact:true}).click();
  await expect(page.getByRole("region",{name:"预览变更",exact:true})).toContainText("没有需要应用的更改");
  await categories.getByRole("button",{name:"NVIDIA NVENC 编码器",exact:true}).click();
  await expect(categories.getByRole("button",{name:"NVIDIA NVENC 编码器",exact:true})).toHaveAttribute("aria-pressed","true");
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await expect(page.getByLabel("量化参数 QP",{exact:true})).toBeHidden();
  await expect(categories.getByRole("button",{name:"NVIDIA NVENC 编码器",exact:true})).toBeVisible();
  await expect(categories.getByRole("button",{name:"Intel Quick Sync 编码器",exact:true})).toHaveCount(0);
  const configName=page.getByLabel("Sunshine 名称",{exact:true});
  assert.equal(await configName.getAttribute("maxlength"),null);
  for(const value of ["Living#Room","Leading ","😀".repeat(33)]){
   await configName.fill(value);await expect(configName).toHaveAttribute("aria-invalid","true");
   await categories.getByRole("button",{name:"预览变更",exact:true}).click();await saveConfig.click();
   await expect(configName).toBeFocused();await expect(configName).toHaveValue(value);assert.equal(commands.length,0);
  }
  await configName.fill("😀".repeat(32));await expect(configName).not.toHaveAttribute("aria-invalid","true");
  await page.getByLabel("Sunshine 名称", { exact: true }).fill("New name");
  for(const category of managementCategories){
   await categories.getByRole("button",{name:category.name,exact:true}).click();
   await expect(page.locator(`#${category.id}`)).toBeVisible();
   await expect(page.getByLabel("Sunshine 名称",{exact:true})).toBeHidden();
  }
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toHaveValue("New name");
  assert.equal(commands.length,passiveCommands,"visiting management categories must preserve a configuration draft without submitting it");
  assert.equal(configReads.length,passiveConfigReads);
  await categories.getByRole("button",{name:"高级",exact:true}).click();
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toBeHidden();
  await page.getByLabel("量化参数 QP",{exact:true}).fill("99");
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await page.getByRole("button",{name:"预览变更",exact:true}).click();
  await expect(categories.getByRole("button",{name:"预览变更",exact:true})).toHaveAttribute("aria-pressed","true");
  await expect(page.getByRole("region",{name:"变更差异预览"})).toContainText("99");
  assert.equal(commands.length,0);
  await saveConfig.click();
  await expect(categories.getByRole("button",{name:"高级",exact:true})).toHaveAttribute("aria-pressed","true");
  await expect(page.getByLabel("量化参数 QP",{exact:true})).toBeFocused();
  assert.equal(commands.length,0,"invalid configuration must not submit a save");
  await expect(page.getByRole("region",{name:"变更差异预览"})).toHaveCount(0);
  await page.getByLabel("量化参数 QP",{exact:true}).fill("24");
  await categories.getByRole("button",{name:"NVIDIA NVENC 编码器",exact:true}).click();
  await page.getByRole("region",{name:"NVIDIA NVENC 编码器",exact:true}).getByRole("checkbox").check();
  await categories.getByRole("button",{name:"输入",exact:true}).click();
  const inputRegion=page.getByRole("region",{name:"输入",exact:true});
  const controller=page.getByRole("combobox",{name:"控制器输入",exact:true});
  const controllerReset=inputRegion.getByRole("checkbox",{name:"控制器输入：恢复默认（删除显式设置）",exact:true});
  await expect(controller).toHaveText("启用");
  const rowCenters=await controller.locator("..").locator("..").evaluate(row=>[...row.children].map(element=>{const box=element.getBoundingClientRect();return box.top+box.height/2}));
  assert.ok(rowCenters.every(center=>Math.abs(center-rowCenters[0])<1),"label, control and reset should share a row");
  await controller.click();
  await expect(page.getByRole("listbox",{name:"控制器输入",exact:true})).toBeVisible();
  await expect(page.getByRole("option").locator("span:first-child")).toHaveText(["未显式设置","启用","禁用"]);
  await page.getByRole("option",{name:"禁用",exact:true}).click();
  await expect(controller).toHaveText("禁用");
  await expect(controller).toBeFocused();
  await controller.press("ArrowDown");
  await controller.press("Home");
  await controller.press("Escape");
  await expect(controller).toHaveText("禁用");
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await controller.press("Enter");
  await controller.press("Home");
  await controller.press("Enter");
  await expect(controller).toHaveText("未显式设置");
  await controller.press("ArrowDown");
  await controller.press("End");
  await controller.press("Enter");
  await expect(controller).toHaveText("禁用");
  await controllerReset.check();
  await expect(controller).toBeDisabled();
  await controllerReset.uncheck();
  await expect(controller).toBeEnabled();
  await expect(controller).toHaveText("禁用");
  for(const theme of ["dark","light"]){
   await page.getByRole("button",{name:theme==="dark"?"切换到深色模式":"切换到浅色模式",exact:true}).click();
   await controller.scrollIntoViewIfNeeded();
   await controller.click();
   assert.deepEqual((await new AxeBuilder({page}).include('section[aria-label="配置设置"]').withTags(["wcag2a","wcag2aa","wcag21aa"]).analyze()).violations,[]);
   if(process.env.SUNSHINE_SCREENSHOT_DIR)await page.getByRole("region",{name:"配置设置",exact:true}).screenshot({path:`${process.env.SUNSHINE_SCREENSHOT_DIR}/sunshine-config-${engine.name()}-${theme}.png`});
   await controller.press("Tab");
   await expect(controllerReset).toBeFocused();
   await expect(page.getByRole("listbox")).toHaveCount(0);
  }
  await page.setViewportSize({width:390,height:844});
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await controller.click();
  await page.getByRole("option",{name:"启用",exact:true}).tap();
  await expect(controller).toHaveText("启用");
  await controller.click();
  await page.getByRole("option",{name:"禁用",exact:true}).click();
  if(process.env.SUNSHINE_SCREENSHOT_DIR)await page.getByRole("region",{name:"配置设置",exact:true}).screenshot({path:`${process.env.SUNSHINE_SCREENSHOT_DIR}/sunshine-config-${engine.name()}-mobile.png`});
  const menuLayout=await categories.evaluate(menu=>({rows:new Set([...menu.children].map(element=>element.getBoundingClientRect().top)).size,fits:menu.scrollWidth<=menu.clientWidth+1}));
  assert.ok(menuLayout.rows>1&&menuLayout.fits,JSON.stringify(menuLayout));
  await categories.getByRole("button",{name:"概况",exact:true}).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toHaveValue("New name");
  assert.deepEqual((await new AxeBuilder({page}).include('section[aria-label="配置设置"]').withTags(["wcag2a","wcag2aa","wcag21aa"]).analyze()).violations,[]);
  await page.setViewportSize({width:1280,height:900});
  await page.getByRole("button",{name:"预览变更",exact:true}).click();
  await expect(categories.getByRole("button",{name:"预览变更",exact:true})).toHaveAttribute("aria-pressed","true");
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toBeHidden();
  await expect(page.getByRole("region",{name:"变更差异预览"})).toContainText("Original");
  await expect(page.getByRole("region",{name:"变更差异预览"})).toContainText("New name");
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await expect(categories.getByRole("button",{name:"概况",exact:true})).toHaveAttribute("aria-pressed","true");
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toHaveValue("New name");
  await categories.getByRole("button",{name:"预览变更",exact:true}).click();
  assert.equal(commands.length,0);
  await page.getByRole("button",{name:"应用更改",exact:true}).click();
  await expect.poll(()=>commands.length).toBe(1);
  assert.deepEqual(commands[0],{kind:"save_config",set:{sunshine_name:"New name",qp:24,controller:false,keyboard:false},remove:["nvenc_preset"],restart_policy:"manual"});
  await expect(categories.getByRole("button",{name:"预览变更",exact:true})).toHaveAttribute("aria-pressed","true");
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await page.getByRole("button",{name:"重启 Sunshine",exact:true}).click();
  const dialog=page.getByRole("dialog",{name:"确认重启 Sunshine"});
  await expect(dialog).toContainText("中断正在进行的串流");assert.equal(commands.length,1);
  await dialog.getByRole("button",{name:"确认",exact:true}).click();
  await expect.poll(()=>commands.length).toBe(2);
  assert.equal(commands[1].administrator_confirmed,true);
  await categories.getByRole("button",{name:"应用管理",exact:true}).click();
  await page.getByRole("button",{name:"刷新应用",exact:true}).click();
  await expect(page.locator("body")).toContainText("Steam");
  await page.getByRole("button",{name:"编辑",exact:true}).click();
  const appName=page.getByLabel("名称",{exact:true});
  assert.equal(await appName.getAttribute("maxlength"),null);
  for(const value of ["界".repeat(43)," Leading"]){
   const before=commands.length;await appName.fill(value);await page.getByRole("button",{name:"保存应用",exact:true}).click();await expect(appName).toHaveAttribute("aria-invalid","true");await expect(appName).toHaveValue(value);assert.equal(commands.length,before);
  }
  await appName.fill("Valid");const detachedInput=page.getByLabel("分离命令（每行一条）",{exact:true});
  await detachedInput.fill(" echo original ");const beforeInvalidCommand=commands.length;await page.getByRole("button",{name:"保存应用",exact:true}).click();await expect(detachedInput).toHaveAttribute("aria-invalid","true");await expect(detachedInput).toHaveValue(" echo original ");await expect(page.getByRole("dialog")).toHaveCount(0);assert.equal(commands.length,beforeInvalidCommand);await detachedInput.fill("");
  await page.getByLabel("名称",{exact:true}).fill("Steam Remote");
  await page.getByRole("button",{name:"保存应用",exact:true}).click();
  await expect.poll(()=>commands.some(command=>command.kind==="save_application")).toBe(true);
  const save=commands.find(command=>command.kind==="save_application");
  assert.deepEqual(save.target,{fingerprint:"c".repeat(64)});assert.equal(save.expected_revision,"d".repeat(64));
  await categories.getByRole("button",{name:"Moonlight 配对",exact:true}).click();
  await page.getByRole("button",{name:"刷新已配对客户端",exact:true}).click();
  await expect(page.locator("body")).toContainText("Moonlight TV");
  await categories.getByRole("button",{name:"诊断与 Sunshine 日志",exact:true}).click();
  await page.getByRole("button",{name:"读取最新日志",exact:true}).click();
  await expect(page.locator("body")).toContainText("Sunshine ready");
  await page.getByRole("button",{name:"读取诊断",exact:true}).click();
  await expect(page.locator("body")).toContainText("2026.914.233613");
  await categories.getByRole("button",{name:"显示与输入维护",exact:true}).click();
  await expect(page.getByRole("button",{name:"重置显示设备",exact:true})).toBeVisible();
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await page.getByRole("button",{name:"刷新服务状态",exact:true}).click();
  await expect(page.locator("body")).toContainText("running");
  const service=page.getByRole("region",{name:"Sunshine 服务",exact:true});
  await service.getByRole("button",{name:"停止",exact:true}).click();
  await page.getByRole("dialog").getByRole("button",{name:"确认",exact:true}).click();
  await expect(service).toContainText("stopped");
  await service.getByRole("button",{name:"刷新服务状态",exact:true}).click();
  await expect(service).toContainText("running");
  await expect(service).not.toContainText("stopped");

  const concurrentConfigDraft="Unsaved alongside app draft";
  await page.getByLabel("Sunshine 名称",{exact:true}).fill(concurrentConfigDraft);
  const beforeDiagnosticRevisit=commands.length;
  await categories.getByRole("button",{name:"诊断与 Sunshine 日志",exact:true}).click();
  await expect(page.locator("#sunshine-diagnostics")).toContainText("Sunshine ready");
  await expect(page.locator("#sunshine-diagnostics")).toContainText("2026.914.233613");
  assert.equal(commands.length,beforeDiagnosticRevisit,"revisiting diagnostics must use existing operation results");
  await categories.getByRole("button",{name:"应用管理",exact:true}).click();
  await page.getByRole("button",{name:"编辑",exact:true}).click();
  const detached=page.getByLabel("分离命令（每行一条）",{exact:true});
  await detached.fill("");
  await detached.pressSequentially("printf hello");
  await detached.press("Enter");
  await detached.pressSequentially("printf world");
  await expect(detached).toHaveValue("printf hello\nprintf world");
  const beforeCommands=commands.length;
  const cover=page.locator('input[name="cover"]');
  const coverKey=page.getByLabel("封面键",{exact:true});
  const coverBytes=Buffer.from([137,80,78,71]);
  await coverKey.fill("unsaved-cover");
  await cover.setInputFiles({name:"unsaved-cover.png",mimeType:"image/png",buffer:coverBytes});
  for(const name of ["概况","诊断与 Sunshine 日志","显示与输入维护","预览变更","Moonlight 配对","实例概览"]){
   await categories.getByRole("button",{name,exact:true}).click();
   await expect(page.locator("#sunshine-applications")).toBeHidden();
   await expect(page.getByRole("button",{name:"保存应用",exact:true})).toHaveCount(0);
   await expect(page.getByRole("button",{name:"上传 PNG",exact:true})).toHaveCount(0);
   assert.equal(commands.length,beforeCommands,"visiting other categories must not save an application or upload its cover");
  }
  await categories.getByRole("button",{name:"应用管理",exact:true}).click();
  await expect(page.getByLabel("名称",{exact:true})).toHaveValue("Steam Remote");
  await expect(detached).toHaveValue("printf hello\nprintf world");
  await expect(coverKey).toHaveValue("unsaved-cover");
  assert.deepEqual(await cover.evaluate(async input=>{
   const files=[...input.files];
   return Promise.all(files.map(async file=>({name:file.name,type:file.type,bytes:[...new Uint8Array(await file.arrayBuffer())]})));
  }),[{name:"unsaved-cover.png",type:"image/png",bytes:[...coverBytes]}]);
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toHaveValue(concurrentConfigDraft);
  await categories.getByRole("button",{name:"应用管理",exact:true}).click();
  await page.getByRole("button",{name:"保存应用",exact:true}).click();
  await page.getByRole("dialog",{name:"确认保存主机命令",exact:true}).getByRole("button",{name:"取消",exact:true}).click();
  assert.equal(commands.length,beforeCommands);
  await expect(detached).toHaveValue("printf hello\nprintf world");
  await page.getByRole("button",{name:"保存应用",exact:true}).click();
  await page.getByRole("dialog",{name:"确认保存主机命令",exact:true}).getByRole("button",{name:"确认",exact:true}).click();
  await expect.poll(()=>commands.length).toBe(beforeCommands+1);
  assert.deepEqual(commands.at(-1).application.detached,["printf hello","printf world"]);
  await detached.fill(Array.from({length:17},(_,index)=>"command "+index).join("\n"));
  await expect(detached).toHaveAttribute("aria-invalid","true");
  await expect(page.getByRole("button",{name:"保存应用",exact:true})).toBeEnabled();const beforeInvalidGroup=commands.length;await page.getByRole("button",{name:"保存应用",exact:true}).click();assert.equal(commands.length,beforeInvalidGroup);
  await expect(detached).toHaveValue(Array.from({length:17},(_,index)=>"command "+index).join("\n"));

  await page.getByLabel("封面键",{exact:true}).fill("test");
  await cover.setInputFiles({name:"wrong.txt",mimeType:"text/plain",buffer:Buffer.from("invalid")});
  await page.getByRole("button",{name:"上传 PNG",exact:true}).click();
  assert.equal(await cover.evaluate(element=>element.validity.customError),true);
  await cover.setInputFiles({name:"cover.png",mimeType:"image/png",buffer:Buffer.from([137,80,78,71])});
  assert.equal(await cover.evaluate(element=>element.validity.customError),false);
  await page.getByRole("button",{name:"上传 PNG",exact:true}).click();
  await expect.poll(()=>commands.some(command=>command.kind==="upload_cover")).toBe(true);

  emptyApplications=true;
  await page.getByRole("button",{name:"刷新应用",exact:true}).click();
  await expect(page.getByRole("button",{name:"编辑",exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:"新建应用",exact:true}).click();
  await page.getByRole("button",{name:"取消",exact:true}).click();
  await expect(page.getByRole("button",{name:"创建应用",exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:"刷新",exact:true}).click();
  await categories.getByRole("button",{name:"概况",exact:true}).click();
  await expect(page.getByLabel("Sunshine 名称",{exact:true})).toHaveValue("Original");
  assert.equal(configReads.length,2);
  await categories.getByRole("button",{name:"输入",exact:true}).click();
  await controller.click();
  await page.getByRole("option",{name:"未显式设置",exact:true}).click();
  await page.getByRole("button",{name:"预览变更",exact:true}).click();
  await expect(page.getByRole("region",{name:"变更差异预览"})).toContainText("控制器输入");
  await page.getByRole("button",{name:"应用更改",exact:true}).click();
  await expect.poll(()=>commands.at(-1)?.kind).toBe("save_config");
  assert.deepEqual(commands.at(-1),{kind:"save_config",set:{sunshine_name:"Original",qp:28,nvenc_preset:1,keyboard:false},remove:["controller"],restart_policy:"manual"});
  for(const state of ["failed","unknown"]){
   const created=serverStart+2_000_000+operationClock++;
   operations.unshift({operation_id:"op_"+randomUUID(),device_id:device.id,action:"sunshine.restart",state,attempt:state==="unknown"?1:0,created_at_micros:created,created_at_server:serverTimestamp(created),updated_at_micros:created,result:null,reconciliation:null,resolution:null,status_reason:"authorization_rotated"});
  }
  await page.getByRole("button",{name:"日志",exact:true}).click();
  const logSection=page.locator(".sunshine-logs-panel");
  const logDate=rangeEditor(page, "sunshine-log-date");
  await logDate.expectValue(serverToday);
  const browserToday=await page.evaluate(()=>{const now=new Date();return now.toISOString().slice(0,10)});
  assert.notEqual(browserToday,serverToday,"the default date comes from the server, not the browser clock");
  assert.equal(logDates.at(-1),serverToday);
  await expect(page.locator("body")).toContainText("Sunshine 进程已重启，运行时生效待验证");
  await expect(page.locator("body")).toContainText("fixture-process-generation");
  await expect(page.getByText("状态原因：实例密码已更换",{exact:true})).toHaveCount(2);
  await expect(page.getByRole("button",{name:"确认已成功",exact:true})).toBeVisible();
  await expect(page.locator("body")).not.toContainText("authorization_rotated");
  const oldUnknown={operation_id:"op_"+randomUUID(),device_id:device.id,action:"sunshine.restart",state:"unknown",attempt:1,created_at_micros:serverStart+1,created_at_server:serverTimestamp(serverStart+1),updated_at_micros:serverStart+1,result:null,reconciliation:null,resolution:null};
  operations.push(oldUnknown);
  const yesterday={operation_id:"op_"+randomUUID(),device_id:device.id,action:"sunshine.config.read",state:"succeeded",attempt:1,created_at_micros:serverStart-1,created_at_server:serverTimestamp(serverStart-1),updated_at_micros:serverStart-1,result:null,reconciliation:null,resolution:null};
  operations.push(yesterday);
  let lateToday;
  for(let index=0;index<175;index++){
   const created=index===174?serverEnd-1:serverStart+3_000_000+index;
   const operation={operation_id:"op_"+randomUUID(),device_id:device.id,action:"sunshine.config.read",state:"succeeded",attempt:1,created_at_micros:created,created_at_server:serverTimestamp(created),updated_at_micros:created,result:null,reconciliation:null,resolution:null};
   operations.unshift(operation);if(index===174)lateToday=operation;
  }
  const browserBoundaryDates=await page.evaluate(([early,late])=>[early,late].map(micros=>new Date(micros/1000).toISOString().slice(0,10)),[oldUnknown.created_at_micros,lateToday.created_at_micros]);
  assert.deepEqual(browserBoundaryDates,[serverToday,serverToday],"both UTC boundaries belong to the selected UTC day");
  await expect(logSection.getByText(oldUnknown.operation_id,{exact:true})).toHaveCount(0);
  largeLogPayloadOnce=true;
  await logSection.getByRole("button",{name:"刷新日志",exact:true}).click();
  await expect(logSection.locator("article")).toHaveCount(50);
  const pagination=logSection.getByRole("navigation",{name:"日志分页",exact:true});
  const nextPage=pagination.getByRole("button",{name:"下一页",exact:true});
  const previousPage=pagination.getByRole("button",{name:"上一页",exact:true});
  const firstPage=pagination.getByRole("button",{name:"首页",exact:true});
  const logLayout=await logSection.evaluate(element=>{
   const panel=element.querySelector(".xcss-content-panel").getBoundingClientRect();
   const instance=element.querySelector(".xcss-content-panel > p").getBoundingClientRect();
   const label=element.querySelector(".xcss-log-date-controls > label").getBoundingClientRect();
   const table=element.querySelector(".xcss-table-scroll").getBoundingClientRect();
   const pager=element.querySelector('nav[aria-label="日志分页"]').getBoundingClientRect();
   return {nameLeft:instance.left-label.left,nameAbove:instance.bottom<label.top,tableOutside:table.top>=panel.bottom,pagerBelow:pager.top>=table.bottom,rightAligned:Math.abs(pager.right-table.right)<1};
  });
  assert.ok(Math.abs(logLayout.nameLeft)<1&&logLayout.nameAbove&&logLayout.tableOutside&&logLayout.pagerBelow&&logLayout.rightAligned,JSON.stringify(logLayout));
  await expect(previousPage).toBeDisabled();
  await expect(firstPage).toBeDisabled();
  await expect(logSection.locator("article").filter({hasText:lateToday.operation_id})).toBeVisible();
  const seen=new Set();
  async function takePage(){const ids=await logSection.locator("article > code").allTextContents();assert.ok(ids.length<=50);for(const id of ids){assert.ok(!seen.has(id),"duplicate across history pages");seen.add(id)}}
  async function navigateHistory(button){const before=await logSection.locator("article > code").first().textContent();await button.click();await expect.poll(()=>logSection.locator("article > code").first().textContent()).not.toBe(before);await expect(logSection.getByText("正在读取所选日期的日志页…",{exact:true})).toHaveCount(0)}
  await takePage();
  while(await nextPage.isEnabled()){await navigateHistory(nextPage);await expect(previousPage).toBeEnabled();await takePage()}
  assert.equal(seen.size,operations.length-1,"every historical task remains reachable across bounded pages");
  const oldArticle=logSection.locator("article").filter({hasText:oldUnknown.operation_id});
  await expect(oldArticle).toBeVisible();
  await navigateHistory(previousPage);await expect(logSection.locator("article")).toHaveCount(50);
  await navigateHistory(nextPage);await expect(oldArticle).toBeVisible();
  await expect(oldArticle.locator("xpath=ancestor::tr")).toContainText("2030-01-02 00:00:00.000001 UTC");
  await expect(logSection.locator("article").filter({hasText:lateToday.operation_id})).toHaveCount(0);
  await expect(logSection.getByText(yesterday.operation_id,{exact:true})).toHaveCount(0);
  await checkDateRangeValidation(logDate,()=>fullLogRequests,serverYesterday,serverToday);
  await expect(logSection.locator("article")).toHaveCount(50);
  assert.equal(logDates.at(-1),serverYesterday);
  const rangedSeen=new Set();
  while(true){for(const id of await logSection.locator("article > code").allTextContents())rangedSeen.add(id);if(!await nextPage.isEnabled())break;await navigateHistory(nextPage)}
  assert.ok(rangedSeen.has(yesterday.operation_id),"range includes previous day's logs");
  assert.ok(rangedSeen.has(lateToday.operation_id),"range includes the end day's final log");
  await logDate.fill(serverYesterday);
  await expect(logSection.locator("article")).toHaveCount(1);
  await expect(logSection.getByText(yesterday.operation_id,{exact:true})).toBeVisible();
  assert.equal(logDates.at(-1),serverYesterday);
  const beforeInvalidDate=fullLogRequests;
  await logDate.fill("");
  await expect(logSection.getByText("请输入有效的年月日。",{exact:true})).toBeVisible();
  await expect(logSection.getByRole("button",{name:"刷新日志",exact:true})).toBeDisabled();
  assert.equal(fullLogRequests,beforeInvalidDate,"an empty date must not fetch unbounded history");
  await logDate.fill(serverToday);
  await expect(logSection.locator("article")).toHaveCount(50);
  assert.equal(logDates.at(-1),serverToday);
  while(await nextPage.isEnabled()){await navigateHistory(nextPage);await expect(previousPage).toBeEnabled()}
  await expect(oldArticle).toBeVisible();
  const beforeResolveRequests=fullLogRequests;
  await oldArticle.getByRole("button",{name:"确认已成功",exact:true}).click();
  await page.getByRole("dialog").getByRole("button",{name:"确认",exact:true}).click();
  await expect(oldArticle.locator("xpath=ancestor::tr")).toContainText("已人工核对");
  await expect.poll(()=>fullLogRequests).toBeGreaterThan(beforeResolveRequests);
  await expect(logSection.getByRole("button",{name:"刷新日志",exact:true})).toBeEnabled();
  const afterResolveRequests=fullLogRequests;
  await page.waitForTimeout(2200);
  assert.equal(fullLogRequests,afterResolveRequests,"a historical page is fetched on entry, navigation or refresh, not every two seconds");
  holdNextCalendar=true;
  await page.getByRole("button",{name:"详细信息",exact:true}).click();
  await page.getByRole("button",{name:"日志",exact:true}).click();
  await expect.poll(()=>typeof releaseCalendar).toBe("function");
  await expect(logSection.locator("article")).toHaveCount(0);
  await expect(logSection.getByText("正在读取 UTC 当天日期…",{exact:true})).toBeVisible();
  failNextLog=true;
  releaseCalendar();
  await expect(page.getByText("日志暂时不可用",{exact:true})).toBeVisible();
  await expect(logSection.locator("article")).toHaveCount(0);
  await expect(logSection.getByText("暂无日志",{exact:true})).toHaveCount(0);
  await logSection.getByRole("button",{name:"刷新日志",exact:true}).click();
  await expect(logSection.locator("article")).toHaveCount(50);
  failCalendarOnce=true;
  await page.getByRole("button",{name:"详细信息",exact:true}).click();
  await page.getByRole("button",{name:"日志",exact:true}).click();
  await expect(page.getByText("无法读取服务器当天日期",{exact:true})).toBeVisible();
  const beforeManualDate=fullLogRequests;
  await rangeEditor(page, "sunshine-log-date").fill(serverToday);
  await expect.poll(()=>fullLogRequests).toBeGreaterThan(beforeManualDate);
  await expect(logSection.locator("article")).toHaveCount(50);
  await expect(page.getByText("无法读取服务器当天日期",{exact:true})).toHaveCount(0);
  for(const operation of operations)if(operation.state==="unknown")operation.state="resolved";
  externalBlocking=1;
  const beforeSummary=summaryResponses;
  await page.getByRole("button",{name:"详细信息",exact:true}).click();
  await categories.getByRole("button",{name:"诊断与 Sunshine 日志",exact:true}).click();
  await expect.poll(()=>summaryResponses).toBeGreaterThan(beforeSummary);
  await expect(page.getByRole("button",{name:"读取诊断",exact:true})).toBeEnabled();
  await expect(page.getByText("该实例有 1 个未完成任务。请在日志页选择相关日期核对；若无记录，请联系原操作者。",{exact:true})).toBeVisible();
  externalBlocking=0;
  await expect(page.getByRole("button",{name:"读取诊断",exact:true})).toBeEnabled({timeout:5000});
  failSummaryOnce=true;
  await expect.poll(()=>failedSummaryResponses).toBeGreaterThan(0);
  await expect(page.getByRole("button",{name:"读取诊断",exact:true})).toBeEnabled();
  await expect(page.getByText("任务状态暂时不可用",{exact:true})).toBeVisible();
  assert.deepEqual(errors,[]);console.log(engine.name()+": current explicit-capability configuration, applications, pairing, logs, diagnostics and service controls passed");
 }finally{await browser.close();}
}}finally{await new Promise(done=>server.httpServer.close(done));}
