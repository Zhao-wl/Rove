// Rove Markdown 预览的可信桥接脚本（#29）。随 Rove 打包，只经带 nonce 的 <script> 加载；
// 文档内容来自宿主净化后的 HTML，文档自带的脚本一律不会执行（页面 CSP 只放行带 nonce 的脚本）。
//
// 协议 {version:1, type, ...}：
//   宿主 → 页面：render、theme、scrollTo、selection、fragment
//   页面 → 宿主：ready、rendered、scroll、dblclick、link、focusExit
(function () {
  'use strict';
  const VERSION = 1;
  const nonce = document.currentScript ? document.currentScript.nonce : '';
  const content = document.getElementById('rove-content');
  let documentId = null;
  let seq = 0;
  let themeKind = 'vscode-dark';
  let mathMacros = {};
  let ignoreScroll = 0;
  let lines = null;
  let scrollPending = false;
  const loaded = {};

  function post(message) {
    message.version = VERSION;
    try {
      window.chrome.webview.postMessage(message);
    } catch (e) {
      // 宿主不在（例如直接用浏览器打开页面调试）。
    }
  }

  // 按需加载打包的库：Mermaid 与 KaTeX 体积较大，只在文档用到时才加载。
  function load(name) {
    if (!loaded[name]) {
      loaded[name] = new Promise(function (resolve, reject) {
        const script = document.createElement('script');
        script.nonce = nonce;
        script.src = 'https://rove.local/' + name;
        script.onload = function () { resolve(); };
        script.onerror = function () { reject(new Error(name)); };
        document.head.appendChild(script);
      });
    }
    return loaded[name];
  }

  // ------------------------------------------------------------------ 源码行映射

  function codeLines() {
    if (lines) {
      return lines;
    }
    lines = [];
    const elements = content.querySelectorAll('.code-line[data-line]');
    for (let i = 0; i < elements.length; i++) {
      const element = elements[i];
      const line = Number(element.getAttribute('data-line'));
      if (isNaN(line) || element.offsetParent === null && element.tagName !== 'BODY') {
        continue;
      }
      lines.push({ element: element, line: line });
    }
    lines.sort(function (a, b) { return a.line - b.line; });
    return lines;
  }

  function topOf(element) {
    return element.getBoundingClientRect().top + window.scrollY;
  }

  function offsetForLine(line) {
    const all = codeLines();
    if (all.length === 0) {
      return 0;
    }
    let previous = null;
    let next = null;
    for (let i = 0; i < all.length; i++) {
      if (all[i].line <= line) {
        previous = all[i];
      } else {
        next = all[i];
        break;
      }
    }
    if (!previous) {
      return 0;
    }
    const previousTop = topOf(previous.element);
    if (next && next.line > previous.line) {
      const progress = (line - previous.line) / (next.line - previous.line);
      return previousTop + progress * (topOf(next.element) - previousTop);
    }
    const height = previous.element.getBoundingClientRect().height;
    return previousTop + height * Math.min(1, Math.max(0, line - previous.line));
  }

  function lineForOffset(offset) {
    const all = codeLines();
    if (all.length === 0) {
      return 0;
    }
    let low = 0;
    let high = all.length - 1;
    while (low < high) {
      const mid = (low + high + 1) >> 1;
      if (topOf(all[mid].element) <= offset) {
        low = mid;
      } else {
        high = mid - 1;
      }
    }
    const previous = all[low];
    const previousTop = topOf(previous.element);
    if (offset < previousTop) {
      return previous.line * Math.max(0, offset) / Math.max(1, previousTop);
    }
    let next = null;
    for (let i = low + 1; i < all.length; i++) {
      if (all[i].line > previous.line) {
        next = all[i];
        break;
      }
    }
    if (next) {
      const nextTop = topOf(next.element);
      if (nextTop > previousTop) {
        const progress = Math.min(1, (offset - previousTop) / (nextTop - previousTop));
        return previous.line + progress * (next.line - previous.line);
      }
    }
    const height = Math.max(1, previous.element.getBoundingClientRect().height);
    return previous.line + Math.min(1, (offset - previousTop) / height);
  }

  function scrollToLine(line) {
    ignoreScroll++;
    window.scrollTo(0, Math.max(0, offsetForLine(line)));
    // scrollTo 不一定触发 scroll 事件（位置没变时），下一帧兜底清除。
    requestAnimationFrame(function () {
      requestAnimationFrame(function () { ignoreScroll = 0; });
    });
  }

  window.addEventListener('scroll', function () {
    if (ignoreScroll > 0) {
      return;
    }
    if (scrollPending) {
      return;
    }
    scrollPending = true;
    requestAnimationFrame(function () {
      scrollPending = false;
      if (ignoreScroll > 0 || documentId === null) {
        return;
      }
      post({ type: 'scroll', documentId: documentId, line: lineForOffset(window.scrollY) });
    });
  }, { passive: true });

  window.addEventListener('resize', function () { lines = null; });

  // ------------------------------------------------------------------ 渲染

  function highlight() {
    let count = 0;
    if (!window.hljs) {
      return count;
    }
    const blocks = content.querySelectorAll('pre > code[class*="language-"]');
    for (let i = 0; i < blocks.length; i++) {
      const code = blocks[i];
      const match = /language-([\w#+.-]+)/.exec(code.className);
      const language = match && window.hljs.getLanguage(match[1]) ? match[1] : null;
      if (!language) {
        continue;
      }
      try {
        const result = window.hljs.highlight(code.textContent, { language: language, ignoreIllegals: true });
        code.innerHTML = result.value;
        code.classList.add('hljs');
        count++;
      } catch (e) {
        // 高亮失败时保留纯文本。
      }
    }
    return count;
  }

  function renderMath() {
    const nodes = content.querySelectorAll('.rove-math');
    if (nodes.length === 0) {
      return Promise.resolve({ math: 0, mathErrors: 0 });
    }
    return load('vendor/katex/katex.min.js').then(function () {
      let errors = 0;
      for (let i = 0; i < nodes.length; i++) {
        const node = nodes[i];
        const tex = node.getAttribute('data-tex') || node.textContent;
        node.setAttribute('data-tex', tex);
        try {
          window.katex.render(tex, node, {
            displayMode: node.classList.contains('rove-math-display'),
            throwOnError: false,
            trust: false,
            macros: Object.assign({}, mathMacros),
          });
          if (node.querySelector('.katex-error')) {
            errors++;
          }
        } catch (e) {
          errors++;
        }
      }
      return { math: nodes.length, mathErrors: errors };
    }, function () {
      return { math: nodes.length, mathErrors: nodes.length };
    });
  }

  function renderMermaid() {
    const nodes = content.querySelectorAll('pre.mermaid');
    if (nodes.length === 0) {
      return Promise.resolve({ mermaid: 0, mermaidErrors: 0 });
    }
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];
      if (!node.hasAttribute('data-source')) {
        node.setAttribute('data-source', node.textContent);
      }
      node.textContent = node.getAttribute('data-source');
      node.removeAttribute('data-processed');
    }
    return load('vendor/mermaid.min.js').then(function () {
      window.mermaid.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        theme: themeKind === 'vscode-light' || themeKind === 'vscode-high-contrast-light' ? 'default' : 'dark',
      });
      return window.mermaid.run({ nodes: Array.prototype.slice.call(nodes), suppressErrors: true });
    }).then(function () {
      let errors = 0;
      for (let i = 0; i < nodes.length; i++) {
        const svg = nodes[i].querySelector('svg');
        if (!svg || nodes[i].querySelector('.error-icon')) {
          errors++;
        }
      }
      return { mermaid: nodes.length, mermaidErrors: errors };
    }, function () {
      return { mermaid: nodes.length, mermaidErrors: nodes.length };
    });
  }

  function images() {
    const all = content.querySelectorAll('img');
    const waits = [];
    for (let i = 0; i < all.length; i++) {
      const image = all[i];
      if (!image.complete) {
        waits.push(new Promise(function (resolve) {
          image.addEventListener('load', resolve, { once: true });
          image.addEventListener('error', resolve, { once: true });
        }));
      }
    }
    const timeout = new Promise(function (resolve) { setTimeout(resolve, 3000); });
    return Promise.race([Promise.all(waits), timeout]).then(function () {
      let local = 0;
      let blocked = 0;
      let failed = 0;
      for (let i = 0; i < all.length; i++) {
        const image = all[i];
        if (image.src.indexOf('https://rove.local/blocked-image.svg') === 0) {
          blocked++;
        } else if (image.naturalWidth > 0) {
          local++;
        } else {
          failed++;
        }
      }
      return { images: all.length, imagesLoaded: local, imagesBlocked: blocked, imagesFailed: failed };
    });
  }

  function setStyles(urls) {
    const old = document.querySelectorAll('link[data-rove-user-style]');
    for (let i = 0; i < old.length; i++) {
      old[i].remove();
    }
    for (let i = 0; i < urls.length; i++) {
      const link = document.createElement('link');
      link.rel = 'stylesheet';
      link.href = urls[i];
      link.setAttribute('data-rove-user-style', '');
      document.head.appendChild(link);
    }
  }

  function render(message) {
    const started = performance.now();
    documentId = message.documentId;
    seq = message.seq;
    mathMacros = message.mathMacros || {};
    setStyles(message.styles || []);
    document.body.classList.toggle('showEditorSelection', !!message.markEditorSelection);
    content.innerHTML = message.html;
    lines = null;
    const code = highlight();
    const current = seq;
    Promise.all([renderMath(), renderMermaid(), images()]).then(function (parts) {
      if (current !== seq) {
        return;
      }
      lines = null;
      if (typeof message.fragment === 'string' && message.fragment) {
        scrollToFragment(message.fragment);
      } else if (typeof message.line === 'number') {
        scrollToLine(message.line);
      }
      const stats = Object.assign({ code: code, ms: performance.now() - started }, parts[0], parts[1], parts[2]);
      stats.documentScript = typeof window.__roveDocumentScript !== 'undefined';
      stats.blocks = codeLines().length;
      post({ type: 'rendered', documentId: documentId, seq: current, stats: stats });
    });
  }

  function applyTheme(message) {
    const root = document.documentElement;
    const vars = message.vars || {};
    for (const name in vars) {
      if (Object.prototype.hasOwnProperty.call(vars, name)) {
        if (vars[name] === null) {
          root.style.removeProperty(name);
        } else {
          root.style.setProperty(name, vars[name]);
        }
      }
    }
    const body = document.body;
    body.classList.remove('vscode-light', 'vscode-dark', 'vscode-high-contrast', 'vscode-high-contrast-light');
    themeKind = message.kind || 'vscode-dark';
    body.classList.add(themeKind);
    if (themeKind === 'vscode-high-contrast-light') {
      body.classList.add('vscode-high-contrast');
    }
    if (content.querySelector('pre.mermaid')) {
      renderMermaid();
    }
    post({ type: 'themeApplied', seq: message.seq || 0 });
  }

  function scrollToFragment(fragment) {
    let target = document.getElementById(fragment);
    if (!target) {
      try {
        target = document.getElementById(decodeURIComponent(fragment));
      } catch (e) {
        target = null;
      }
    }
    if (target) {
      ignoreScroll++;
      target.scrollIntoView();
      requestAnimationFrame(function () { ignoreScroll = 0; });
    }
  }

  function markSelection(line) {
    const old = content.querySelectorAll('.code-active-line');
    for (let i = 0; i < old.length; i++) {
      old[i].classList.remove('code-active-line');
    }
    const all = codeLines();
    let best = null;
    for (let i = 0; i < all.length; i++) {
      if (all[i].line <= line) {
        best = all[i];
      }
    }
    if (best) {
      best.element.classList.add('code-active-line');
    }
  }

  window.chrome.webview.addEventListener('message', function (event) {
    const message = event.data;
    if (!message || message.version !== VERSION) {
      return;
    }
    switch (message.type) {
      case 'render':
        render(message);
        break;
      case 'theme':
        applyTheme(message);
        break;
      case 'scrollTo':
        if (message.documentId === documentId) {
          scrollToLine(message.line);
        }
        break;
      case 'selection':
        if (message.documentId === documentId) {
          markSelection(message.line);
        }
        break;
      case 'fragment':
        if (message.documentId === documentId) {
          scrollToFragment(message.fragment);
        }
        break;
      default:
        break;
    }
  });

  // ------------------------------------------------------------------ 交互

  document.addEventListener('dblclick', function (event) {
    if (documentId === null) {
      return;
    }
    const offset = event.clientY + window.scrollY;
    post({ type: 'dblclick', documentId: documentId, line: Math.floor(lineForOffset(offset)) });
  });

  document.addEventListener('click', function (event) {
    let node = event.target;
    while (node && node !== document.body && node.tagName !== 'A') {
      node = node.parentElement;
    }
    if (!node || node.tagName !== 'A' || !node.hasAttribute('href')) {
      return;
    }
    event.preventDefault();
    const href = node.getAttribute('href');
    if (href.charAt(0) === '#') {
      scrollToFragment(href.slice(1));
      return;
    }
    post({ type: 'link', documentId: documentId, href: href });
  }, true);

  // 中键与 Ctrl+点击也不能让网页自己打开新窗口。
  document.addEventListener('auxclick', function (event) { event.preventDefault(); }, true);

  // Tab 走到预览的边界时把焦点交回宿主（宿主负责后续的 Tab 顺序）。
  document.addEventListener('keydown', function (event) {
    if (event.key !== 'Tab' || event.ctrlKey || event.altKey || event.metaKey) {
      return;
    }
    const focusable = Array.prototype.filter.call(
      content.querySelectorAll('a[href], input, summary, [tabindex]:not([tabindex="-1"])'),
      function (e) { return e.offsetParent !== null; });
    const active = document.activeElement;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const outside = !active || active === document.body || focusable.indexOf(active) < 0;
    const leaving = focusable.length === 0 ||
      (event.shiftKey && (active === first || outside)) ||
      (!event.shiftKey && active === last);
    if (leaving) {
      event.preventDefault();
      post({ type: 'focusExit', shift: event.shiftKey });
    }
  }, true);

  post({ type: 'ready' });
})();
