#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JsonLinePeer, WorkerJavaScriptSession } from "../node-repl/open-computer-use-repl.mjs";
const [nativePath, app, stateFile] = process.argv.slice(2);
const native = new JsonLinePeer({command:nativePath,args:["mcp"],env:{...process.env,OPEN_COMPUTER_USE_ALLOW_GLOBAL_POINTER_FALLBACKS:"1"}});
let session;
try {
  await native.initialize();
  session = new WorkerJavaScriptSession({native});
  const result = await session.run(`var app=await cua.getApp(${JSON.stringify(app)},{initialState:false});
    await app.getAXState({includeScreenshot:false,emit:false});
    await app.pressKey("ctrl+a",{keyMethod:"global"});
    await app.pressKey("j",{keyMethod:"global"});`,20000);
  assert.ok(!result.isError, result.content.filter(x=>x.type==="text").map(x=>x.text).join("\n"));
  const deadline = Date.now()+8000;
  while(JSON.parse(readFileSync(stateFile,"utf8")).text!=="j") {
    assert.ok(Date.now()<deadline,"Worker global key did not update the real fixture");
    await new Promise(resolve=>setTimeout(resolve,50));
  }
  console.log("Windows Worker JS → native MCP → SendInput → real WPF keyboard update passed.");
}finally {await session?.close();native.close();}
