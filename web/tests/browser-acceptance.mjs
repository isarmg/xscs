import assert from 'node:assert/strict';
import {chromium,firefox,expect} from '@playwright/test';
import {preview} from 'vite';
import {randomUUID} from 'node:crypto';
const session={authenticated:true,user_id:'A'.repeat(43),username:'admin',role:'admin',csrf_token:'A'.repeat(43)};
const server=await preview({preview:{host:'127.0.0.1',port:0,strictPort:true}});
try {
 for(const engine of [chromium,firefox]){
  const browser=await engine.launch();
  try{
   const page=await browser.newPage({locale:'zh-CN'});const errors=[];page.on('pageerror',e=>errors.push(e.message));
   await page.emulateMedia({colorScheme:'light'});
   await page.addInitScript(()=>localStorage.removeItem('sarmg:theme'));
   const diagnostics={backend:'rustdesk',execution_mode:'embedded',integration_state:'awaiting_integration',rustdesk_version:null};
   const device={id:randomUUID(),name:'原生 RustDesk 主机',role:'controlled',addresses:[{host:'host.example.com',port:21118}],addresses_at_micros:Date.now()*1000,registered:true,pairing_pending:false,revoked:false,client_online:true,diagnostics,capabilities:{protocol:'xscs-management/2',client_version:'2.0.0-dev',os:'windows_x86_64',backend:'rustdesk',execution_mode:'embedded',diagnostics:true},last_seen_at_micros:Date.now()*1000};
   const controller={...device,id:randomUUID(),name:'RustDesk 主控端',role:'controller',addresses:[],addresses_at_micros:null};
   const devices=[device,controller],operations=[];const edges=new Set();let submitted=0,accessRevision=0;
   await page.route('**/api/v1/**',async route=>{
    const req=route.request(),path=new URL(req.url()).pathname;
    if(path==='/api/v1/rustdesk/devices'){
     if(req.method()==='POST'){
      assert.equal(req.headers()['x-csrf-token'],session.csrf_token);assert.equal(req.postDataJSON().role,'controller');
      const created={...device,id:randomUUID(),name:'新实例',role:req.postDataJSON().role,addresses:[],addresses_at_micros:null,registered:false,client_online:false,pairing_pending:true,capabilities:null,diagnostics:null};devices.push(created);
      return route.fulfill({status:201,json:{device:created,manager_id:randomUUID(),token:'b'.repeat(36)}});
     }
     return route.fulfill({json:devices});
    }
    if(path.endsWith('/access')){
     const id=path.split('/').at(-2);const peer=id===device.id?controller.id:device.id;
     if(req.method()==='PUT'){
      assert.equal(req.headers()['x-csrf-token'],session.csrf_token);assert.equal(req.postDataJSON().expected_revision,accessRevision.toString(16).padStart(64,'0'));assert.ok(req.postDataJSON().peer_ids.every(p=>p===peer));
      if(req.postDataJSON().peer_ids.length)edges.add('allowed');else edges.delete('allowed');accessRevision++;
     }
     return route.fulfill({json:{peer_ids:edges.has('allowed')?[peer]:[],revision:accessRevision.toString(16).padStart(64,'0')}});
    }
    if(path.endsWith('/authorization'))return route.fulfill({json:{manager_id:randomUUID(),device_id:device.id,authorization_code:'c'.repeat(36)}});
    if(path.endsWith('/tasks')){
     if(req.method()==='POST'){
      submitted++;assert.equal(req.headers()['x-csrf-token'],session.csrf_token);assert.ok(req.headers()['idempotency-key']);assert.deepEqual(req.postDataJSON(),{kind:'read_diagnostics'});
      const op={operation_id:`op_${randomUUID()}`,device_id:device.id,action:'rustdesk.diagnostics.read',state:'succeeded',created_at_server:new Date().toISOString(),result:{kind:'diagnostics_read',snapshot:diagnostics},reconciliation:null};operations.unshift(op);return route.fulfill({status:202,json:op});
     }
     assert.equal(new URL(req.url()).search,'?recent=50');return route.fulfill({json:operations});
    }
    return route.fulfill({json:session});
   });
   const wasm=page.waitForResponse(response=>new URL(response.url()).pathname.endsWith('.wasm')&&response.status()===200);
   await page.goto(server.resolvedUrls.local[0]);
   await wasm;
   await expect(page.locator('html')).toHaveAttribute('data-xcss-runtime','leptos',{timeout:30_000});
   await expect(page.locator('html')).toHaveAttribute('data-theme','light');
   await page.emulateMedia({colorScheme:'dark'});
   await expect(page.locator('html')).toHaveAttribute('data-theme','dark');
   await page.getByRole('button',{name:'切换到浅色模式',exact:true}).click();
   await page.emulateMedia({colorScheme:'light'});
   await page.emulateMedia({colorScheme:'dark'});
   await expect(page.locator('html')).toHaveAttribute('data-theme','light');
   await expect(page.getByRole('heading',{name:'RustDesk 设备管理'})).toBeVisible();
   await page.getByRole('link',{name:device.name,exact:true}).click();
   await expect(page.getByRole('heading',{name:device.name,exact:true})).toBeVisible();
   await expect(page.getByText('等待原生客户端就绪',{exact:true})).toBeVisible();
   await expect(page.getByText('host.example.com:21118',{exact:true})).toBeVisible();
   await page.getByRole('checkbox',{name:controller.name,exact:true}).check();
   await page.getByRole('button',{name:'保存控制权限',exact:true}).click();
   await expect(page.getByRole('status').filter({hasText:'控制权限已保存'})).toBeVisible();
   await page.getByRole('link',{name:'实例',exact:true}).click();
   await page.getByRole('link',{name:controller.name,exact:true}).click();
   await expect(page.getByRole('checkbox',{name:device.name,exact:true})).toBeChecked();
   await expect(page.getByText('控制端已连接',{exact:true})).toBeVisible();
   await page.getByRole('checkbox',{name:device.name,exact:true}).uncheck();
   await page.getByRole('button',{name:'保存控制权限',exact:true}).click();
   await expect(page.getByRole('status').filter({hasText:'控制权限已保存'})).toBeVisible();
   await page.getByRole('link',{name:'实例',exact:true}).click();
   await page.getByRole('link',{name:device.name,exact:true}).click();
   await expect(page.getByRole('checkbox',{name:controller.name,exact:true})).not.toBeChecked();
   await page.getByRole('button',{name:'显示授权码',exact:true}).click();
   await expect(page.getByText('c'.repeat(36),{exact:true})).toBeVisible();
   // A fresh last-seen value must update the device without losing local edits
   // or disclosing/hiding enrollment credentials as a side effect of polling.
   await page.getByLabel('实例名称',{exact:true}).fill('尚未保存的名称');
   device.last_seen_at_micros+=5_000_000;
   const refreshed=page.waitForResponse(response=>new URL(response.url()).pathname==='/api/v1/rustdesk/devices');
   await page.getByRole('button',{name:'刷新',exact:true}).click();
   await refreshed;
   await expect(page.getByLabel('实例名称',{exact:true})).toHaveValue('尚未保存的名称');
   await expect(page.getByText('c'.repeat(36),{exact:true})).toBeVisible();
   await page.getByRole('button',{name:'隐藏授权码',exact:true}).click();
   await page.getByRole('button',{name:'读取主机诊断',exact:true}).click();
   await expect(page.getByText('rustdesk.diagnostics.read',{exact:true})).toBeVisible();assert.equal(submitted,1);
   assert.equal(await page.getByRole('button',{name:/PIN|重启|应用更改/}).count(),0);
   await page.getByRole('combobox',{name:'新实例角色',exact:true}).selectOption('controller');
   await page.getByRole('button',{name:'新建实例',exact:true}).click();
   await expect(page.getByText('b'.repeat(36),{exact:true})).toBeVisible();
   await expect(page.getByRole('button',{name:'读取主机诊断',exact:true})).toBeDisabled();
   assert.deepEqual(errors,[]);
  }finally{await browser.close()}
 }
}finally{await server.close()}
console.log('Embedded RustDesk management browser acceptance passed (Chromium, Firefox).');
