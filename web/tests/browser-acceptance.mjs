import { checkAccountPage } from "./account-page.mjs";
import { checkWebLanguage } from "./language.mjs";
import { checkHeaderActions, checkHeaderLogout } from "./header-actions.mjs";
import { paintedSurfaceColor } from "./painted-surface.mjs";
import assert from "node:assert/strict";
import {chromium,firefox,expect} from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import {preview} from "vite";
import {randomUUID} from "node:crypto";
const session={authenticated:true,user_id:"A".repeat(43),username:"admin",role:"admin",csrf_token:"A".repeat(43)};
const configFields=[];
async function assertColumnContentAlignment(table){
 const offsets=await table.evaluate(element=>{const textStart=cell=>{const walker=document.createTreeWalker(cell,NodeFilter.SHOW_TEXT);let text;while((text=walker.nextNode())&&!text.textContent.trim()){}if(!text)throw new Error("table cell has no visible text");const range=document.createRange();range.selectNodeContents(text);return range.getBoundingClientRect().left};const contentStart=cell=>textStart(cell);const headings=[...element.querySelectorAll("thead th")],values=[...element.querySelector("tbody tr").children];if(headings.length!==values.length)throw new Error("table column count mismatch");return headings.map((heading,index)=>Math.abs(textStart(heading)-contentStart(values[index])))});
 assert.ok(offsets.every(offset=>offset<0.5),`column content offsets: ${JSON.stringify(offsets)}`);
}
const server=await preview({preview:{host:"127.0.0.1",port:0,strictPort:true}});
try {
 for(const engine of [chromium,firefox]){
  const browser=await engine.launch();
  try {
   const context=await browser.newContext({ locale: "zh-CN", viewport:{width:390,height:844}});const page=await context.newPage();const errors=[];const devices=[];let posts=0;let configAttempts=0;let authorizationAttempts=0;let failNextAuthorization=false;let failNextTasks=false;
   page.on("pageerror",e=>errors.push(e.message));
   await page.route("**/api/v1/**",async route=>{
    const req=route.request();const path=new URL(req.url()).pathname;
    if(path.endsWith("/sunshine/config-fields")){configAttempts++;if(configAttempts===1)return route.fulfill({status:503,json:{code:"service_unavailable",retryable:true,message:"SECRET fields",request_id:"fields-failure-123"}});return route.fulfill({json:configFields});}
    if(path.endsWith("/sunshine/devices")){
     if(req.method()==="POST"){
      posts++;assert.equal(req.headers()["x-csrf-token"],session.csrf_token);
      assert.deepEqual(req.postDataJSON(),{name:"新实例"});
      if(posts===1)return route.fulfill({status:503,json:{code:"service_unavailable",retryable:true,message:"SECRET details",request_id:"create-123"}});
      const device={id:randomUUID(),name:"新实例",registered:false,pairing_pending:true,revoked:false,client_online:false,sunshine_reachable:null,configuration_state:"unknown",snapshot:null,capabilities:null,last_seen_at_micros:null};
      devices.push(device);return route.fulfill({status:201,json:{device,manager_id:randomUUID(),token:"b".repeat(36)}});
     }return route.fulfill({json:devices});
    }
    if(path.match(/\/sunshine\/devices\/[^/]+$/)&&req.method()==="PATCH"){
     devices[0].name=req.postDataJSON().name;
     return route.fulfill({json:devices[0]});
    }
    if(path.endsWith("/pairing")&&req.method()==="DELETE"){
     devices[0].pairing_pending=false;
     return route.fulfill({status:204})
    }
    if(path.match(/\/sunshine\/devices\/[^/]+$/)&&req.method()==="DELETE"){
     devices.splice(0,1);return route.fulfill({status:204})
    }
    if(path.endsWith("/authorization")){
     authorizationAttempts++;
     if(failNextAuthorization){
      failNextAuthorization=false;
      return route.fulfill({status:503,json:{code:"service_unavailable",retryable:true,message:"SECRET authorization",request_id:"authorization-failure-123"}});
     }
     return route.fulfill({json:{manager_id:randomUUID(),device_id:devices[0]?.id??randomUUID(),authorization_code:"b".repeat(36)}});
    }
    if(path.endsWith("/tasks/calendar"))return route.fulfill({json:{today:"2030-01-02"}});
    if(path.endsWith("/tasks/summary"))return route.fulfill({json:{blocking_count:0}});
    if(path.endsWith("/tasks")){if(failNextTasks){failNextTasks=false;return route.fulfill({status:503,json:{code:"service_unavailable",retryable:true,message:"SECRET tasks",request_id:"tasks-failure-123"}})}return route.fulfill({json:new URL(req.url()).searchParams.has("date")?{operations:[],next_cursor:null,previous_cursor:null}:[]});}
    return route.fulfill({json:session});
   });
   await page.goto("http://127.0.0.1:"+server.httpServer.address().port);
   await expect(page.getByRole("alert")).toContainText("fields-failure-123");
   await expect(page.locator("body")).not.toContainText("SECRET fields");
   await page.getByRole("alert").getByRole("button",{name:"重试",exact:true}).click();
   await expect.poll(()=>configAttempts).toBeGreaterThan(1);
   await expect(page.getByRole("alert")).toHaveCount(0);
   await checkHeaderActions(page, "/sunshine/devices");
      await checkAccountPage(page);
   await page.setViewportSize({width:1838,height:900});
   const listGaps=await page.evaluate(()=>{
    const header=document.querySelector(".xcss-page-header");
    const menu=document.querySelector(".xcss-header-navigation button[aria-pressed='true']");
    const first=document.querySelector(".xcss-table-scroll[aria-label='实例统计']");
        const second=document.querySelector("section.xcss-content-stack[aria-label='实例']");
    if(!header||!menu||!first||!second)throw new Error("Sunshine spacing fixture is incomplete");
    // Layout spacing uses element boxes; glyph ink depends on browser font metrics.
    const headerBox=header.getBoundingClientRect(),menuBox=menu.getBoundingClientRect(),titleBox=first.getBoundingClientRect();
    return {menuWithinHeader:menuBox.top>=headerBox.top&&menuBox.bottom<=headerBox.bottom,titleGap:titleBox.top-headerBox.bottom,contentGaps:[second.getBoundingClientRect().top-first.closest("section").getBoundingClientRect().bottom]};
   });
   assert.ok(listGaps.menuWithinHeader,JSON.stringify(listGaps));
   assert.ok(Math.abs(listGaps.titleGap)<=1,JSON.stringify(listGaps));
   assert.ok(listGaps.contentGaps.every(gap=>Math.abs(gap-16)<=1),JSON.stringify(listGaps));
   await expect(page.locator("h2").filter({hasText:/^(统计|实例|实例列表)$/})).toHaveCount(0);
   const statisticsRowTops=await page.locator(".xcss-statistics-table tbody tr").evaluateAll(rows=>rows.map(row=>({label:row.querySelector("th").getBoundingClientRect().top,value:row.querySelector("td").getBoundingClientRect().top})));
   assert.equal(statisticsRowTops.length,4);
   assert.ok(statisticsRowTops.every(row=>Math.abs(row.label-statisticsRowTops[0].label)<=1&&Math.abs(row.value-row.label)<=1),JSON.stringify(statisticsRowTops));
   await page.setViewportSize({width:360,height:740});
   await page.getByRole("button",{name:"新建实例",exact:true}).click();
   await expect(page.getByRole("alert")).toContainText("create-123");await expect(page.locator("body")).not.toContainText("SECRET details");
   await page.getByRole("button",{name:"新建实例",exact:true}).click();
   await expect(page.getByRole("button",{name:"关闭通知",exact:true})).toBeVisible();
   await expect(page).toHaveURL(/#instances$/);
   await expect(page.getByRole("button",{name:"实例列表",exact:true})).toHaveAttribute("aria-pressed","true");
   await expect(page.getByRole("link",{name:"选择实例 新实例"})).toHaveText("新实例");
   await expect(page.getByRole("button",{name:"关闭通知",exact:true})).toHaveCount(0,{timeout:7000});
   await expect(page.getByRole("complementary")).toHaveCount(0);
   await page.getByRole("button",{name:"实例列表",exact:true}).click();
   await expect(page.getByRole("link",{name:"选择实例 新实例"})).toHaveText("新实例");
   const table=page.getByRole("table",{name:"Sunshine 实例列表"});
   const statistics=page.getByRole("table",{name:"实例统计"});
   await expect(statistics.getByRole("columnheader")).toHaveText(["统计项","总数 / 在线"]);
   await expect(statistics.getByRole("rowheader")).toHaveText(["总数","Windows","Linux","macOS"]);
   await expect(statistics.locator("tbody td")).toHaveText(["1 / 0","0 / 0","0 / 0","0 / 0"]);
   await expect(table.locator("tbody tr")).toHaveCount(1);
   await expect(table.getByRole("columnheader")).toHaveText(["实例名称","注册状态","客户端 状态","Sunshine 接口","配置状态","操作系统/架构","最近连接","删除"]);
   await expect(table.locator("tbody td")).toHaveText(["等待配对","离线","未知","尚未核对","尚未上报","尚未连接","删除"]);
   await expect(table).not.toContainText(devices[0].id);
   await expect(table).not.toContainText("b".repeat(36));
   const instanceNameStyle=await table.getByRole("link",{name:"选择实例 新实例"}).evaluate(element=>({color:getComputedStyle(element).color,parentColor:getComputedStyle(element.parentElement).color,decoration:getComputedStyle(element).textDecorationLine}));
   assert.equal(instanceNameStyle.color,instanceNameStyle.parentColor);
   assert.equal(instanceNameStyle.decoration,"none");
   assert.ok((await table.locator("th, td").evaluateAll(elements=>elements.map(element=>getComputedStyle(element).textAlign))).every(value=>value==="left"));
   await assertColumnContentAlignment(table);
   assert.ok((await table.locator(".xcss-actions").evaluateAll(elements=>elements.map(element=>getComputedStyle(element).justifyContent))).every(value=>value==="flex-start"));
   assert.equal(await table.locator("tbody tr").evaluate(row=>getComputedStyle(row).display),"table-row");
   assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   assert.deepEqual((await new AxeBuilder({page}).withTags(["wcag2a","wcag2aa","wcag21aa"]).analyze()).violations,[]);
   failNextAuthorization=true;
   await page.getByRole("link",{name:"选择实例 新实例"}).click();
   await expect(page.getByRole("button",{name:"详细信息",exact:true})).toHaveAttribute("aria-pressed","true");
   const pairingDetails=page.getByRole("region",{name:"配对账户信息"});
   const overview=page.getByRole("region",{name:"实例概览"});
   const configPanel=page.getByRole("region",{name:"配置设置"});
   const categories=page.getByRole("navigation",{name:"Sunshine 配置分类"});
   await expect(configPanel.locator(":scope > nav")).toHaveCount(0);
   await expect(page.locator(".sunshine-workspace > nav")).toHaveCount(1);
   await expect(configPanel.getByRole("heading",{name:"配置设置",exact:true})).toHaveCount(0);
   const menuTextOffset=await categories.evaluate(menu=>{
    const firstButton=menu.querySelector("button");
    if(!firstButton)throw new Error("Sunshine detail menu is incomplete");
    const label=document.createRange();label.selectNodeContents(firstButton);
    return label.getBoundingClientRect().left-menu.getBoundingClientRect().left;
   });
   assert.ok(Math.abs(menuTextOffset)<1,`Sunshine detail menu text is offset by ${menuTextOffset}px`);
   assert.equal(await categories.evaluate(menu=>getComputedStyle(menu).borderTopWidth),"0px");
   assert.equal(await categories.locator("button").first().textContent(),"实例概览");
   await expect(categories.getByRole("button",{name:"实例概览"})).toHaveAttribute("aria-pressed","true");
   await expect(configPanel.getByRole("region",{name:"实例概览"})).toBeVisible();
   await expect(overview.locator(":scope > section")).toHaveCount(4);
   const actionTextOffset=await overview.locator(".sunshine-instance-actions").evaluate(section=>{
    const button=section.querySelector("button");
    if(!button)throw new Error("Sunshine instance action is missing");
    const text=document.createRange();text.selectNodeContents(button);
    return text.getBoundingClientRect().left-section.getBoundingClientRect().left;
   });
   assert.ok(Math.abs(actionTextOffset)<1,`Sunshine action text is offset by ${actionTextOffset}px`);
   await expect(pairingDetails.locator("dd").first()).toHaveText("新实例");
   await expect(pairingDetails).toContainText(devices[0].id);
   await expect(page.getByText("等待配对",{exact:true})).toBeVisible();
   await expect(page.getByLabel("Sunshine 密码",{exact:true})).toHaveCount(0);
   await expect(page.getByText("账户名",{exact:true})).toHaveCount(1);
   await expect(page.getByText("账户",{exact:true})).toHaveCount(1);
   await expect(page.getByText("密码",{exact:true})).toHaveCount(1);
   await expect(page.getByRole("region",{name:"实例设置"})).toBeVisible();
   await expect(page.getByRole("region",{name:"Sunshine 状态"})).toBeVisible();
   await expect(page.getByRole("alert").filter({hasText:"无法读取实例密码"})).toContainText("authorization-failure-123");
   await page.getByRole("group",{name:"全局操作"}).getByRole("button",{name:"刷新",exact:true}).click();
   await expect.poll(()=>authorizationAttempts).toBeGreaterThan(1);
   await expect(page.locator(".sunshine-workspace .sunshine-token").first()).toHaveText("b".repeat(36));
   await expect(page.getByRole("alert")).toHaveCount(0);
   await page.setViewportSize({width:1838,height:900});
   const overviewAlignment=await page.evaluate(()=>{
    const labels=[...document.querySelectorAll(".sunshine-overview .sunshine-detail-list dt"),document.querySelector(".sunshine-instance-settings .xcss-form-field > span")];
    const values=[...document.querySelectorAll(".sunshine-overview .sunshine-detail-list dd"),document.querySelector(".sunshine-instance-settings .xcss-form-field > input")];
    if(labels.some(value=>!value)||values.some(value=>!value))throw new Error("Sunshine overview columns are incomplete");
    return {labels:labels.map(value=>value.getBoundingClientRect().left),values:values.map(value=>value.getBoundingClientRect().left)};
   });
   assert.ok(overviewAlignment.labels.every(value=>Math.abs(value-overviewAlignment.labels[0])<=1),JSON.stringify(overviewAlignment));
   assert.ok(overviewAlignment.values.every(value=>Math.abs(value-overviewAlignment.values[0])<=1),JSON.stringify(overviewAlignment));
   const layoutGaps=await page.evaluate(()=>{
    const header=document.querySelector(".xcss-page-header");
    const primary=document.querySelector(".xcss-header-navigation button[aria-pressed='true']");
    const secondary=document.querySelector(".sunshine-config-navigation button[aria-pressed='true']");
    const panel=document.querySelector(".sunshine-config-panel");
    if(!header||!primary||!secondary||!panel)throw new Error("Sunshine detail menu layout is incomplete");
    const textRect=element=>{const range=document.createRange();range.selectNodeContents(element);return range.getBoundingClientRect()};
    const first=textRect(primary),second=textRect(secondary);
    return {expected:parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--xcss-content-spacing")),gaps:[first.top-header.getBoundingClientRect().top,second.top-first.bottom,panel.getBoundingClientRect().top-second.bottom]};
   });
   assert.ok(layoutGaps.gaps.every(gap=>Math.abs(gap-layoutGaps.expected)<=1),`Sunshine visible menu gaps differ: ${JSON.stringify(layoutGaps)}`);
   await page.setViewportSize({width:390,height:844});
   const wrappedMenus=await page.evaluate(()=>{
    const primary=document.querySelector(".xcss-header-navigation");
    const secondary=document.querySelector(".sunshine-config-navigation");
    if(!primary||!secondary)throw new Error("Sunshine menus are missing");
    return {primaryWrap:getComputedStyle(primary).flexWrap,secondaryWrap:getComputedStyle(secondary).flexWrap,primaryFits:primary.scrollWidth<=primary.clientWidth+1,secondaryFits:secondary.scrollWidth<=secondary.clientWidth+1,secondaryRows:new Set([...secondary.children].map(item=>item.getBoundingClientRect().top)).size};
   });
   assert.equal(wrappedMenus.primaryWrap,"wrap");
   assert.equal(wrappedMenus.secondaryWrap,"wrap");
   assert.ok(wrappedMenus.primaryFits&&wrappedMenus.secondaryFits,JSON.stringify(wrappedMenus));
   assert.ok(wrappedMenus.secondaryRows>1,JSON.stringify(wrappedMenus));
   const crowdedPrimary=await page.locator(".xcss-header-navigation").evaluate(menu=>{
    const extras=Array.from({length:4},()=>{const item=menu.firstElementChild.cloneNode(true);item.textContent="额外菜单";menu.append(item);return item});
    const result={rows:new Set([...menu.children].map(item=>item.getBoundingClientRect().top)).size,fits:menu.scrollWidth<=menu.clientWidth+1};
    extras.forEach(item=>item.remove());return result;
   });
   assert.ok(crowdedPrimary.rows>1&&crowdedPrimary.fits,JSON.stringify(crowdedPrimary));
   const nameInput=page.getByRole("textbox",{name:"实例名称",exact:true});
   await nameInput.fill("正在编辑的名称");
   await categories.getByRole("button",{name:"概况",exact:true}).click();
   await expect(page.locator("[data-config-category] > h3")).toHaveCount(0);
   await expect(page.locator("#sunshine-instance-overview")).toBeHidden();
   await expect(page.getByRole("button",{name:/^(从 客户端 读取配置|使用最新配置|获取客户端最新配置)/})).toHaveCount(0);
   await expect(page.getByRole("button",{name:"重启 Sunshine",exact:true})).toBeVisible();
   await categories.getByRole("button",{name:"输入",exact:true}).click();
   await expect(page.getByRole("button",{name:"重启 Sunshine",exact:true})).toHaveCount(0);
   await categories.getByRole("button",{name:"预览变更",exact:true}).click();
   await expect(page.getByRole("region",{name:"预览变更",exact:true})).not.toContainText("请先在概况中读取配置");
   await expect(page.getByRole("region",{name:"预览变更",exact:true}).getByRole("button",{name:"应用更改",exact:true})).toHaveCount(0);
   await expect(page.getByRole("button",{name:"重启 Sunshine",exact:true})).toHaveCount(0);
   await categories.getByRole("button",{name:"实例概览"}).click();
   await expect(nameInput).toHaveValue("正在编辑的名称");
   devices[0].name="另一浏览器保存的名称";
   await page.getByRole("group",{name:"全局操作"}).getByRole("button",{name:"刷新",exact:true}).click();
   await expect(pairingDetails.locator("dd").first()).toHaveText(devices[0].name);
   await expect(nameInput).toHaveValue("正在编辑的名称");
   await nameInput.fill("新实例");
   await page.getByRole("button",{name:"保存名称",exact:true}).click();
   await expect.poll(()=>devices[0].name).toBe("新实例");
   await expect(page.getByRole("button",{name:"保存名称",exact:true})).toBeDisabled();
   failNextTasks=true;
   await page.getByRole("group",{name:"全局操作"}).getByRole("button",{name:"刷新",exact:true}).click();
   await expect(page.getByRole("alert")).toContainText("tasks-failure-123");
   await expect(page.locator("body")).not.toContainText("SECRET tasks");
   await expect(page.getByRole("alert")).toHaveCount(0,{timeout:5000});
   await nameInput.fill("\uFEFF"+"a".repeat(32));
   await expect(page.getByRole("alert")).toContainText("实例名称须为 1–32 个字符");
   await expect(page.getByRole("button",{name:"保存名称",exact:true})).toBeDisabled();
   assert.equal(devices[0].name,"新实例");
   await nameInput.fill("\uFEFF新实例");
   await expect(page.getByRole("alert")).toHaveCount(0);
   await expect(page.getByRole("button",{name:"保存名称",exact:true})).toBeEnabled();
   await page.getByRole("button",{name:"保存名称",exact:true}).click();
   await expect.poll(()=>devices[0].name).toBe("\uFEFF新实例");
   await nameInput.fill("新实例");
   await page.getByRole("button",{name:"保存名称",exact:true}).click();
   await expect.poll(()=>devices[0].name).toBe("新实例");
   for(const theme of ["dark","light"]){
    await page.getByRole("button",{name:theme==="dark"?"切换到深色模式":"切换到浅色模式",exact:true}).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme",theme);
    const readColors=async()=>({...await page.evaluate(()=>({page:getComputedStyle(document.documentElement).backgroundColor,cardText:getComputedStyle(document.querySelector(".sunshine-config-panel")).color,actionText:getComputedStyle(document.querySelector('.sunshine-config-navigation button:not([aria-pressed="true"])')).color})),card:await paintedSurfaceColor(page.locator(".sunshine-config-panel"))});
    if(theme==="light")await expect.poll(readColors).toEqual({page:"rgb(255, 255, 255)",card:"rgb(242, 242, 242)",cardText:"rgb(31, 31, 31)",actionText:"rgb(68, 68, 68)"});
    else await expect.poll(async()=>(await readColors()).card).not.toBe("rgb(242, 242, 242)");
    const violations=(await new AxeBuilder({page}).withTags(["wcag2a","wcag2aa","wcag21aa"]).analyze()).violations;
    assert.deepEqual(violations,[]);
   }
   assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   await expect(page.locator("body")).toContainText("更换后客户端必须重新配对");
   await page.getByRole("button",{name:"更换密码",exact:true}).click();
   const passwordDialog=page.getByRole("dialog",{name:"更换密码",exact:true});
   await expect(passwordDialog).toContainText("新的密码重新配对");
   await passwordDialog.getByRole("button",{name:"取消",exact:true}).click();
   const detailActions=page.getByRole("button",{name:"删除实例",exact:true}).locator("..");
   await detailActions.evaluate(element=>element.click());
   await expect(page.getByRole("dialog")).toHaveCount(0);
   await page.getByRole("button",{name:"取消配对",exact:true}).click();
   await page.getByRole("dialog").getByRole("button",{name:"取消",exact:true}).click();
   assert.equal(devices[0].pairing_pending,true);
   await expect(page.getByText("等待配对",{exact:true})).toBeVisible();
   await page.getByRole("button",{name:"取消配对",exact:true}).click();await page.getByRole("button",{name:"确认",exact:true}).click();
   await expect(page.getByText("配对已取消",{exact:true})).toBeVisible();
   await expect(page.locator(".sunshine-workspace .sunshine-token").first()).toHaveText("b".repeat(36));
   while(await page.getByRole("button",{name:"关闭通知",exact:true}).count())await page.getByRole("button",{name:"关闭通知",exact:true}).first().click();
   await page.getByRole("button",{name:"日志",exact:true}).click();
   await expect(page.getByRole("button",{name:"日志",exact:true})).toHaveAttribute("aria-pressed","true");
   await expect(page.locator(".sunshine-workspace > .sunshine-logs-panel")).toBeVisible();
   await expect(page.locator(".sunshine-logs-panel > h2")).toHaveCount(0);
   await page.setViewportSize({width:1838,height:900});
   await expect.poll(()=>page.evaluate(()=>document.fonts.status),{timeout:5000,message:"Sunshine log fonts did not settle"}).toBe("loaded");
   try{
    // Route updates, font loading and resizing may complete in different browser frames.
    // Keep the same visible 16px spacing requirement once the logs layout has settled.
    await expect.poll(()=>page.evaluate(async()=>{
     await new Promise(resolve=>requestAnimationFrame(resolve));
     const menu=document.querySelector(".xcss-header-navigation button[aria-pressed='true']");
     const panel=document.querySelector(".sunshine-workspace > .sunshine-logs-panel");
     if(!menu||!panel)throw new Error("Sunshine log layout is incomplete");
     const text=document.createRange();text.selectNodeContents(menu);
     const gap=panel.getBoundingClientRect().top-text.getBoundingClientRect().bottom;
     return {gap,withinSpacing:Math.abs(gap-16)<=1};
    }),{timeout:5000,message:"Sunshine log menu spacing must be 16 ± 1px"}).toMatchObject({withinSpacing:true});
   }catch(error){
    console.error("Sunshine log layout diagnostics",await page.evaluate(()=>({
     viewport:{width:innerWidth,height:innerHeight},fonts:document.fonts.status,
     elements:[...document.querySelectorAll(".xcss-page-header,.xcss-header-navigation,.xcss-shell-main > *, .sunshine-workspace > *")].map(element=>({tag:element.tagName,className:element.className,hidden:element.hidden,display:getComputedStyle(element).display,rect:element.getBoundingClientRect().toJSON()})),
     errors:[...document.querySelectorAll("[role='alert']")].map(element=>element.textContent?.replace(/SECRET[^。\n]*/g,"[redacted]").slice(0,160)),
    })));
    throw error;
   }
   const sunshineLogControls=await page.evaluate(()=>{
    const label=document.querySelector(".sunshine-logs-panel .xcss-log-date-controls > label");
    const button=document.querySelector(".sunshine-logs-panel .xcss-log-date-controls button");
    const input=document.querySelector(".sunshine-logs-panel .xcss-log-date-controls input");
    if(!label||!button||!input)throw new Error("Sunshine log date controls are incomplete");
    const text=element=>{const range=document.createRange();range.selectNodeContents(element);return range.getBoundingClientRect()};
    return {textOffset:text(button).top-text(label).top,buttonRightOfLabel:button.getBoundingClientRect().left>=text(label).right,inputBelowLabel:input.getBoundingClientRect().top>label.getBoundingClientRect().bottom};
   });
   assert.ok(Math.abs(sunshineLogControls.textOffset)<=1&&sunshineLogControls.buttonRightOfLabel&&sunshineLogControls.inputBelowLabel,JSON.stringify(sunshineLogControls));
   await page.setViewportSize({width:360,height:740});
   await page.getByRole("button",{name:"详细信息",exact:true}).click();
   await checkWebLanguage(page, {"routes":[["instances","Instance list"],["details","Details"],["logs","Logs"]],"names":["新实例"]});
   await page.getByRole("button",{name:"实例列表",exact:true}).click();
   await page.getByRole("button",{name:"删除",exact:true}).click();
   await page.getByRole("button",{name:"取消",exact:true}).click();
   await expect(page.getByRole("button",{name:"确认删除",exact:true})).toHaveCount(0);
   await page.getByRole("button",{name:"删除",exact:true}).click();
   await page.getByRole("button",{name:"确认删除",exact:true}).click();
   await expect(page.getByText("暂无实例，请新建并注册 客户端。",{exact:true})).toBeVisible();
   await checkHeaderLogout(page, session.csrf_token);
   assert.deepEqual(errors,[]);console.log(engine.name()+": Client registration, name limits, CSRF, default appearance, mobile and WCAG passed");
  }finally{await browser.close();}
 }
}finally{await new Promise(done=>server.httpServer.close(done));}
