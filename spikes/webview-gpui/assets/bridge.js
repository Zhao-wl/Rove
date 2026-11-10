(() => {
  if(window.__roveBridge) return; window.__roveBridge=true;
  const send = value => chrome.webview.postMessage({version:1,...value});
  chrome.webview.addEventListener('message', event => {
    const m=event.data;
    if(m.version!==1) return;
    if(m.type==='ping') send({type:'pong',seq:m.seq});
    if(m.type==='theme') {document.body.style.background=m.dark?'#20242b':'#fafafa';document.body.style.color=m.dark?'#ddd':'#111';send({type:'themeApplied',seq:m.seq});}
    if(m.type==='scroll') {scrollTo(0,m.y);send({type:'scrollApplied',seq:m.seq,y:scrollY});}
  });
  // Tab is not an AcceleratorKeyPressed event. Only trusted physical events
  // route focus; synthetic key events cannot fabricate acceptance evidence.
  addEventListener('keydown', event => {
    if(event.key !== 'Tab' || !event.isTrusted) return;
    event.preventDefault();
    send({type:'focusExit',shift:event.shiftKey});
  }, true);
  const initialize=()=>{
    for(let i=0;i<100;i++){const s=document.createElement('section');s.dataset.line=i*20;s.textContent=`Block ${i} / source line ${i*20}`;document.querySelector('#content').append(s);}
    send({type:'ready',earlyInjection:!!window.__roveEarly,localImageLoaded:document.querySelector('img').naturalWidth>0,documentScriptBlocked:document.querySelector('#script-status').textContent==='Document script did not run'});
  };
  if(document.readyState==='loading') addEventListener('DOMContentLoaded',initialize); else initialize();
  let queued=false;
  addEventListener('scroll',()=>{if(queued)return;queued=true;requestAnimationFrame(()=>{queued=false;send({type:'scroll',y:scrollY});});});
})();
