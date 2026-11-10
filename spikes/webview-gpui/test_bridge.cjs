// Logic-only test: synthetic objects are not physical-key acceptance evidence.
const fs=require('node:fs'), vm=require('node:vm'), assert=require('node:assert/strict');
const handlers={}, sent=[];
const context={window:{},chrome:{webview:{postMessage:m=>sent.push(m),addEventListener(){}}},document:{readyState:'loading'},addEventListener:(name,handler)=>handlers[name]=handler};
vm.runInNewContext(fs.readFileSync(__dirname+'/assets/bridge.js','utf8'),context);
for(const shift of [false,true]) {
 let prevented=false;
 handlers.keydown({key:'Tab',isTrusted:true,shiftKey:shift,preventDefault(){prevented=true;}});
 assert.equal(prevented,true);assert.equal(sent.at(-1).type,'focusExit');assert.equal(sent.at(-1).shift,shift);
}
const count=sent.length;
handlers.keydown({key:'Tab',isTrusted:false});handlers.keydown({key:'x',isTrusted:true});
assert.equal(sent.length,count);
console.log('PASS: Tab/Shift+Tab routing logic, synthetic and non-Tab rejection; physical keys NOT tested');
