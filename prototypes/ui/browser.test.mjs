import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {mkdir,writeFile,rm} from "node:fs/promises";
import path from "node:path";
import {DESIGNS} from "./model.mjs";

const url=process.env.TC_UI_URL??"http://127.0.0.1:4177";
const artifacts=process.env.TC_UI_ARTIFACTS;
if (!artifacts) throw new Error("Set TC_UI_ARTIFACTS to the session evidence directory.");
await mkdir(artifacts,{recursive:true});
const profile=path.join(artifacts,"ui-01-browser-profile");
const browser=spawn(process.env.TC_UI_BROWSER??"C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  ["--headless=new","--disable-gpu","--no-first-run","--no-default-browser-check","--remote-debugging-port=0",`--user-data-dir=${profile}`,"about:blank"],{stdio:["ignore","ignore","pipe"]});
let socket;
const report={runId:"ui-01-round-01",time:new Date().toISOString(),checks:[],screenshots:[],errors:[],requests:[]};
const pending=new Map();
let nextId=0,sessionId;
function command(method,params={},session=sessionId) {
  return new Promise((resolve,reject)=>{
    const id=++nextId;
    const timeout=setTimeout(()=>{pending.delete(id);reject(new Error(`CDP timeout: ${method}`));},15000);
    pending.set(id,{resolve:(value)=>{clearTimeout(timeout);resolve(value);},reject:(error)=>{clearTimeout(timeout);reject(error);}});
    socket.send(JSON.stringify({id,method,params,...(session?{sessionId:session}:{})}));
  });
}
async function evaluate(expression) {
  const result=await command("Runtime.evaluate",{expression,awaitPromise:true,returnByValue:true});
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text+": "+(result.exceptionDetails.exception?.description??""));
  return result.result.value;
}
const settle = () => evaluate("new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))");
async function click(selector) {
  await evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  await settle();
}
async function focus(selector) {
  await settle();
  await evaluate(`document.querySelector(${JSON.stringify(selector)}).focus()`);
  await settle();
}
async function type(selector,value) {
  await evaluate(`{ const input=document.querySelector(${JSON.stringify(selector)}); input.value=${JSON.stringify(value)};input.dispatchEvent(new Event('input',{bubbles:true})); }`);
}
async function key(key,code,modifiers=0) {
  const virtual={Escape:27,Enter:13,Tab:9,ArrowDown:40,ArrowUp:38,ArrowLeft:37,ArrowRight:39,Home:36,End:35,Delete:46,Backspace:8," ":32}[key]??key.toUpperCase().charCodeAt(0);
  await command("Input.dispatchKeyEvent",{type:"keyDown",key,code,modifiers,windowsVirtualKeyCode:virtual,...(key==="Enter"?{text:"\r",unmodifiedText:"\r"}:{})});
  await command("Input.dispatchKeyEvent",{type:"keyUp",key,code,modifiers,windowsVirtualKeyCode:virtual});
  await evaluate("new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))");
}
async function mouse(selector) {
  const rect=await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`);
  await command("Input.dispatchMouseEvent",{type:"mousePressed",...rect,button:"left",clickCount:1});
  await command("Input.dispatchMouseEvent",{type:"mouseReleased",...rect,button:"left",clickCount:1});
}
const close = () => click("#modal-close");
const accentShades={blue:{light:"#006da0",dark:"#00a4ef"},red:{light:"#bc3a16",dark:"#f25022"},green:{light:"#4c7100",dark:"#7fba00"},yellow:{light:"#805b00",dark:"#ffb900"}};
async function record(name,fn) {
  await fn();
  report.checks.push({name,result:"PASS",time:new Date().toISOString()});
}
try {
  const endpoint=await new Promise((resolve,reject)=>{
    let output="";
    const timeout=setTimeout(()=>reject(new Error("Browser debugging endpoint unavailable")),20000);
    browser.stderr.on("data",chunk=>{
      output+=chunk;
      const match=output.match(/DevTools listening on (ws:\/\/[^\s]+)/);
      if (match) {clearTimeout(timeout);resolve(match[1]);}
    });
    browser.on("error",reject);
    browser.on("exit",code=>{clearTimeout(timeout);reject(new Error(`Browser exited before readiness: ${code}`));});
  });
  socket=new WebSocket(endpoint);
  await new Promise((resolve,reject)=>{socket.addEventListener("open",resolve,{once:true});socket.addEventListener("error",reject,{once:true});});
  socket.addEventListener("message",event=>{
    const message=JSON.parse(event.data);
    if (message.id) {
      const item=pending.get(message.id);
      if (!item) return;
      pending.delete(message.id);
      if (message.error) item.reject(new Error(message.error.message)); else item.resolve(message.result);
    } else if (message.method==="Runtime.exceptionThrown") report.errors.push(message.params.exceptionDetails);
    else if (message.method==="Network.requestWillBeSent") report.requests.push(message.params.request.url);
  });
  const target=await command("Target.createTarget",{url:"about:blank"});
  sessionId=(await command("Target.attachToTarget",{targetId:target.targetId,flatten:true})).sessionId;
  await command("Runtime.enable");
  await command("Page.enable");
  await command("Network.enable");
  await command("Page.navigate",{url});
  for (let attempt=0;attempt<60;attempt++) {
    if (await evaluate(`document.querySelectorAll('#designs button').length===5`)) break;
    await new Promise(resolve=>setTimeout(resolve,100));
    if (attempt===59) throw new Error("UI did not become ready.");
  }
  for (const design of DESIGNS) {
    await click(`[data-design-choice="${design.id}"]`);
    await record(`${design.id}: keyboard selection, navigation and palette`,async()=>{
      await click('[data-command="reset"]');
      await focus("#list-0");
      await key("ArrowDown","ArrowDown");
      await key(" ","Space");
      assert.equal(await evaluate(`document.querySelectorAll('#pane-left [data-selected="true"]').length`),1);
      await key("Tab","Tab");
      assert.equal(await evaluate("document.activeElement.id"),"list-1");
      await key("Tab","Tab");
      await key("P","KeyP",10);
      assert.equal(await evaluate("document.querySelector('#modal').open"),true);
      await key("Escape","Escape");
      assert.equal(await evaluate("document.querySelector('#modal').open"),false);
      assert.equal(await evaluate("document.activeElement.id"),"list-0");
      await key("ArrowRight","ArrowRight",10);
      assert.equal(await evaluate("document.querySelector('#divider').getAttribute('aria-valuenow')"),"55");
      await click('[data-command="reset"]');
    });
    await record(`${design.id}: filter, sort, favorites and pin`,async()=>{
      await type("#filter-0","roadmap");
      assert.equal(await evaluate("document.querySelectorAll('#list-0 [data-row]').length"),1);
      assert.equal(await evaluate("document.activeElement.id"),"filter-0");
      assert.equal(await evaluate("document.querySelectorAll('#pane-left [data-selected=\"true\"]').length"),0,"Selection before input shortcut");
      await key("a","KeyA",2);
      assert.equal(await evaluate("document.querySelectorAll('#pane-left [data-selected=\"true\"]').length"),0);
      await type("#filter-0","");
      await evaluate(`{const select=document.querySelector('#sort-0');select.value='size';select.dispatchEvent(new Event('change',{bubbles:true}));}`);
      await click('[data-command="favorites"][data-pane="0"]');
      await click('[data-favorite="C:\\\\Demo\\\\Pictures"]');
      assert.match(await evaluate("document.querySelector('#pane-left .path').textContent"),/Pictures/);
      await focus("#list-0");await key("Enter","Enter");
      await click('[data-command="pin"][data-pane="0"]');
      await click('[data-command="favorites"][data-pane="0"]');
      assert.equal(await evaluate(`document.querySelectorAll('[data-favorite]').length`),5);
      await close();await click('[data-command="reset"]');
    });
    await record(`${design.id}: operation cancellation and approved effects`,async()=>{
      await focus("#list-0");await key("Delete","Delete");
      assert.equal(await evaluate("document.activeElement.id"),"cancel-plan");
      await key("Enter","Enter");
      assert.equal(await evaluate("document.querySelector('#modal').open"),false);
      assert.equal(await evaluate("document.querySelectorAll('#list-0 [data-row]').length"),9);
      await focus("#list-0");await key("C","KeyC",10);
      await click("#approve-plan");
      assert.equal(await evaluate("document.querySelectorAll('#list-1 [data-row]').length"),4);
      assert.equal(await evaluate("document.querySelectorAll('#list-0 [data-row]').length"),9);
      await focus("#list-0");await key("C","KeyC",10);
      assert.equal(await evaluate("document.querySelector('#modal-title').textContent"),"No changes made");
      await close();await click('[data-command="reset"]');
      await focus("#list-0");await key("M","KeyM",10);await click("#approve-plan");
      assert.equal(await evaluate("document.querySelectorAll('#list-0 [data-row]').length"),8);
      await focus("#list-1");await key("Delete","Delete");await click("#approve-plan");
      assert.equal(await evaluate("document.querySelectorAll('#list-1 [data-row]').length"),3);
      await click('[data-command="reset"]');
    });
    await record(`${design.id}: mock search, AI example and intent`,async()=>{
      await click('[data-command="search"]');
      await type("#search-input","Penta");
      assert.equal(await evaluate("document.querySelectorAll('[data-search-result]').length"),1);
      await type("#search-input","not-found");
      assert.equal(await evaluate("document.querySelectorAll('[data-search-result]').length"),0);
      await click("#ai-search");
      await new Promise(resolve=>setTimeout(resolve,600));
      assert.equal(await evaluate("document.querySelectorAll('[data-search-result]').length"),3);
      await click('[data-search-result="0"]');
      assert.match(await evaluate("document.querySelector('#pane-left .path').textContent"),/Srbsko/);
      await click('[data-command="palette"]');
      await type("#palette-input","Copy my Penta hackathon presentation to Kosik");
      await key("Enter","Enter");
      assert.match(await evaluate("document.querySelector('.plan').textContent"),/Penta/);
      await click("#approve-plan");
      assert.match(await evaluate("document.querySelector('#pane-right').textContent"),/Penta-hackathon/);
      await click('[data-command="reset"]');
    });
    await record(`${design.id}: mouse row selection and divider`,async()=>{
      await mouse('#row-0-1 input');
      assert.equal(await evaluate("document.querySelectorAll('#pane-left [data-selected=\"true\"]').length"),1);
      await focus("#divider");await key("ArrowLeft","ArrowLeft");
      assert.equal(await evaluate("document.querySelector('#divider').getAttribute('aria-valuenow')"),"45");
      await click('[data-command="reset"]');
    });
    await record(`${design.id}: additional command and dialog states`,async()=>{
      for (const [id,title] of [["code","Open in VS Code"],["copilot","Start Copilot session"],["answer","A compact answer, with sources"],["voice","Review a transcript"]]) {
        await click('[data-command="palette"]');
        await click(`[data-palette-command="${id}"]`);
        assert.equal(await evaluate("document.querySelector('#modal-title').textContent"),title);
        assert.match(await evaluate("document.querySelector('#modal-label').textContent"),/PREVIEW|EXAMPLE/);
        await close();
      }
      await click('[data-command="palette"]');
      await click('[data-palette-command="sort"]');
      await evaluate("new Promise(resolve=>requestAnimationFrame(resolve))");
      assert.equal(await evaluate("document.activeElement.id"),"sort-0");
      await click('[data-command="palette"]');
      await click('[data-palette-command="balance"]');
      assert.equal(await evaluate("document.querySelector('#divider').getAttribute('aria-valuenow')"),"50");
      await click('[data-command="favorites"][data-pane="1"]');
      await click('[data-favorite="C:\\\\Demo\\\\Pictures"]');
      await focus("#list-1");await key("Backspace","Backspace");
      await evaluate(`document.querySelector('#pane-right [data-row="6"]').dispatchEvent(new MouseEvent('click',{bubbles:true}))`);
      await key("Enter","Enter");
      await focus("#list-0");
      await key("C","KeyC",10);
      assert.match(await evaluate("document.querySelector('#modal-body').textContent"),/permission denied/);
      await close();await click('[data-command="reset"]');
    });
    for (const [width,height] of [[1366,768],[1920,1080]]) {
      await command("Emulation.setDeviceMetricsOverride",{width,height,deviceScaleFactor:1,mobile:false});
      for (const accent of Object.keys(accentShades)) {
        for (let attempt=0;attempt<4;attempt++) {
          if (await evaluate("document.documentElement.dataset.accent")===accent) break;
          await click("#accent");
        }
        for (const theme of ["dark","light"]) {
          if (await evaluate("document.documentElement.dataset.theme")!==theme) await click("#theme");
          await record(`${design.id}: ${theme}/${accent} layout ${width}x${height}`,async()=>{
          assert.equal(await evaluate("document.documentElement.dataset.accent"),accent,"Theme changed accent family");
          assert.equal(await evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()"),accentShades[accent][theme]);
          const overflow=await evaluate(`document.documentElement.scrollWidth>innerWidth`);
          assert.equal(overflow,false,"Horizontal page overflow");
          const bad=await evaluate(`Array.from(document.querySelectorAll('.lab button,.app-header button,#file-actions button,.pane-tools button,.prompt-dock button,.statusbar')).filter(el=>{const r=el.getBoundingClientRect();return r.width&& (r.left<0||r.right>innerWidth||r.bottom>innerHeight);}).map(el=>({text:el.textContent,rect:el.getBoundingClientRect().toJSON(),viewport:[innerWidth,innerHeight]}))`);
          assert.deepEqual(bad,[],"Clipped visible controls");
          });
          const filename=`ui-01-${design.id}-${theme}-${accent}-${width}.png`;
          const screenshot=await command("Page.captureScreenshot",{format:"png",captureBeyondViewport:false});
          await writeFile(path.join(artifacts,filename),Buffer.from(screenshot.data,"base64"));
          report.screenshots.push(filename);
        }
      }
    }
  }
  await record("Feedback local persistence",async()=>{
    await click("#feedback-open");
    await type("#design-notes","Automated test note: keep the pane clarity.");
    assert.match(await evaluate("localStorage.getItem('tc-ui-notes')"),/pane clarity/);
    await click("#prefer-design");
    assert.equal(await evaluate("JSON.parse(localStorage.getItem('tc-ui-notes')).minimal.preferred"),true);
    await close();
    await evaluate("localStorage.removeItem('tc-ui-notes')");
  });
  await record("No runtime exceptions",async()=>assert.deepEqual(report.errors,[]));
  await record("No external requests",async()=>assert.ok(report.requests.every(request=>request.startsWith(url+"/") || request==="about:blank")));
  console.log(`${report.checks.length} browser checks passed; ${report.screenshots.length} screenshots saved.`);
} catch(error) {
  report.errors.push({message:error.message,stack:error.stack});
  console.error(error);
  process.exitCode=1;
} finally {
  await writeFile(path.join(artifacts,"ui-01-browser-results.json"),JSON.stringify(report,null,2));
  if (socket?.readyState===WebSocket.OPEN) {
    try {await command("Browser.close",{},undefined);} catch(error) {console.error(`Browser close: ${error.message}`);}
    socket.close();
  }
  if (browser.exitCode===null) {
    await Promise.race([new Promise(resolve=>browser.once("exit",resolve)),new Promise(resolve=>setTimeout(resolve,3000))]);
    if (browser.exitCode===null) browser.kill();
  }
  await rm(profile,{recursive:true,force:true,maxRetries:20,retryDelay:500});
}
