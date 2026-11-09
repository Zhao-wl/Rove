(() => {
  const send = value => chrome.webview.postMessage({version:1,...value});
  chrome.webview.addEventListener('message', event => {
    const m=event.data;
    if(m.version!==1) return;
    if(m.type==='ping') send({type:'pong',seq:m.seq});
    if(m.type==='theme') {document.body.style.background=m.dark?'#20242b':'#fafafa';document.body.style.color=m.dark?'#ddd':'#111';send({type:'themeApplied',seq:m.seq});}
    if(m.type==='scroll') {scrollTo(0,m.y);send({type:'scrollApplied',seq:m.seq,y:scrollY});}
  });
  addEventListener('DOMContentLoaded',()=>{
    for(let i=0;i<100;i++){const s=document.createElement('section');s.dataset.line=i*20;s.textContent=`Block ${i} / source line ${i*20}`;document.querySelector('#content').append(s);}
    send({type:'ready',documentScriptBlocked:document.querySelector('#script-status').textContent==='Document script did not run'});
  });
  let queued=false;
  addEventListener('scroll',()=>{if(queued)return;queued=true;requestAnimationFrame(()=>{queued=false;send({type:'scroll',y:scrollY});});});
})();
