import assert from "node:assert/strict";
import {execFile,spawn} from "node:child_process";
import {promisify} from "node:util";
import {mkdtemp,readFile,open,rm} from "node:fs/promises";
import {join,resolve} from "node:path";
import {tmpdir} from "node:os";
import {createServer as httpsServer} from "node:https";
import {request as httpRequest} from "node:http";
import {connect} from "node:net";
import {randomBytes} from "node:crypto";
import {DatabaseSync} from "node:sqlite";
import {chromium,expect} from "@playwright/test";
import {withLocalServer} from "./local-server.mjs";

// Actual browser, Manager, TLS ingress and independent Client process. Only Sunshine is a fixture.
assert.ok(process.env.SUNSHINE_TEST_CLIENT_BINARY, "Set SUNSHINE_TEST_CLIENT_BINARY to an explicitly built xscc executable");
const root=await mkdtemp(join(tmpdir(),"xscc-e2e-"));
const exec=promisify(execFile);
let sunshine,ingress,client,clientLog;
const tunnels=new Set();
const clientBinary=resolve(process.env.SUNSHINE_TEST_CLIENT_BINARY);
async function runWithInput(args,input,env){
 return new Promise((done,fail)=>{
  const child=spawn(clientBinary,args,{env,stdio:["pipe","pipe","pipe"]});let stdout="",stderr="";
  child.stdout.on("data",chunk=>stdout+=chunk);child.stderr.on("data",chunk=>stderr+=chunk);
  const timer=setTimeout(()=>{child.kill("SIGTERM");fail(new Error("Test Client command timed out"))},40000);
  child.once("error",error=>{clearTimeout(timer);fail(error)});child.once("exit",code=>{clearTimeout(timer);code===0?done(stdout):fail(new Error("Client command failed with exit "+code+": "+stderr+stdout))});
  child.stdin.end(input);
 });
}
async function stopClient(){
 if(client&&client.exitCode===null&&client.signalCode===null){
  const stopped=new Promise((done,fail)=>{const timer=setTimeout(()=>fail(new Error("Test Client failed to stop; state retained at "+root)),10000);client.once("exit",()=>{clearTimeout(timer);done()})});
  client.kill("SIGTERM");await stopped;
 }
}
async function startClient(state){
 client=spawn(clientBinary,["run","--state",state],{env:{...process.env,SSL_CERT_FILE:join(root,"ca.crt")},stdio:["ignore",clientLog.fd,clientLog.fd]});
 await new Promise((done,fail)=>{client.once("spawn",done);client.once("error",fail)});
}
try {
 const caKey=join(root,"ca.key"),caCert=join(root,"ca.crt"),key=join(root,"server.key"),csr=join(root,"server.csr"),cert=join(root,"server.crt"),sunshineKey=join(root,"sunshine.key"),sunshineCert=join(root,"sunshine-cacert.pem");
 await exec("openssl",["req","-x509","-newkey","rsa:2048","-nodes","-keyout",caKey,"-out",caCert,"-days","1","-subj","/CN=Client test CA","-addext","basicConstraints=critical,CA:TRUE"]);
 await exec("openssl",["req","-new","-newkey","rsa:2048","-nodes","-keyout",key,"-out",csr,"-subj","/CN=localhost","-addext","subjectAltName=DNS:localhost,IP:127.0.0.1","-addext","basicConstraints=critical,CA:FALSE","-addext","extendedKeyUsage=serverAuth"]);
 await exec("openssl",["x509","-req","-in",csr,"-CA",caCert,"-CAkey",caKey,"-CAcreateserial","-out",cert,"-days","1","-copy_extensions","copy"]);
 const tls={key:await readFile(key),cert:await readFile(cert)};
 await exec("openssl",["req","-x509","-newkey","rsa:2048","-nodes","-keyout",sunshineKey,"-out",sunshineCert,"-days","1","-subj","/CN=Sunshine Gamestream Host"]);
 const sunshineTls={key:await readFile(sunshineKey),cert:await readFile(sunshineCert)};
 let config={sunshine_name:"Original",qp:"28",output_name:"DISPLAY1",custom_setting:"preserved","global_prep_cmd":"LOCAL_ONLY_SECRET_COMMAND"};
 const pendingPairing={id:"5".repeat(32),name:"测试 Moonlight",address:"192.0.2.44"};
 let pendingPairings=[pendingPairing],pairedClients=[],pinSubmissions=[];
 let writes=0,restarts=0;let loseRestartReceipt=false;let loseNextAck=false;let droppedAcks=0;let delaySaveReceipt=false;let releaseSaveReceipt;
 sunshine=httpsServer(sunshineTls,async(req,res)=>{
  const json=value=>{res.setHeader("content-type","application/json");res.end(JSON.stringify(value))};
  if(req.headers.authorization!=="Basic "+Buffer.from("fixture:local-only-password").toString("base64")){res.statusCode=401;return json({status:false});}
  if(req.method==="GET"&&req.url==="/api/config")return json({status:true,platform:"linux",version:"2026.914.233613",...config});
  if(req.method==="POST"&&req.url==="/api/config"){
   let body="";for await(const chunk of req)body+=chunk;
   config=JSON.parse(body);writes++;if(delaySaveReceipt){delaySaveReceipt=false;releaseSaveReceipt=()=>json({status:true});return;}return json({status:true});
  }
  if(req.method==="GET"&&req.url==="/api/pin")return json({pairings:pendingPairings});
  if(req.method==="GET"&&req.url==="/api/clients/list")return json({status:true,named_certs:pairedClients});
  if(req.method==="POST"&&req.url==="/api/pin"){
   let body="";for await(const chunk of req)body+=chunk;
   const request=JSON.parse(body);pinSubmissions.push(request);
   if(request.pairing_id!==pendingPairing.id||request.pin!=="1234")return json({status:false});
   pendingPairings=[];pairedClients=[{uuid:"123e4567-e89b-12d3-a456-426614174000",name:request.name,enabled:true}];return json({status:true});
  }
  if(req.method==="POST"&&req.url==="/api/restart"){restarts++;if(loseRestartReceipt)return res.destroy();return json({status:true});}
  res.statusCode=404;json({status:false});
 });
 await new Promise(done=>sunshine.listen(0,"127.0.0.1",done));
 await withLocalServer({prefix:"XSCS",binary:process.env.XSCS_TEST_SERVER_BINARY??"../target/x86_64-unknown-linux-gnu/debug/xscs",extraEnv:{XSCS_DEVELOPMENT:"true"}},async({base,password,database})=>{
  const target=new URL(base);
  ingress=httpsServer(tls,(req,res)=>{
   const upstream=httpRequest(base+req.url,{method:req.method,headers:{...req.headers,"x-forwarded-proto":"https"}},reply=>{res.writeHead(reply.statusCode,reply.headers);reply.pipe(res)});
   upstream.on("error",()=>{res.statusCode=502;res.end()});req.pipe(upstream);
  });
  ingress.on("upgrade",(req,socket,head)=>{
   const upstream=connect(Number(target.port),"127.0.0.1",()=>{
    let headers=req.method+" "+req.url+" HTTP/1.1\r\n";
    for(const [name,value] of Object.entries({...req.headers,"x-forwarded-proto":"https"}))headers+=name+": "+value+"\r\n";
    upstream.write(headers+"\r\n");if(head.length)upstream.write(head);socket.pipe(upstream);
    upstream.on("data",bytes=>{
     if(loseNextAck&&bytes.includes(Buffer.from('"type":"result_accepted"'))){loseNextAck=false;droppedAcks++;socket.destroy();upstream.destroy();return;}
     socket.write(bytes);
    });
   });
   tunnels.add(socket);socket.on("close",()=>{tunnels.delete(socket);upstream.destroy()});
   upstream.on("end",()=>socket.end());upstream.on("close",()=>socket.destroy());
   socket.on("error",()=>upstream.destroy());upstream.on("error",()=>socket.destroy());
  });
  await new Promise(done=>ingress.listen(0,"127.0.0.1",done));
  const browser=await chromium.launch();const errors=[];
  try {
   const page=await browser.newPage({ locale: "zh-CN" });page.setDefaultTimeout(20000);page.setDefaultNavigationTimeout(20000);page.on("pageerror",e=>errors.push(e.message));
   await page.goto(base);await page.getByLabel("用户名",{exact:true}).fill("admin");await page.getByLabel("密码",{exact:true}).fill(password);
   await page.getByRole("button",{name:"登录",exact:true}).click();
   const created=page.waitForResponse(r=>r.url().endsWith("/sunshine/devices")&&r.request().method()==="POST");
   await page.getByRole("button",{name:"新建实例",exact:true}).click();
   assert.equal((await created).status(),201);
   await expect(page.getByRole("link",{name:"选择实例 新实例",exact:true})).toBeVisible();
   const createdDevices=await(await page.request.get(base+"/api/v1/sunshine/devices")).json();
   assert.equal(createdDevices.length,1);const id=createdDevices[0].id;const api="/api/v1/sunshine/devices/"+id;
   // Recover the durable account through its API, independently of DevTools response-body retention.
   const authorization=await(await page.request.get(base+api+"/authorization")).json();
   const ticket={device:createdDevices[0],token:authorization.authorization_code};
   console.log("e2e: isolated Manager instance created");
   await expect(page.getByRole("dialog")).toHaveCount(0);
   await page.getByRole("button",{name:"详细信息",exact:true}).click();
   await page.getByLabel("实例名称",{exact:true}).fill("Client 闭环测试");
   await page.getByRole("button",{name:"保存名称",exact:true}).click();
   const state=join(root,"client");
   const pairingInput=JSON.stringify({server:"https://127.0.0.1:"+ingress.address().port+"/",authorization_code:ticket.token,sunshine_endpoint:"https://127.0.0.1:"+sunshine.address().port+"/",sunshine_username:"fixture",sunshine_password:"local-only-password"});
   const pairArgs=["pair","--state",state,"--input-stdin","--non-interactive","--format","json"];
   // The CLI must reject an untrusted Manager before enrolling the durable client credential.
   await assert.rejects(runWithInput(pairArgs,pairingInput,{...process.env,SSL_CERT_FILE:"/nonexistent-test-trust-store"}));
   await runWithInput(pairArgs,pairingInput,{...process.env,SSL_CERT_FILE:caCert});
   console.log("e2e: Client paired over verified TLS");
   clientLog=await open(join(root,"client.log"),"wx",0o600);await startClient(state);
   const device=async()=>{const r=await page.request.get(base+"/api/v1/sunshine/devices");return(await r.json()).find(d=>d.id===id)};
   await expect.poll(async()=>(await device()).client_online,{timeout:20000}).toBe(true);
   await expect.poll(async()=>(await device()).sunshine_reachable,{timeout:20000}).toBe(true);
   await page.getByRole("button",{name:"刷新",exact:true}).click();
   await expect(page.getByRole("button",{name:"详细信息",exact:true})).toHaveAttribute("aria-pressed","true");
   await page.getByRole("navigation",{name:"Sunshine 配置分类"}).getByRole("button",{name:"概况",exact:true}).click();
   await expect(page.getByLabel("Sunshine 名称",{exact:true})).toHaveValue("Original");
   // Change local managed settings after opening; save must overwrite them with the frozen page snapshot.
   config.qp="49";config.keyboard="true";config.custom_setting="latest local setting";
   await page.getByLabel("Sunshine 名称", { exact: true }).fill("网页保存");
   await page.getByRole("button",{name:"预览变更",exact:true}).click();
   const queued=page.waitForResponse(r=>r.url().endsWith("/tasks")&&r.request().method()==="POST");
   await page.getByRole("button",{name:"应用更改",exact:true}).click();
   const operation=await(await queued).json();
   assert.equal((await queued).request().postDataJSON().kind,"save_config");
   assert.ok(!("expected_revision" in (await queued).request().postDataJSON()));
   const getOperation=async id=>(await(await page.request.get(base+"/api/v1/sunshine/operations/"+id)).json());
   await expect.poll(async()=>(await getOperation(operation.operation_id)).state,{timeout:20000}).toBe("succeeded");
   assert.equal(writes,1);assert.equal(restarts,0);assert.equal(config.sunshine_name,"网页保存");assert.equal(config.output_name,"DISPLAY1");assert.equal(config.custom_setting,"latest local setting");assert.equal(config.qp,"28");assert.ok(!("keyboard" in config));assert.equal(config.global_prep_cmd,"LOCAL_ONLY_SECRET_COMMAND");assert.ok(!("status" in config));
   const saved=await device();assert.equal(saved.configuration_state,"awaiting_restart");
   assert.ok(!JSON.stringify(saved).includes("LOCAL_ONLY_SECRET_COMMAND"));
   const executionRecord=async id=>JSON.parse(await readFile(join(state,"journal",id+".json"),"utf8"));
   await expect.poll(async()=>(await executionRecord(operation.operation_id)).acknowledged,{timeout:10000}).toBe(true);
   const backup=JSON.parse(await readFile(join(state,"config-backups","latest.json"),"utf8"));
   assert.equal(backup.configuration.global_prep_cmd,"LOCAL_ONLY_SECRET_COMMAND");
   assert.equal(backup.configuration.sunshine_name,"Original");
   assert.equal(backup.configuration.qp,"49");assert.equal(backup.configuration.keyboard,"true");
   console.log("e2e: frozen browser snapshot overwrite, local settings, protected backup and durable receipt verified");
   await expect(page.getByLabel("配置操作状态",{exact:true}).filter({visible:true})).toHaveText("配置已保存，等待管理员重启。",{timeout:10000});
   await expect(page.getByRole("button",{name:"应用更改",exact:true})).toHaveCount(0);
   const categories=page.getByRole("navigation",{name:"Sunshine 配置分类"});
   await categories.getByRole("button",{name:"Moonlight 配对",exact:true}).click();
   await expect(page.getByRole("combobox",{name:"待配对客户端",exact:true})).toContainText(pendingPairing.name);
   await page.getByLabel("PIN（4 位数字）",{exact:true}).fill("1234");
   const pinQueued=page.waitForResponse(r=>r.url().endsWith("/tasks")&&r.request().method()==="POST"&&r.request().postDataJSON()?.kind==="submit_pairing_pin");
   await page.getByRole("button",{name:"配对",exact:true}).click();
   const pinOperation=await(await pinQueued).json();
   await expect.poll(async()=>(await getOperation(pinOperation.operation_id)).state,{timeout:20000}).toBe("succeeded");
   await expect(page.getByText("Moonlight 已配对。",{exact:true})).toBeVisible();
   await expect(page.getByRole("heading",{name:pendingPairing.name,exact:true})).toBeVisible({timeout:20000});
   await expect(page.getByText("暂无配对请求，请先在 Moonlight 中发起配对。",{exact:true})).toBeVisible();
   assert.deepEqual(pinSubmissions,[{pairing_id:pendingPairing.id,pin:"1234",name:pendingPairing.name}]);
   await expect.poll(async()=>(await executionRecord(pinOperation.operation_id)).acknowledged,{timeout:10000}).toBe(true);
   assert.equal(writes,1);
   await categories.getByRole("button",{name:"概况",exact:true}).click();
   console.log("e2e: browser Moonlight selection, PIN, local HTTPS and paired-list refresh verified");
   // Offline submission is durable, and a fresh Client process reuses its registered identity.
   await stopClient();
   const csrf=(await queued).request().headers()["x-csrf-token"];
   const submit=async command=>{const r=await page.request.post(base+api+"/tasks",{headers:{"Origin":base,"Sec-Fetch-Site":"same-origin","x-csrf-token":csrf,"Idempotency-Key":randomBytes(16).toString("hex")},data:command});assert.equal(r.status(),202,"Task admission: "+(r.ok()?"ok":(await r.json()).code));return r.json()};
   const offline=await submit({kind:"read_config"});assert.equal(offline.state,"pending");
   await startClient(state);
   await expect.poll(async()=>(await getOperation(offline.operation_id)).state,{timeout:20000}).toBe("succeeded");
   assert.equal((await device()).configuration_state,"awaiting_restart","a read must not clear awaiting_restart");
   // Lose the Manager's durable receipt. Reconnect must replay the result, never repeat the POST.
   loseNextAck=true;
   const lostAck=await submit({kind:"patch_config",expected_revision:(await device()).snapshot.revision,set:{qp:29},remove:[],restart_policy:"manual"});
   await expect.poll(async()=>(await getOperation(lostAck.operation_id)).state,{timeout:20000}).toBe("succeeded");
   await expect.poll(()=>droppedAcks,{timeout:20000}).toBe(1);
   await expect.poll(async()=>(await executionRecord(lostAck.operation_id)).acknowledged,{timeout:20000}).toBe(true);
   assert.equal(writes,2);
   console.log("e2e: offline delivery and lost result receipt recovered");
   // Cut WSS while Sunshine's write reply is delayed; keep the same Client process alive.
   delaySaveReceipt=true;
   const disconnectedSave=await submit({kind:"patch_config",expected_revision:(await device()).snapshot.revision,set:{qp:30},remove:[],restart_policy:"manual"});
   await expect.poll(()=>typeof releaseSaveReceipt,{timeout:20000}).toBe("function");
   for(const socket of tunnels)socket.destroy();
   await expect.poll(async()=>(await getOperation(disconnectedSave.operation_id)).state,{timeout:10000}).toBe("unknown");
   releaseSaveReceipt();
   await expect.poll(async()=>(await getOperation(disconnectedSave.operation_id)).reconciliation?.kind,{timeout:20000}).toBe("config_saved");
   await expect.poll(async()=>(await executionRecord(disconnectedSave.operation_id)).acknowledged,{timeout:20000}).toBe(true);
   assert.equal(writes,3);assert.equal(config.qp,"30");
   assert.equal((await device()).configuration_state,"awaiting_restart");
   const resolved=await page.request.post(base+"/api/v1/sunshine/operations/"+disconnectedSave.operation_id+"/resolve",{headers:{"Origin":base,"Sec-Fetch-Site":"same-origin","x-csrf-token":csrf},data:{resolution:"confirmed_succeeded"}});
   assert.equal(resolved.status(),200);
   console.log("e2e: WSS loss during POST retained execution without another write");
   // A changed unmanaged field must also invalidate the full configuration revision.
   const revision=(await device()).snapshot.revision;config.custom_setting="local edit";
   const conflict=await submit({kind:"patch_config",expected_revision:revision,set:{qp:30},remove:[],restart_policy:"manual"});
   await expect.poll(async()=>(await getOperation(conflict.operation_id)).state,{timeout:10000}).toBe("failed");
   assert.equal((await getOperation(conflict.operation_id)).result.kind,"conflict");assert.equal(writes,3);
   const read=await submit({kind:"read_config"});
   await expect.poll(async()=>(await getOperation(read.operation_id)).state,{timeout:10000}).toBe("succeeded");
   const current=(await getOperation(read.operation_id)).result.snapshot.revision;
   loseRestartReceipt=true;
   const restart=await submit({kind:"restart",expected_revision:current,administrator_confirmed:true});
   await expect.poll(async()=>(await getOperation(restart.operation_id)).state,{timeout:20000}).toBe("unknown");
   assert.equal(restarts,1);
   await stopClient();await startClient(state);
   await expect.poll(async()=>(await getOperation(restart.operation_id)).reconciliation?.kind,{timeout:20000}).toBe("unknown");
   assert.equal(restarts,1);
   const resolveRestart=await page.request.post(base+"/api/v1/sunshine/operations/"+restart.operation_id+"/resolve",{headers:{"Origin":base,"Sec-Fetch-Site":"same-origin","x-csrf-token":csrf},data:{resolution:"unable_to_confirm"}});
   assert.equal(resolveRestart.status(),200);
   await expect.poll(async()=>(await executionRecord(restart.operation_id)).acknowledged,{timeout:25000}).toBe(true);
   assert.equal(restarts,1,"final receipt compacts uncertainty without repeating restart");
   console.log("e2e: restart uncertainty and human resolution receipt verified");
   const revoked=await page.request.post(base+api+"/revoke",{headers:{"Origin":base,"Sec-Fetch-Site":"same-origin","x-csrf-token":csrf}});assert.equal(revoked.status(),204);
   await expect.poll(()=>client.exitCode,{timeout:10000}).not.toBe(null);
   assert.equal((await device()).client_online,false);
   const db=new DatabaseSync(database,{readOnly:true});
   const audited=db.prepare("SELECT count(*) AS count FROM audit_logs WHERE action='operation.reconciled'").get();assert.ok(audited.count>=1);
   assert.equal(db.prepare("SELECT count(*) AS count FROM sqlite_schema WHERE name='hosts'").get().count,0);db.close();
   const log=await readFile(join(root,"client.log"),"utf8");assert.ok(!log.includes("local-only-password")&&!log.includes(ticket.token));
   await expect(page.locator("body")).not.toContainText("local-only-password");assert.deepEqual(errors,[]);
   console.log("PASS: browser → durable Manager operation → verified WSS → independent Client → pinned built-in-style Sunshine certificate → merged local HTTPS save → audit; offline delivery, durable ACK loss/replay, WSS loss during a write without cancellation/reexecution, protected config backup, full-revision conflict, uncertain restart/reconnect and revocation");
  }finally{await stopClient();await browser.close();for(const socket of tunnels)socket.destroy();ingress.closeAllConnections();await new Promise(done=>ingress.close(done));ingress=undefined;}
 });
}finally{
 await stopClient();await clientLog?.close();if(sunshine){sunshine.closeAllConnections();await new Promise(done=>sunshine.close(done));}
 // Only this test-created directory is removed, after every child process has stopped.
 await rm(root,{recursive:true});
}
