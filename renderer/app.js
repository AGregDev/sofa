(function () {
  const $ = (s, r = document) => r.querySelector(s);
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const img = (p) => 'sofa://img/?p=' + encodeURIComponent(p);
  const api = window.sofa;

  const S = {
    items: [],
    settings: {},
    setupDone: true,
    tab: 'home',
    tabFocus: {},
    focusKey: null,
    layers: [],
    running: [], // windows the PS button came home from: [{ hwnd, title, exe, itemId, icon }]
    social: { discord: null, recentVoice: [], at: 0 },
  };
  const socialItems = new Map();
  let focused = null;

  const CAT_LABEL = { games: 'Games', media: 'Media', apps: 'Apps', system: 'System', hidden: 'Hidden' };
  const SYS_DESC = {
    sleep: 'The PC sleeps until you wake it',
    restart: 'Restarts the PC',
    shutdown: 'Turns the PC off',
    desktop: 'Shows the Windows desktop. Press the PS button to come back',
    quit: 'Closes Sofa completely',
    bigpicture: 'Opens Steam in Big Picture mode',
    settings: 'Sounds, motion, startup and hidden apps',
    rescan: 'Looks for newly installed games and apps',
    display: 'Resolution, HDR and scaling',
    sound: 'TV, headphones or the controller',
    bluetooth: 'Pair a controller or headphones',
  };

  // Running apps become cards that borrow their library item's art when Sofa knows it.
  function runItems() {
    return S.running.map((r) => {
      const base = r.itemId && S.items.find((i) => i.id === r.itemId);
      return {
        ...(base || {}),
        id: 'run:' + r.hwnd,
        hwnd: r.hwnd,
        running: true,
        baseId: base ? base.id : null,
        name: base ? base.name : (r.title || 'App'),
        art: base ? base.art : {},
        iconData: r.icon,
        source: base ? base.source : 'app',
        canKill: !/\\(applicationframehost|explorer)\.exe$/i.test(r.exe || ''),
      };
    });
  }
  const byId = (id) => S.items.find((i) => i.id === id)
    || (String(id).startsWith('run:') ? runItems().find((i) => i.id === id) : null)
    || socialItems.get(id) || null;
  const runningFor = (it) => runItems().find((r) => r.baseId && r.baseId === it.id);
  const inCat = (cat) => S.items.filter((i) => i.category === cat);
  const smooth = () => (S.settings.motion === 'reduced' ? 'auto' : 'smooth');

  function ago(t) {
    const d = Math.floor((Date.now() - t) / 86400000);
    if (d <= 0) return 'today';
    if (d === 1) return 'yesterday';
    if (d < 7) return `${d} days ago`;
    if (d < 30) { const w = Math.round(d / 7); return `${w} week${w === 1 ? '' : 's'} ago`; }
    return 'in ' + new Date(t).toLocaleDateString(undefined, { month: 'long', year: d > 300 ? 'numeric' : undefined });
  }

  function sortItems(list, cat) {
    if (cat === 'system') return list;
    return [...list].sort((a, b) => (b.lastPlayed || 0) - (a.lastPlayed || 0) || a.name.localeCompare(b.name));
  }

  // ---------- Focus and spatial navigation ----------

  function root() {
    const top = S.layers[S.layers.length - 1];
    return top ? top.el : $('#app');
  }

  function setFocus(el, opts = {}) {
    if (!el) return;
    if (focused) focused.classList.remove('focus');
    focused = el;
    el.classList.add('focus');
    if (!opts.silent) Sfx.play('move');
    if (el.classList.contains('tile') && $('#app').contains(el)) {
      S.focusKey = el.dataset.key;
      S.tabFocus[S.tab] = el.dataset.key;
      showHero(el.dataset.id);
      updateHints();
    }
    if (opts.scroll !== false) reveal(el);
  }

  function move(dir) {
    const els = [...root().querySelectorAll('[data-f]')].filter((e) => e.offsetParent);
    if (!focused || !root().contains(focused)) return setFocus(els[0]);
    const a = focused.getBoundingClientRect();
    const ax = a.left + a.width / 2, ay = a.top + a.height / 2;
    let best = null, bestScore = Infinity;
    for (const el of els) {
      if (el === focused) continue;
      const b = el.getBoundingClientRect();
      const bx = b.left + b.width / 2, by = b.top + b.height / 2;
      let primary, secondary;
      if (dir === 'right') { primary = b.left - a.right; secondary = Math.abs(by - ay); if (bx <= ax + 1) continue; }
      else if (dir === 'left') { primary = a.left - b.right; secondary = Math.abs(by - ay); if (bx >= ax - 1) continue; }
      else if (dir === 'down') { primary = b.top - a.bottom; secondary = Math.abs(bx - ax); if (by <= ay + 1) continue; }
      else { primary = a.top - b.bottom; secondary = Math.abs(bx - ax); if (by >= ay - 1) continue; }
      const score = Math.max(primary, -8) + secondary * 0.6;
      if (score < bestScore) { bestScore = score; best = el; }
    }
    if (best) setFocus(best);
    else Sfx.play('edge');
  }

  function reveal(el) {
    const track = el.closest('.track');
    if (track) {
      const pad = parseFloat(getComputedStyle(track).paddingLeft) || 0;
      let left = track.scrollLeft;
      if (el.offsetLeft - pad < left) left = el.offsetLeft - pad;
      else if (el.offsetLeft + el.offsetWidth + pad > left + track.clientWidth) left = el.offsetLeft + el.offsetWidth + pad - track.clientWidth;
      if (left !== track.scrollLeft) track.scrollTo({ left, behavior: smooth() });
    }
    const inner = el.closest('.shelf-inner');
    if (inner) {
      const row = el.closest('.row');
      const grid = el.closest('.grid');
      // First grid row keeps its heading; later rows scroll to the top with room for the focus ring.
      const firstTop = grid && grid.firstElementChild.offsetTop;
      const y = row ? row.offsetTop : grid ? (el.offsetTop === firstTop ? 0 : el.offsetTop - 16) : 0;
      inner.style.transform = `translateY(${-y}px)`;
    }
    if (!track && !inner) el.scrollIntoView({ block: 'nearest', behavior: smooth() });
  }

  function activate(el) {
    if (el._act) { Sfx.play('select'); el._act(); return; }
    if (el.classList.contains('tile')) openItem(byId(el.dataset.id), el);
  }

  // ---------- Layers ----------

  function openLayer(el, html, opts = {}) {
    el.innerHTML = html;
    el.hidden = false;
    const layer = { el, prev: focused, ...opts };
    S.layers.push(layer);
    wire(el, opts.acts || {});
    setFocus(el.querySelector(opts.focus || '[data-f]'), { silent: true });
    updateHints();
    return layer;
  }

  function closeLayer(silent) {
    const L = S.layers.pop();
    if (!L) return;
    L.el.hidden = true;
    L.el.innerHTML = '';
    if (L.onClose) L.onClose();
    if (!silent) Sfx.play('back');
    if (L.prev && document.contains(L.prev)) setFocus(L.prev, { silent: true, scroll: false });
    else restoreFocus();
    updateHints();
  }

  // Elements with data-act="name" call acts[name](el) when accepted.
  function wire(scope, acts) {
    scope.querySelectorAll('[data-act]').forEach((el) => {
      const fn = acts[el.dataset.act];
      if (fn) el._act = () => fn(el);
      el.addEventListener('click', () => { setFocus(el, { silent: true }); activate(el); });
    });
  }

  // ---------- Top bar ----------

  function tabs() {
    const t = [['home', 'Home'], ['games', 'Games'], ['media', 'Media'], ['apps', 'Apps'], ['social', 'Social'], ['system', 'System']];
    if (S.settings.showHidden) t.push(['hidden', 'Hidden']);
    return t;
  }

  function renderTabs() {
    $('#tabList').innerHTML = tabs().map(([k, label]) => `<div class="tab${k === S.tab ? ' on' : ''}" data-tab="${k}">${label}</div>`).join('');
    $('#tabList').querySelectorAll('.tab').forEach((el) => el.addEventListener('click', () => switchTab(el.dataset.tab)));
    requestAnimationFrame(() => {
      const on = $('#tabList .tab.on');
      if (!on) return;
      const nav = $('#tabs').getBoundingClientRect(), r = on.getBoundingClientRect();
      $('#tabLine').style.transform = `translateX(${r.left - nav.left}px) scaleX(${r.width})`;
    });
  }

  function switchTab(k) {
    const list = tabs().map((t) => t[0]);
    if (typeof k === 'number') k = list[(list.indexOf(S.tab) + k + list.length) % list.length];
    if (!list.includes(k) || k === S.tab) return;
    Sfx.play('tab');
    S.tab = k;
    renderTabs();
    renderShelf();
    restoreFocus();
    // The new tab's art follows at once, so it never shows the previous tab's game.
    clearTimeout(heroTimer);
    renderHero(focused && focused.classList.contains('tile') ? focused.dataset.id : null);
  }

  function tickClock() {
    $('#clock').textContent = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  // ---------- Shelf ----------

  function tile(it, key) {
    let inner;
    if (it.source === 'social') {
      const pic = it.avatar ? `<img class="avatar" src="${esc(it.avatar)}" alt="">`
        : it.brandMark ? brand(it.brandMark, 'mark') : it.glyph ? icon(it.glyph) : `<span class="initials">${esc(initials(it.name))}</span>`;
      inner = `<div class="face social${it.on ? ' on' : ''}">${pic}<div class="name">${esc(it.name)}</div>${it.sub ? `<div class="sub">${esc(it.sub)}</div>` : ''}</div>`;
      return `<div class="tile" data-f data-id="${esc(it.id)}" data-key="${esc(key)}">${inner}</div>`;
    }
    if (it.source === 'nav') {
      inner = `<div class="face nav">${icon('move')}<div class="name">${esc(it.name)}</div><div class="sub">${esc(it.sub)}</div></div>`;
      return `<div class="tile" data-f data-id="${esc(it.id)}" data-key="${esc(key)}">${inner}</div>`;
    }
    if (it.art?.capsule) inner = `<img class="cap" src="${img(it.art.capsule)}" alt="" loading="lazy" decoding="async">`;
    else if (it.art?.icon) inner = `<div class="face"><img src="${img(it.art.icon)}" alt=""><div class="name">${esc(it.name)}</div></div>`;
    else if (it.source === 'web') inner = `<div class="face"><div class="word" style="color:${esc(it.color)}">${esc(it.name)}</div><div class="name">Streaming</div></div>`;
    else if (it.source === 'system') inner = `<div class="face">${icon(it.glyph)}<div class="name">${esc(it.name)}</div></div>`;
    else if (it.iconData) inner = `<div class="face"><img src="${esc(it.iconData)}" alt=""><div class="name">${esc(it.name)}</div></div>`;
    else inner = `<div class="face">${icon('play')}<div class="name">${esc(it.name)}</div></div>`;
    const fav = it.favorite && !it.running ? `<span class="fav">${icon('star')}</span>` : '';
    const live = it.running ? '<span class="live"><i></i>Running</span>' : '';
    return `<div class="tile${it.running ? ' is-running' : ''}" data-f data-id="${esc(it.id)}" data-key="${esc(key)}">${inner}${fav}${live}</div>`;
  }

  function homeRows() {
    const visible = S.items.filter((i) => i.category !== 'hidden' && i.category !== 'system');
    const recent = visible.filter((i) => i.lastPlayed).sort((a, b) => b.lastPlayed - a.lastPlayed).slice(0, 10);
    const favs = visible.filter((i) => i.favorite);
    if (Date.now() - S.social.at > 60000) refreshSocial();
    // Home shows the first few of each section; the full list is one tile away.
    const MAX = 14;
    const capped = (cat) => {
      const all = sortItems(inCat(cat), cat);
      if (all.length <= MAX) return all;
      const more = { id: 'nav:' + cat, source: 'nav', tab: cat, name: `All ${CAT_LABEL[cat].toLowerCase()}`, sub: `${all.length}`, art: {} };
      socialItems.set(more.id, more);
      const list = [...all.slice(0, MAX), more];
      list.count = all.length;
      return list;
    };
    return [
      ['running', 'Running', runItems()],
      ['voice', 'Discord voice', voiceItems()],
      ['recent', 'Jump back in', recent],
      ['fav', 'Favorites', favs],
      ['games', 'Games', capped('games')],
      ['media', 'Media', capped('media')],
      ['apps', 'Apps', capped('apps')],
    ].filter((r) => r[2].length);
  }

  function renderShelf() {
    const shelf = $('#shelf');
    let html = '';
    if (S.tab === 'social') {
      html = socialRows().map(([k, title, list]) =>
        `<section class="row" data-row="${k}"><h2>${title}${list.count != null ? `<span class="count">${list.count}</span>` : ''}</h2><div class="track">${list.map((i) => tile(i, k + ':' + i.id)).join('')}</div></section>`).join('');
      if (Date.now() - S.social.at > 30000) refreshSocial();
    } else if (S.tab === 'home') {
      html = homeRows().map(([k, title, list]) =>
        `<section class="row" data-row="${k}"><h2>${title}<span class="count">${list.count ?? list.length}</span></h2><div class="track">${list.map((i) => tile(i, k + ':' + i.id)).join('')}</div></section>`).join('');
    } else {
      const list = sortItems(inCat(S.tab), S.tab);
      html = `<div class="grid-head">${CAT_LABEL[S.tab]}<span class="count">${list.length}</span></div>` +
        (list.length ? `<div class="grid">${list.map((i) => tile(i, S.tab + ':' + i.id)).join('')}</div>`
          : `<div class="empty">Nothing here yet. Move an app here from its options with ${key('triangle')}</div>`);
    }
    shelf.innerHTML = `<div class="shelf-inner">${html}</div>`;
    shelf.querySelectorAll('.track').forEach((t) => { t.style.position = 'relative'; });
    shelf.querySelectorAll('.tile').forEach((el) => {
      el.addEventListener('click', () => { setFocus(el, { silent: true }); activate(el); });
      el.addEventListener('mouseenter', () => { if (!document.body.classList.contains('no-cursor') && !S.layers.length) setFocus(el, { scroll: false }); });
    });
    // Only drop focus that was on the old shelf: a menu or sheet open on top keeps its
    // focus when the library refreshes underneath it.
    if (focused && !document.contains(focused)) focused = null;
  }

  function restoreFocus() {
    if (S.layers.length) return;
    const shelf = $('#shelf');
    const want = S.tabFocus[S.tab];
    let el = want && shelf.querySelector(`[data-key="${CSS.escape(want)}"]`);
    if (!el && want) el = shelf.querySelector(`[data-id="${CSS.escape(want.split(':').slice(1).join(':'))}"]`);
    el = el || shelf.querySelector('.tile');
    if (el) setFocus(el, { silent: true });
    else { focused = null; showHero(null); }
  }

  // ---------- Hero ----------

  let heroTimer = null;
  let heroId = undefined;

  function showHero(id) {
    clearTimeout(heroTimer);
    heroTimer = setTimeout(() => renderHero(id), 90);
  }

  function renderHero(id) {
    if (id === heroId) return;
    heroId = id;
    const it = id && byId(id);
    applyLight(it);
    const art = $('#heroArt');
    const text = $('#heroText');
    // Fade out the previous art; anything older is already invisible, so drop it now
    // rather than keep several full-screen images compositing at once.
    const olds = [...art.children];
    olds.forEach((old, i) => {
      if (i < olds.length - 1) { old.remove(); return; }
      old.classList.remove('in');
      setTimeout(() => old.remove(), 600);
    });
    if (!it) { text.innerHTML = ''; return; }

    let node;
    if (it.source === 'nav') {
      text.innerHTML = `<h1 class="hero-title">${esc(it.name)}</h1><div class="hero-meta">${esc(it.sub)} in your library</div>
        <div class="hero-actions"><span>${key('cross')}Open</span></div>`;
      return;
    }
    if (it.source === 'social') {
      node = document.createElement('div');
      node.className = 'icon-stage social-stage';
      node.innerHTML = it.avatar ? `<img src="${esc(it.avatar)}" alt="">` : it.brandMark ? brand(it.brandMark) : it.glyph ? icon(it.glyph) : `<div class="word">${esc(initials(it.name))}</div>`;
      requestAnimationFrame(() => requestAnimationFrame(() => node.classList.add('in')));
      art.appendChild(node);
      text.innerHTML = `<h1 class="hero-title">${esc(it.name)}</h1>${it.sub ? `<div class="hero-meta">${esc(it.sub)}</div>` : ''}`;
      return;
    }
    if (it.art?.hero) {
      // Big hero images decode off the main thread before they fade in, so moving
      // across a row never waits on a 4K image.
      node = new Image();
      node.className = 'art';
      node.decoding = 'async';
      node.src = img(it.art.hero);
      node.decode().then(() => { if (heroId === id) node.classList.add('in'); }, () => {});
    } else {
      node = document.createElement('div');
      node.className = 'icon-stage';
      if (it.art?.capsule) { node.classList.add('poster'); node.innerHTML = `<img src="${img(it.art.capsule)}" alt="">`; }
      else if (it.art?.icon) node.innerHTML = `<img src="${img(it.art.icon)}" alt="">`;
      else if (it.iconData) node.innerHTML = `<img src="${esc(it.iconData)}" alt="">`;
      else if (it.source === 'web') node.innerHTML = `<div class="word" style="color:${esc(it.color)}">${esc(it.name)}</div>`;
      else node.innerHTML = icon(it.glyph || 'app');
      requestAnimationFrame(() => requestAnimationFrame(() => node.classList.add('in')));
    }
    art.appendChild(node);

    const title = it.art?.logo
      ? `<img class="hero-logo" src="${img(it.art.logo)}" alt="${esc(it.name)}">`
      : `<h1 class="hero-title">${esc(it.name)}</h1>`;
    const meta = [];
    if (it.running) {
      text.innerHTML = `${title}
        <div class="hero-meta"><span class="live-dot"></span>Running</div>
        <div class="hero-actions"><span>${key('cross')}Resume</span><span>${key('triangle')}Close</span></div>`;
      return;
    }
    if (it.source === 'steam') meta.push('Steam');
    else if (it.source === 'recomp') meta.push(it.project && it.project !== it.name ? `${it.project} · RecompLauncher` : 'RecompLauncher');
    else if (it.id === 'web:browser') meta.push(it.browser || 'Browser');
    else if (it.source === 'web') meta.push(it.launch.inApp ? 'TV app inside Sofa' : `Opens full screen in ${it.browser || 'your browser'}`);
    else if (it.source === 'system') meta.push(SYS_DESC[it.glyph] || '');
    else meta.push({ games: 'Game', media: 'Media', apps: 'App', hidden: 'Hidden app' }[it.category] || 'App');
    if (it.lastPlayed) meta.push((it.category === 'games' ? 'Played ' : 'Opened ') + ago(it.lastPlayed));
    const verb = it.source === 'system' ? 'Select' : it.category === 'games' ? 'Play' : 'Open';
    text.innerHTML = `${title}
      <div class="hero-meta">${meta.map(esc).join('<i class="dot"></i>')}</div>
      <div class="hero-actions"><span>${key('cross')}${verb}</span>${it.source === 'system' ? '' : `<span>${key('triangle')}Options</span>`}</div>`;
  }

  // ---------- Hints ----------

  // The hint bar only offers what works on the focused tile.
  let hintSig = '';
  function updateHints() {
    const h = $('#hints');
    if (S.layers.length) { hintSig = ''; h.innerHTML = ''; return; }
    const it = focused && focused.classList.contains('tile') ? byId(focused.dataset.id) : null;
    const hasOptions = !!it && !['system', 'social', 'nav'].includes(it.source);
    const sig = hasOptions ? 'o' : 'n';
    if (sig === hintSig && h.innerHTML) return;
    hintSig = sig;
    h.innerHTML = `
      <span>${key('cross')}Select</span>
      ${hasOptions ? `<span>${key('triangle')}Options</span>` : ''}
      <span>${key('square')}Search</span>
      <span>${key('options')}Settings</span>
      <span class="spacer">${key('L2')}${key('R2')}Volume</span>`;
  }

  // ---------- Opening things ----------

  function openItem(it, el) {
    if (!it) return;
    if (it.source === 'nav') return switchTab(it.tab);
    if (it.source === 'social') { Sfx.play('select'); return socialAction(it); }
    if (it.source === 'system') return systemAction(it);
    // Like a console: picking something that's already running just resumes it.
    const already = it.running ? it : runningFor(it);
    if (already) return resume(already);
    const current = runItems()[0];
    if (current) return openSwitch(current, it, el);
    start(it, el);
  }

  function start(it, el) {
    Sfx.play('launch');
    Input.rumble(80, 0.5);
    playLaunch(it, el);
    api.launch(it.id);
  }

  function resume(r) {
    Sfx.play('select');
    Input.rumble(60, 0.4);
    api.resume(r.hwnd);
  }

  function closeRunning(r, force) {
    Sfx.play('back');
    S.running = S.running.filter((x) => x.hwnd !== r.hwnd);
    rerender();
    return api.closeRunning(r.hwnd, force);
  }

  function openSwitch(current, it, el) {
    Sfx.play('open');
    openLayer($('#confirm'), `<div class="panel">
      <h3>${esc(current.name)} is still running</h3>
      <p>Close it first, or keep it open and start ${esc(it.name)} as well.</p>
      <div class="opt" data-f data-act="swap">${icon('close')}Close it and start ${esc(it.name)}</div>
      <div class="opt" data-f data-act="both">${icon('play')}Start ${esc(it.name)} as well</div>
      <div class="opt" data-f data-act="no">${icon('restart')}Cancel</div>
    </div>`, {
      acts: {
        swap: () => { closeLayer(true); closeRunning(current); setTimeout(() => start(it, null), 1500); },
        both: () => { closeLayer(true); start(it, el && document.contains(el) ? el : null); },
        no: () => closeLayer(),
      },
    });
  }

  function openRunMenu(r) {
    Sfx.play('open');
    openLayer($('#menu'), `<div class="panel">
      <h3>${esc(r.name)}</h3>
      <div class="sub">Running</div>
      <div class="opt" data-f data-act="resume">${icon('play')}Resume</div>
      <div class="opt" data-f data-act="close">${icon('close')}Close ${esc(r.name)}</div>
      ${r.canKill ? `<div class="opt" data-f data-act="kill">${icon('shutdown')}Force close<span class="val">If it stops responding</span></div>` : ''}
      <div class="foot"><span>${key('cross')}Select</span><span>${key('circle')}Back</span></div>
    </div>`, {
      acts: {
        resume: () => { closeLayer(true); resume(r); },
        close: () => { closeLayer(true); closeRunning(r, false); },
        kill: () => { closeLayer(true); closeRunning(r, true); },
      },
    });
  }

  // Re-draw the current tab and keep focus where it was.
  function rerender() {
    const keep = S.focusKey;
    renderShelf();
    heroId = undefined;
    if (!S.layers.length) { S.tabFocus[S.tab] = keep || S.tabFocus[S.tab]; restoreFocus(); }
  }

  // PS button: open Home with the app you came from as the first card.
  function goHome(info) {
    hideLaunch();
    const setupOpen = S.layers.some((l) => l.el.id === 'setup');
    if (!setupOpen) while (S.layers.length) closeLayer(true);
    if (info && info.running) S.running = info.running;
    S.tab = 'home';
    renderTabs();
    renderShelf();
    heroId = undefined;
    S.tabFocus.home = info && info.hwnd ? `running:run:${info.hwnd}` : null;
    if (!setupOpen) { restoreFocus(); Sfx.play('open'); }
  }

  function systemAction(it) {
    const a = it.glyph;
    if (a === 'settings') return openSettings();
    if (a === 'sound') return openAudio();
    if (a === 'rescan') return rescan();
    if (it.confirm) {
      return openConfirm(`${it.name}?`, a === 'restart' ? 'Anything unsaved in open apps will be lost.' : 'Anything unsaved in open apps will be lost.', it.name, () => api.launch(it.id));
    }
    Sfx.play('select');
    api.launch(it.id);
  }

  async function rescan() {
    Sfx.play('select');
    const el = focused;
    if (el) el.querySelector('.name').textContent = 'Scanning';
    apply(await api.rescan());
  }

  let launchTimer = null;
  function playLaunch(it, el) {
    const L = $('#launch');
    const r = el ? el.getBoundingClientRect() : { left: innerWidth / 2 - 80, top: innerHeight / 2 - 120, width: 160, height: 240 };
    const capsule = !!it.art?.capsule;
    const src = it.art?.capsule || it.art?.icon;
    L.innerHTML = `<div class="ghost${capsule ? '' : ' icon'}" style="left:${r.left}px;top:${r.top}px;width:${r.width}px;height:${r.height}px">${src ? `<img src="${img(src)}" alt="">` : ''}</div>
      <div class="caption"><b>${esc(it.name)}</b><div class="bar"><i></i></div></div>`;
    L.hidden = false;
    const ghost = L.querySelector('.ghost');
    const h = innerHeight * 0.36;
    const s = h / r.height;
    const tx = innerWidth / 2 - (r.width * s) / 2 - r.left;
    const ty = innerHeight / 2 - h / 2 - innerHeight * 0.06 - r.top;
    requestAnimationFrame(() => requestAnimationFrame(() => { ghost.style.transform = `translate(${tx}px, ${ty}px) scale(${s})`; }));
    clearTimeout(launchTimer);
    launchTimer = setTimeout(hideLaunch, 7000);
  }

  function hideLaunch() {
    clearTimeout(launchTimer);
    const L = $('#launch');
    L.hidden = true;
    L.innerHTML = '';
  }

  // ---------- Item options (△) ----------

  function openItemMenu() {
    if (!focused || !focused.classList.contains('tile')) return;
    const it = byId(focused.dataset.id);
    if (!it || it.source === 'system' || it.source === 'social') return;
    if (it.running) return openRunMenu(it);
    Sfx.play('open');
    const moves = ['games', 'media', 'apps'].filter((c) => c !== it.category);
    const html = `<div class="panel">
      <h3>${esc(it.name)}</h3>
      <div class="sub">${CAT_LABEL[it.category] || ''}</div>
      <div class="opt" data-f data-act="fav">${icon('star')}${it.favorite ? 'Remove from favorites' : 'Add to favorites'}</div>
      ${moves.map((c) => `<div class="opt" data-f data-act="move" data-cat="${c}">${icon('move')}Move to ${CAT_LABEL[c]}</div>`).join('')}
      <div class="opt" data-f data-act="rename">${icon('rename')}Rename</div>
      <div class="opt" data-f data-act="cover">${icon('image')}Change cover</div>
      ${['steam', 'start', 'recomp'].includes(it.source) ? `<div class="opt" data-f data-act="uninstall">${icon('trash')}Uninstall</div>` : ''}
      <div class="opt" data-f data-act="hide">${icon(it.category === 'hidden' ? 'show' : 'hide')}${it.category === 'hidden' ? 'Show again' : 'Hide'}</div>
      <div class="foot"><span>${key('cross')}Select</span><span>${key('circle')}Close</span></div>
    </div>`;
    const update = async (patch) => { closeLayer(true); apply(await api.setItem(it.id, patch)); };
    openLayer($('#menu'), html, {
      acts: {
        fav: () => update({ favorite: !it.favorite }),
        move: (el) => update({ category: el.dataset.cat, hidden: false }),
        hide: () => update(it.category === 'hidden' ? { hidden: false, category: null } : { hidden: true }),
        cover: () => { closeLayer(true); openCoverPicker(it); },
        uninstall: async () => {
          closeLayer(true);
          const c = await api.uninstallCheck(it.id);
          const run = async () => {
            const r = await api.uninstall(it.id);
            if (r === 'removed') Sfx.play('back');
            else if (String(r).startsWith('error')) openConfirm("Couldn't uninstall", r.slice(6), 'Open installed apps', () => api.open('ms-settings:appsfeatures'));
          };
          const text = {
            steam: ['Steam asks you to confirm, then removes the game and its files.', 'Uninstall'],
            recomp: ['Opens RecompLauncher, where you can remove it.', 'Open RecompLauncher'],
            program: [`Opens the uninstaller for ${c.name}. It asks you to confirm in its own window.`, 'Uninstall'],
            appx: ['Removes it from this PC. This can’t be undone here.', 'Uninstall'],
          }[c.kind];
          if (text) return openConfirm(`Uninstall ${it.name}?`, text[0], text[1], run);
          if (c.kind === 'system') return openConfirm(`${it.name} is part of Windows`, 'Windows doesn’t allow removing it. You can hide it instead.', 'Hide', () => api.setItem(it.id, { hidden: true }).then(apply));
          openConfirm(`No uninstaller found for ${it.name}`, 'Windows’ installed apps list may still have it.', 'Open installed apps', () => api.open('ms-settings:appsfeatures'));
        },
        rename: () => {
          closeLayer(true);
          openKeyboard({ title: 'Rename', value: it.name, done: (v) => v.trim() && api.setItem(it.id, { name: v.trim() }).then(apply) });
        },
      },
    });
  }

  // ---------- Social: Discord voice ----------

  const initials = (s) => String(s || '?').split(/\s+/).map((w) => w[0]).join('').slice(0, 2).toUpperCase();

  function socialRows() {
    socialItems.clear();
    const add = (it) => { socialItems.set(it.id, { source: 'social', art: {}, ...it }); return socialItems.get(it.id); };
    const rows = [];
    const d = S.social.discord;

    if (d && d.current) {
      const v = d.voice || {};
      const list = [
        ...d.current.members.map((m) => add({ id: 'dc:m:' + m.id, kind: 'member', name: m.name, avatar: m.avatar, sub: m.muted ? 'Muted' : 'In voice' })),
        add({ id: 'dc:mute', kind: 'mute', name: v.mute ? 'Unmute' : 'Mute', glyph: v.mute ? 'micOff' : 'mic', on: v.mute }),
        add({ id: 'dc:deaf', kind: 'deaf', name: v.deaf ? 'Undeafen' : 'Deafen', glyph: 'headphones', on: v.deaf }),
        add({ id: 'dc:leave', kind: 'leave', name: 'Leave voice', glyph: 'close' }),
      ];
      rows.push(['voice', esc(d.current.name), list]);
    }
    if (d && d.guilds && d.guilds.length) {
      const list = d.guilds.map((g) => add({ id: 'dc:g:' + g.id, kind: 'guild', guildId: g.id, name: g.name, avatar: g.icon, sub: 'Voice channels' }));
      list.push(add({ id: 'dc:setup', kind: 'dsetup', name: 'Discord account', brandMark: 'discord', sub: d.user ? d.user.name : 'Connected' }));
      list.count = d.guilds.length;
      rows.push(['servers', 'Discord servers', list]);
    } else if (d && d.notRunning) {
      rows.push(['dsetup', 'Discord', [add({ id: 'dc:start', kind: 'dstart', name: 'Open Discord', brandMark: 'discord', sub: 'Discord needs to be running' })]]);
    } else {
      const err = d && d.error;
      rows.push(['dsetup', 'Discord', [add({ id: 'dc:setup', kind: 'dsetup', name: err ? 'Reconnect Discord' : 'Connect Discord', brandMark: 'discord', sub: err || 'Join voice from the couch' })]]);
    }
    return rows;
  }

  // Home's Discord row: the channel you're in with its controls, then channels you joined recently.
  function voiceItems() {
    const d = S.social.discord;
    if (!d || !d.configured || d.error) return [];
    const add = (it) => { const full = { source: 'social', art: {}, ...it }; socialItems.set(full.id, full); return full; };
    const list = [];
    if (d.current) {
      const v = d.voice || {};
      const n = d.current.members.length;
      list.push(add({ id: 'dc:here', kind: 'here', name: d.current.name, sub: `${n} in voice`, brandMark: 'discord', on: true }));
      list.push(add({ id: 'dc:mute', kind: 'mute', name: v.mute ? 'Unmute' : 'Mute', glyph: v.mute ? 'micOff' : 'mic', on: v.mute }));
      list.push(add({ id: 'dc:deaf', kind: 'deaf', name: v.deaf ? 'Undeafen' : 'Deafen', glyph: 'headphones', on: v.deaf }));
      list.push(add({ id: 'dc:leave', kind: 'leave', name: 'Leave voice', glyph: 'close' }));
    }
    for (const c of S.social.recentVoice || []) {
      if (d.current && c.id === d.current.id) continue;
      const who = c.members.length ? `${c.members.length} in voice` : (c.guild || 'Empty');
      list.push(add({ id: 'dc:vc:' + c.id, kind: 'vc', channelId: c.id, name: c.name, sub: who, avatar: c.members[0]?.avatar || null, glyph: 'sound' }));
    }
    return list;
  }

  // Polls Discord quietly; the screen only redraws when something visible changed,
  // so focus never jumps while you're moving around.
  let socialBusy = null;
  let socialSig = '';
  function refreshSocial() {
    if (socialBusy) return socialBusy;
    socialBusy = Promise.all([api.discord(), api.discordRecent()]).then(([discord, recentVoice]) => {
      S.social = { discord, recentVoice: Array.isArray(recentVoice) ? recentVoice : [], at: Date.now() };
      const sig = JSON.stringify(S.social.discord) + JSON.stringify(S.social.recentVoice);
      if (sig === socialSig) return;
      socialSig = sig;
      if ((S.tab === 'social' || S.tab === 'home') && !S.layers.length) rerender();
    }).finally(() => { socialBusy = null; });
    return socialBusy;
  }
  setInterval(() => { if ((S.tab === 'social' || S.tab === 'home') && document.hasFocus()) refreshSocial(); }, 60000);

  function socialAction(it) {
    switch (it.kind) {
      case 'dsetup': return openDiscordSetup();
      case 'dstart': {
        const app = S.items.find((i) => i.source === 'start' && /^discord$/i.test(i.name));
        if (app) return start(app, null);
        return openConfirm('Discord isn’t installed', 'Install Discord on this PC, then open it once and sign in.', 'OK', () => {});
      }
      case 'mute': return api.discordVoice({ mute: !S.social.discord.voice?.mute }).then(refreshSocial);
      case 'deaf': return api.discordVoice({ deaf: !S.social.discord.voice?.deaf }).then(refreshSocial);
      case 'leave': return api.discordJoin(null).then(refreshSocial);
      case 'guild': return openGuild(it);
      case 'vc': return api.discordJoin(it.channelId).then(refreshSocial);
      case 'here': return openVoiceMembers();
      default: return null;
    }
  }

  function openVoiceMembers() {
    const c = S.social.discord && S.social.discord.current;
    if (!c) return;
    Sfx.play('open');
    openLayer($('#menu'), `<div class="panel">
      <h3>${esc(c.name)}</h3>
      <div class="sub">${c.members.length} in voice</div>
      ${c.members.map((m) => `<div class="opt">${m.avatar ? `<img class="opt-av" src="${esc(m.avatar)}" alt="">` : icon('users')}<span class="sep">${esc(m.name)}</span>${m.muted ? `<span class="val">${icon('micOff')}</span>` : ''}</div>`).join('')}
      <div class="group-gap"></div>
      <div class="opt" data-f data-act="leave">${icon('close')}Leave voice</div>
      <div class="foot"><span>${key('circle')}Back</span></div>
    </div>`, { acts: { leave: async () => { closeLayer(true); await api.discordJoin(null); refreshSocial(); } } });
  }

  async function openGuild(g) {
    Sfx.play('open');
    const chans = await api.discordGuild(g.guildId);
    const list = Array.isArray(chans) ? chans : [];
    const current = S.social.discord?.current?.id;
    openLayer($('#menu'), `<div class="panel">
      <h3>${esc(g.name)}</h3>
      <div class="sub">${list.length ? 'Voice channels' : esc(chans.error || 'No voice channels you can join')}</div>
      ${list.map((c) => `<div class="opt" data-f data-act="join" data-c="${esc(c.id)}">${icon('sound')}<span class="sep">${esc(c.name)}${c.members.length ? `<span class="who">${esc(c.members.slice(0, 4).join(', '))}${c.members.length > 4 ? ` and ${c.members.length - 4} more` : ''}</span>` : ''}</span>${c.id === current ? '<span class="val">Connected</span>' : ''}</div>`).join('')}
      <div class="foot"><span>${key('cross')}Join</span><span>${key('circle')}Back</span></div>
    </div>`, {
      acts: {
        join: async (el) => { closeLayer(true); await api.discordJoin(el.dataset.c); Sfx.play('select'); refreshSocial(); },
      },
    });
  }

  // Discord only lets an app control voice for the people who made it, so each person
  // makes a small Discord application once. Sofa walks through it step by step and
  // picks the Client ID and secret straight off the clipboard.
  const DISCORD_ID = /^\d{17,21}$/;
  const DISCORD_SECRET = /^[A-Za-z0-9_-]{24,64}$/;

  async function openDiscordSetup(force) {
    Sfx.play('open');
    const cfg = await api.discordConfig();
    const d = S.social.discord;
    if (cfg.signedIn && d && d.user && !force) return openDiscordAccount(d.user);
    let clientId = cfg.clientId;
    let secret = null;
    let msg = '';
    const hasSecret = () => !!(secret || cfg.hasSecret);
    const step = (n, done, title, detail, act, val) => `<div class="opt step${done ? ' done' : ''}${act === 'connect' && clientId && hasSecret() ? ' ready' : ''}" data-f data-act="${act}">
        <span class="num">${done ? icon('check') : n}</span><span class="sep">${title}<span class="who">${detail}</span></span>${val ? `<span class="val">${val}</span>` : ''}</div>`;
    const draw = () => `<div class="panel">
      <h3>Connect Discord</h3>
      <div class="sub">One-time setup, about two minutes. Discord only lets apps you made control voice, so you make a small one for Sofa.</div>
      ${step(1, !!clientId, 'Make a Discord application', 'Opens discord.com/developers. Choose New Application and name it Sofa.', 'page')}
      ${step(2, !!clientId, 'Add the redirect', `In OAuth2, add the redirect <b>${esc(cfg.redirect)}</b> and save.`, 'page')}
      ${step(3, !!clientId, 'Paste the Client ID', 'Copy it from OAuth2, then select this.', 'id', clientId ? esc(clientId.slice(0, 6)) + '…' : '')}
      ${step(4, hasSecret(), 'Paste the Client Secret', 'Reset Secret, copy it, then select this.', 'secret', hasSecret() ? 'Saved' : '')}
      ${step(5, false, 'Connect', 'Discord asks you to Authorize Sofa once.', 'connect')}
      ${msg ? `<div class="note">${esc(msg)}</div>` : ''}
      <div class="foot"><span>${key('cross')}Select</span><span>${key('circle')}Back</span></div>
    </div>`;
    const reopen = (focus) => openLayer($('#menu'), draw(), { acts, focus: `[data-act="${focus}"]` });
    const redraw = (focus) => { closeLayer(true); reopen(focus); };
    const fromClipboard = async (re) => {
      const t = (await api.clipboard()).trim();
      return re.test(t) ? t : null;
    };
    const acts = {
      page: () => api.openSocial('https://discord.com/developers/applications'),
      id: async () => {
        const t = await fromClipboard(DISCORD_ID);
        if (t) { clientId = t; msg = ''; Sfx.play('select'); return redraw('secret'); }
        closeLayer(true);
        openKeyboard({ title: 'Client ID', value: clientId, done: (v) => { clientId = v.trim(); setTimeout(() => reopen('secret'), 50); } });
      },
      secret: async () => {
        const t = await fromClipboard(DISCORD_SECRET);
        if (t && !DISCORD_ID.test(t)) { secret = t; msg = ''; Sfx.play('select'); return redraw('connect'); }
        closeLayer(true);
        openKeyboard({ title: 'Client Secret', done: (v) => { secret = v.trim() || secret; setTimeout(() => reopen('connect'), 50); } });
      },
      connect: async (el) => {
        if (!DISCORD_ID.test(clientId || '')) { msg = 'The Client ID is a long number from the OAuth2 page.'; return redraw('id'); }
        if (!hasSecret()) { msg = 'Paste the Client Secret first.'; return redraw('secret'); }
        el.querySelector('.who').textContent = 'Waiting for Discord. Approve Sofa there.';
        const r = await api.setDiscordApp(clientId, secret);
        S.social.discord = r;
        socialSig = '';
        if (r && r.user) { closeLayer(true); Sfx.play('select'); toast('Discord connected', `Signed in as ${r.user.name}`); rerender(); return; }
        msg = (r && r.error) || 'Discord didn’t answer. Is it open and signed in?';
        redraw('connect');
      },
    };
    openLayer($('#menu'), draw(), { acts, focus: clientId ? (hasSecret() ? '[data-act="connect"]' : '[data-act="secret"]') : '[data-act="page"]' });
  }

  function openDiscordAccount(user) {
    openLayer($('#menu'), `<div class="panel">
      <h3>Discord</h3>
      <div class="sub">Signed in as ${esc(user.name)}. Sofa stays signed in until you disconnect.</div>
      <div class="opt" data-f data-act="redo">${icon('restart')}Set up again</div>
      <div class="opt" data-f data-act="forget">${icon('close')}Disconnect</div>
      <div class="foot"><span>${key('cross')}Select</span><span>${key('circle')}Back</span></div>
    </div>`, {
      acts: {
        redo: () => { closeLayer(true); openDiscordSetup(true); },
        forget: () => openConfirm('Disconnect Discord?', 'Sofa forgets the application and its sign-in.', 'Disconnect', async () => {
          closeLayer(true);
          S.social.discord = await api.discordForget();
          socialSig = '';
          rerender();
        }),
      },
    });
  }

  // ---------- DualSense: battery, lightbar, mute button ----------

  const LIGHT_LABEL = { follow: 'Selected game', amber: 'Amber', off: 'Off' };
  const LAYOUT_LABEL = { auto: 'Match controller', ps: 'PlayStation', xbox: 'Xbox', nintendo: 'Nintendo' };
  const LAYOUTS = ['auto', 'ps', 'xbox', 'nintendo'];
  async function cycleLayout(el) {
    const next = LAYOUTS[(LAYOUTS.indexOf(S.settings.controllerLayout || 'auto') + 1) % LAYOUTS.length];
    S.settings = await api.setSettings({ controllerLayout: next });
    Input.setLayout(next);
    el.querySelector('.val').textContent = LAYOUT_LABEL[next];
  }

  function applyLight(it) {
    const mode = S.settings.controllerLight || 'follow';
    if (mode === 'off') return DS.setLight(null);
    if (mode === 'amber') return DS.setLight('#f0a24b');
    DS.setLight(it && it.color ? it.color : '#f0a24b');
  }

  function showBattery(s) {
    const el = $('#padBatt');
    if (!s.connected || s.battery == null) { el.hidden = true; return; }
    el.hidden = false;
    el.classList.toggle('low', s.battery <= 20 && !s.charging);
    el.innerHTML = `${brand('playstation')}${s.charging ? icon('zap') : ''}<span>${s.battery}%</span>`;
  }
  DS.on('battery', showBattery);
  // PS button read over HID: tell the helper, which brings Sofa forward from a game
  // (and keeps Steam down). While Sofa is in front the normal input path handles it.
  DS.on('ps', (kind) => api.padHome(kind));

  // Mute button, like on a PS5: mutes you in Discord voice, otherwise the Windows microphone.
  async function syncMuteLed() {
    const d = S.social.discord;
    if (d && d.current && d.voice) return DS.setMuteLed(d.voice.mute);
    DS.setMuteLed((await api.micMute('get')) === 'muted');
  }
  DS.on('mute', async () => {
    const d = S.social.discord;
    if (d && d.current && d.voice) {
      await api.discordVoice({ mute: !d.voice.mute });
      await refreshSocial();
    } else {
      DS.setMuteLed((await api.micMute('toggle')) === 'muted');
    }
    syncMuteLed();
    Input.rumble(40, 0.3);
  });

  // ---------- Controller headphones ----------
  // Like a PS5: headphones in the controller's jack take Windows sound and Discord
  // voice; unplugging gives both back. Windows only offers the jack over USB.
  async function onJack(j) {
    if (S.settings.padAudio === false) return;
    if (j.headphones && j.bt) {
      toast('Headphones need a USB cable', "Windows can't send sound to the controller over Bluetooth. Connect it with a USB cable and Sofa moves sound and Discord voice to it.", 9000);
      return;
    }
    if (!j.headphones) {
      const r = await api.padAudio(false);
      if (r && r.restored) toast(`Sound back on ${r.restored.replace(/^\w+ \((.*)\)$/, '$1')}`, '');
      return;
    }
    // Just after the cable goes in, Windows can take a moment to list the controller.
    for (let i = 0; i < 5; i++) {
      const r = await api.padAudio(true);
      if (r && r.ok) { toast('Sound on the controller', r.discord ? 'Headphones connected. Discord voice too.' : 'Headphones connected.'); return; }
      await new Promise((res) => setTimeout(res, 1500));
      if (!DS.state.headphones && DS.state.connected) return;
    }
    toast('Headphones connected', "Windows hasn't listed the controller's audio yet. Choose it in System, Sound output.");
  }
  DS.on('jack', onJack);

  // ---------- Browser choice ----------

  const BROWSER = { firefox: 'Firefox', chrome: 'Chrome', edge: 'Edge' };

  function browserOpts() {
    const b = S.browsers || { installed: [], chosen: null };
    const rows = b.installed.map((k) => `<div class="opt" data-f data-act="pickBrowser" data-b="${k}">${icon('globe')}<span class="sep">${BROWSER[k]}</span>${b.chosen === k ? `<span class="val">${icon('check')}</span>` : ''}</div>`);
    if (!b.installed.includes('firefox')) {
      rows.unshift(`<div class="opt" data-f data-act="getFirefox">${icon('globe')}<span class="sep">Install Firefox<span class="who">From Mozilla, through Windows' package manager. Installing accepts Mozilla's terms.</span></span><span class="val">Install</span></div>`);
    }
    return rows.join('');
  }

  async function pickBrowser(k) {
    S.settings = await api.setSettings({ browser: k });
    S.browsers = await api.browsers();
    Sfx.play('select');
  }

  async function getFirefox() {
    const ok = await api.installFirefox();
    S.browsers = await api.browsers();
    if (!ok) openConfirm("Firefox didn't install", 'Windows may have asked for permission, or the download failed. You can try again from settings.', 'OK', () => {});
  }

  async function openBrowserPicker() {
    Sfx.play('open');
    S.browsers = await api.browsers();
    const draw = () => `<div class="panel"><h3>Browser</h3><div class="sub">Used for the Browser tile and for streaming services</div>${browserOpts()}
      <div class="foot"><span>${key('cross')}Select</span><span>${key('circle')}Back</span></div></div>`;
    const acts = {
      pickBrowser: async (o) => { await pickBrowser(o.dataset.b); closeLayer(true); openBrowserPicker(); },
      getFirefox: async (o) => { o.querySelector('.val').textContent = 'Installing'; await getFirefox(); closeLayer(true); openBrowserPicker(); },
    };
    openLayer($('#menu'), draw(), { acts });
  }

  // ---------- Sound output ----------

  async function openAudio() {
    Sfx.play('open');
    const devices = await api.audioDevices();
    const pad = (n) => /wireless controller|dualsense/i.test(n);
    openLayer($('#menu'), `<div class="panel">
      <h3>Sound output</h3>
      <div class="sub">The controller's headphone jack appears here while it's connected by USB</div>
      ${devices.map((d, i) => `<div class="opt" data-f data-act="use" data-i="${i}">${pad(d.name) ? brand('playstation') : icon('sound')}<span class="sep">${esc(d.name)}</span>${d.default ? `<span class="val">${icon('check')}</span>` : ''}</div>`).join('')}
      ${devices.length ? '' : '<div class="sub">No playback devices found</div>'}
      <div class="group-gap"></div>
      <div class="opt" data-f data-act="more">${icon('settings')}More sound settings</div>
      <div class="foot"><span>${key('cross')}Use</span><span>${key('circle')}Back</span></div>
    </div>`, {
      focus: '.opt',
      acts: {
        use: async (el) => { closeLayer(true); await api.setAudio(devices[+el.dataset.i].id); Sfx.play('select'); },
        more: () => { closeLayer(true); api.open('ms-settings:sound'); },
      },
    });
  }

  // ---------- Cover picker ----------

  async function openCoverPicker(it) {
    Sfx.play('open');
    const pics = await api.pictures();
    const el = $('#cover');
    const html = `<div class="picker-wrap">
      <h2 class="picker-title">Cover for ${esc(it.name)}</h2>
      <div class="sub">Pictures, Downloads and Desktop, newest first</div>
      <div class="grid picker-grid">
        <div class="tile pick-act" data-f data-act="browse"><div class="face">${icon('move')}<div class="name">Browse</div></div></div>
        ${it.art?.custom ? `<div class="tile pick-act" data-f data-act="reset"><div class="face">${icon('restart')}<div class="name">Original art</div></div></div>` : ''}
        ${pics.map((p, i) => `<div class="tile" data-f data-act="pick" data-i="${i}"><img class="cap" loading="lazy" decoding="async" src="${img(p)}" alt=""></div>`).join('')}
      </div>
      ${pics.length ? '' : '<div class="empty">No pictures found in Pictures, Downloads or Desktop</div>'}
    </div>
    <div class="kb-hints"><span>${key('cross')}Use as cover</span><span>${key('circle')}Cancel</span></div>`;
    const done = (v) => { closeLayer(true); if (v) apply(v); Sfx.play('select'); };
    openLayer(el, html, {
      acts: {
        pick: async (t) => done(await api.setCover(it.id, pics[+t.dataset.i])),
        reset: async () => done(await api.setCover(it.id, null)),
        browse: async () => { const v = await api.browseCover(it.id); if (v) done(v); },
      },
    });
  }

  // ---------- Settings (Options button) ----------

  function openSettings() {
    Sfx.play('open');
    const s = S.settings;
    const sw = (on) => `<span class="switch${on ? ' on' : ''}"></span>`;
    const html = `<div class="panel">
      <h3>Settings</h3>
      <div class="sub">Sofa ${esc(S.version || '')}</div>
      <div class="opt" data-f data-act="toggle" data-k="launchAtLogin">Start Sofa with Windows${sw(s.launchAtLogin)}</div>
      <div class="opt" data-f data-act="homeBtn">Home button<span class="val">${homeLabel()}</span></div>
      <div class="opt" data-f data-act="layout">Button layout<span class="val">${LAYOUT_LABEL[s.controllerLayout || 'auto']}</span></div>
      <div class="opt" data-f data-act="browser">Browser<span class="val">${esc(S.items.find((i) => i.id === 'web:browser')?.browser || 'None')}</span></div>
      <div class="opt" data-f data-act="light">Controller light<span class="val">${LIGHT_LABEL[s.controllerLight || 'follow']}</span></div>
      <div class="opt" data-f data-act="toggle" data-k="padAudio">Headphones in the controller take the sound${sw(s.padAudio !== false)}</div>
      <div class="opt" data-f data-act="steamGuide">Keep Steam off the PS button${sw(s.steamGuide !== false)}</div>
      <div class="opt" data-f data-act="toggle" data-k="keepInFront">Keep Sofa in front of the desktop${sw(s.keepInFront)}</div>
      <div class="opt" data-f data-act="toggle" data-k="fullscreen">Full screen${sw(s.fullscreen)}</div>
      <div class="opt" data-f data-act="toggle" data-k="sounds">Interface sounds${sw(s.sounds)}</div>
      <div class="opt" data-f data-act="motion">Reduce motion${sw(s.motion === 'reduced')}</div>
      <div class="opt" data-f data-act="toggle" data-k="showHidden">Show hidden apps${sw(s.showHidden)}</div>
      <div class="group-gap"></div>
      <div class="opt" data-f data-act="rescan">${icon('rescan')}Rescan library</div>
      <div class="opt" data-f data-act="setup">${icon('settings')}Run setup again</div>
      <div class="opt" data-f data-act="desktop">${icon('desktop')}Go to desktop<span class="val">PS button returns</span></div>
      <div class="opt" data-f data-act="quit">${icon('quit')}Quit Sofa</div>
      <div class="foot"><span>${key('cross')}Change</span><span>${key('circle')}Close</span></div>
    </div>`;
    const flip = async (el, patch) => {
      S.settings = await api.setSettings(patch);
      el.querySelector('.switch').classList.toggle('on');
      applySettings();
    };
    openLayer($('#settings'), html, {
      acts: {
        toggle: (el) => flip(el, { [el.dataset.k]: !S.settings[el.dataset.k] }),
        // Turning it on applies it now (asking first if that means restarting Steam);
        // turning it off only stops Sofa from changing Steam again.
        steamGuide: async (el) => {
          const on = S.settings.steamGuide === false;
          await flip(el, { steamGuide: on });
          if (on) offerSteamGuideFix();
        },
        motion: (el) => flip(el, { motion: S.settings.motion === 'reduced' ? 'full' : 'reduced' }),
        homeBtn: async (el) => {
          S.settings = await api.setSettings({ homeButton: S.settings.homeButton === 'combo' ? 'ps' : 'combo' });
          el.querySelector('.val').innerHTML = homeLabel();
        },
        browser: () => { closeLayer(true); openBrowserPicker(); },
        layout: (el) => cycleLayout(el),
        light: async (el) => {
          const order = ['follow', 'amber', 'off'];
          const next = order[(order.indexOf(S.settings.controllerLight || 'follow') + 1) % order.length];
          S.settings = await api.setSettings({ controllerLight: next });
          el.querySelector('.val').textContent = LIGHT_LABEL[next];
          applyLight(heroId ? byId(heroId) : null);
        },
        rescan: () => { closeLayer(true); rescan(); },
        setup: async () => { closeLayer(true); await api.setupReset(); openSetup(); },
        desktop: () => { closeLayer(true); api.desktop(); },
        quit: () => api.launch('sys:quit'),
      },
    });
  }

  function homeLabel() {
    return S.settings.homeButton === 'combo' ? 'Create + Options' : 'PS button';
  }
  const homeKeys = () => (S.settings.homeButton === 'combo' ? `${key('Create')}+${key('options')}` : key('PS'));

  function applySettings() {
    Sfx.enabled = S.settings.sounds;
    document.body.classList.toggle('reduced', S.settings.motion === 'reduced');
    const hadHidden = $('#tabList').textContent.includes('Hidden');
    if (hadHidden !== !!S.settings.showHidden) {
      if (!S.settings.showHidden && S.tab === 'hidden') S.tab = 'home';
      renderTabs();
      renderShelf();
      if (!S.layers.length) restoreFocus();
    }
  }

  // ---------- Confirm + quick menu ----------

  function openConfirm(title, body, yes, onYes) {
    Sfx.play('open');
    openLayer($('#confirm'), `<div class="panel">
      <h3>${esc(title)}</h3><p>${esc(body)}</p>
      <div class="opt" data-f data-act="yes">${icon('check')}${esc(yes)}</div>
      <div class="opt" data-f data-act="no">${icon('close')}Cancel</div>
    </div>`, { acts: { yes: () => { closeLayer(true); onYes(); }, no: () => closeLayer() } });
  }

  // ---------- On-screen keyboard (search + rename) ----------

  const KB_ROWS = ['1234567890', 'qwertyuiop', "asdfghjkl'", 'zxcvbnm-.&'];

  function openKeyboard(opts) {
    let value = opts.value || '';
    const isSearch = !!opts.search;
    const keys = KB_ROWS.map((r) => [...r].map((c) => `<div class="kb-key" data-f data-act="char" data-c="${esc(c)}">${esc(c)}</div>`).join('')).join('') +
      `<div class="kb-key wide" data-f data-act="space">${icon('space')}</div>
       <div class="kb-key" data-f data-act="erase">${icon('backspace')}</div>
       <div class="kb-key mid" data-f data-act="paste">Paste</div>
       <div class="kb-key mid" data-f data-act="clear">Clear</div>
       <div class="kb-key mid" data-f data-act="done">${isSearch ? 'Close' : 'Done'}</div>`;
    const html = `<div class="kb-wrap">
      <div class="kb-field">${icon(isSearch ? 'search' : 'rename')}<span class="kb-text"></span><i class="caret"></i></div>
      <div class="kb-cols"><div class="kb-keys">${keys}</div>
      <div class="kb-side">${isSearch ? '<h2 class="kb-title"></h2><div class="grid kb-results"></div>' : ''}</div></div>
    </div>
    <div class="kb-hints"><span>${key('cross')}Type</span><span>${key('square')}Delete</span><span>${key('triangle')}Space</span><span>${key('circle')}${isSearch ? 'Close' : 'Cancel'}</span></div>`;

    const draw = () => {
      $('.kb-text', el).innerHTML = value ? esc(value) : `<span class="ph">${isSearch ? 'Search' : esc(opts.title)}</span>`;
      if (!isSearch) return;
      const q = value.trim().toLowerCase();
      const hits = q ? S.items.filter((i) => (S.settings.showHidden || i.category !== 'hidden') && i.name.toLowerCase().includes(q)).slice(0, 8) : [];
      $('.kb-title', el).textContent = q ? (hits.length ? `${hits.length} result${hits.length === 1 ? '' : 's'}` : 'No matches') : 'Type to search everything';
      const res = $('.kb-results', el);
      res.innerHTML = hits.map((i) => tile(i, 'search:' + i.id)).join('');
      res.querySelectorAll('.tile').forEach((t) => {
        t._act = () => { const it = byId(t.dataset.id); closeLayer(true); Input.textMode = false; openItem(it, null); };
        t.addEventListener('click', () => t._act());
      });
    };
    const type = (c) => { value += c; draw(); };
    const erase = () => { value = value.slice(0, -1); draw(); };
    const paste = async () => {
      const t = (await api.clipboard()).replace(/\s+/g, ' ').trim();
      if (t) { type(t); Sfx.play('select'); } else Sfx.play('edge');
    };
    const el = $('#search');
    openLayer(el, html, {
      focus: '[data-c="a"]',
      acts: {
        char: (k) => type(k.dataset.c),
        space: () => type(' '),
        erase,
        clear: () => { value = ''; draw(); },
        paste,
        done: () => { closeLayer(); finish(); },
      },
      onAction(action, src) {
        if (action === 'char') { type(src); return true; }
        if (action === 'paste') { paste(); return true; }
        // A real keyboard's Enter finishes, rather than pressing the focused key.
        if (action === 'accept' && src === 'key') { closeLayer(); finish(); return true; }
        if (action === 'erase' || action === 'search') { erase(); return true; }
        if (action === 'menu') { type(' '); return true; }
        return false;
      },
      onClose() { Input.textMode = false; },
    });
    Input.textMode = true;
    const finish = () => { if (!isSearch && opts.done) opts.done(value); };
    draw();
  }

  // ---------- Setup ----------

  function openSetup() {
    let step = 0;
    let raw = null;
    const el = $('#setup');
    const layer = openLayer(el, '<div class="setup-panel"></div>', {
      onAction(action, src) {
        // The controller check uses holds for ✕ and ○; the keyboard still works normally.
        if (step === 1 && src === 'pad' && (action === 'accept' || action === 'back')) return true;
        if (action === 'back') { if (step > 0) { step--; draw(); Sfx.play('back'); } return true; }
        if (action === 'accept' && !(focused && focused._act && el.contains(focused))) { next(); return true; }
        return false;
      },
      onClose() { Input.onRaw(null); },
    });
    const total = 7;
    api.browsers().then((b) => { S.browsers = b; if (step === 4) draw(); });
    api.steamGuide().then((g) => { S.steamGuide = g; if (step === 5) draw(); });
    const progress = () => `<div class="step-no">${Array.from({ length: total }, (_, i) => `<i class="${i <= step ? 'on' : ''}"></i>`).join('')}<span>&nbsp;${step + 1} of ${total}</span></div>`;
    const counts = () => ({ games: inCat('games').length, media: inCat('media').length, apps: inCat('apps').length });

    function next() {
      Sfx.play('select');
      if (step === total - 1) {
        api.setupDone();
        S.setupDone = true;
        closeLayer(true);
        return;
      }
      step++;
      draw();
    }

    function draw() {
      Input.onRaw(null);
      const panel = $('.setup-panel', el);
      const s = S.settings;
      const sw = (on) => `<span class="switch${on ? ' on' : ''}"></span>`;
      const cont = `<div class="opt" data-f data-act="next">${icon('check')}Continue</div>`;
      let h = '';
      if (step === 0) {
        h = `${progress()}<h1>Set up Sofa</h1>
          <p class="lead">Sofa puts your games, streaming and apps on the TV, and everything works from the controller. Setup takes about a minute.</p>
          <div class="actions"><span>${key('cross')}Start</span></div>`;
      } else if (step === 1) {
        const btns = [['cross', 0], ['circle', 1], ['square', 2], ['triangle', 3], ['L1', 4], ['R1', 5], ['L2', 6], ['R2', 7], ['options', 9], ['PS', 16]];
        h = `${progress()}<h1>Check your controller</h1>
          <p class="lead">Press any button, including ${key('cross')} and ${key('circle')}. Each one lights up here as it registers.</p>
          <div class="body"><div class="pad-test">${btns.map(([k, i]) => key(k).replace('class="key"', `class="key" data-b="${i}"`)).join('')}</div>
          <div class="pad-name"></div></div>
          <div class="actions"><span class="hold" data-h="0">Hold ${key('cross')} to continue</span><span class="hold" data-h="1">Hold ${key('circle')} to go back</span></div>`;
      } else if (step === 2) {
        h = `${progress()}<h1>Your library</h1>
          <p class="lead">Found on Steam and in the Start menu. Move or hide anything later with ${key('triangle')}</p>
          <div class="body stats">
            <div class="stat"><b data-n="games">0</b><span>Games</span></div>
            <div class="stat"><b data-n="media">0</b><span>Media</span></div>
            <div class="stat"><b data-n="apps">0</b><span>Apps</span></div>
          </div>
          <div class="actions"><span>${key('cross')}Continue</span><span>${key('circle')}Back</span></div>`;
      } else if (step === 3) {
        h = `${progress()}<h1>Preferences</h1>
          <p class="lead">You can change these later in settings with ${key('options')}</p>
          <div class="body">
            <div class="opt" data-f data-act="toggle" data-k="launchAtLogin">Start Sofa with Windows${sw(s.launchAtLogin)}</div>
            <div class="opt" data-f data-act="toggle" data-k="fullscreen">Full screen${sw(s.fullscreen)}</div>
            <div class="opt" data-f data-act="toggle" data-k="sounds">Interface sounds${sw(s.sounds)}</div>
            <div class="opt" data-f data-act="homeBtn">Home button<span class="val">${homeLabel()}</span></div>
            <div class="opt" data-f data-act="layout">Button layout<span class="val">${LAYOUT_LABEL[s.controllerLayout || 'auto']}</span></div>
            <div class="group-gap"></div>${cont}
          </div>`;
      } else if (step === 4) {
        h = `${progress()}<h1>Browser</h1>
          <p class="lead">Websites, Netflix, Disney+, Prime Video and Twitch open full screen in it, with their own sign-ins kept for the TV.</p>
          <div class="body">${browserOpts()}<div class="group-gap"></div>${cont}</div>`;
      } else if (step === 5) {
        const tw = [
          ['ms-settings:gaming-gamebar', 'Stop Game Bar opening on the PS button', 'Game Bar', 'Turn off opening Game Bar with the controller button'],
          ['ms-settings:display', 'Make text readable from the couch', 'Display', 'Scale: 150% or more'],
          ['ms-settings:sound', 'Send sound to the TV or receiver', 'Sound', 'Output: your TV or receiver'],
        ];
        // Steam's own setting is changed by Sofa itself (see offerSteamGuideFix).
        const st = S.steamGuide;
        const steamRow = st && st.installed ? `<div class="opt tweak" data-f data-act="steamFix"><span class="sep">Stop Steam answering the PS button<span class="who">${st.off ? 'Done: "Guide button focuses Steam" is off' : 'Sofa turns off "Guide button focuses Steam" for you'}</span></span><span class="val">${st.off ? icon('check') : 'Fix'}</span></div>` : '';
        h = `${progress()}<h1>Recommended changes</h1>
          <p class="lead">So only Sofa answers the PS button, and text is readable from the couch.</p>
          <div class="body">${steamRow}${tw.map(([u, l, v, d]) => `<div class="opt tweak" data-f data-act="open" data-u="${u}"><span class="sep">${esc(l)}<span class="who">${esc(d)}</span></span><span class="val">Open ${v}</span></div>`).join('')}
          <div class="group-gap"></div>${cont}</div>`;
      } else {
        h = `${progress()}<h1>Ready</h1>
          <div class="actions"><span>${key('cross')}Go to Home</span></div>`;
      }
      panel.innerHTML = h;
      wire(panel, {
        next,
        toggle: async (o) => {
          S.settings = await api.setSettings({ [o.dataset.k]: !S.settings[o.dataset.k] });
          o.querySelector('.switch').classList.toggle('on');
          applySettings();
        },
        open: (o) => api.open(o.dataset.u),
        steamFix: async () => {
          if (S.steamGuide && S.steamGuide.off) return toast('Steam is already set up', 'Only Sofa answers the PS button.');
          await offerSteamGuideFix();
          // Redraw this step once the fix is done (a Steam restart takes a while), or
          // after a minute, or as soon as setup moves on.
          const t0 = Date.now();
          const watch = setInterval(async () => {
            if (!S.layers.includes(layer) || step !== 5) return clearInterval(watch);
            if (S.layers[S.layers.length - 1] !== layer) return; // the question is still open
            const st = await api.steamGuide();
            if ((st && st.off) || Date.now() - t0 > 60000) { clearInterval(watch); S.steamGuide = st; if (step === 5) draw(); }
          }, 1500);
        },
        homeBtn: async (o) => {
          S.settings = await api.setSettings({ homeButton: S.settings.homeButton === 'combo' ? 'ps' : 'combo' });
          o.querySelector('.val').innerHTML = homeLabel();
        },
        pickBrowser: async (o) => { await pickBrowser(o.dataset.b); draw(); },
        layout: (o) => cycleLayout(o),
        getFirefox: async (o) => { o.querySelector('.val').textContent = 'Installing'; await getFirefox(); draw(); },
      });
      focused = null;
      const first = panel.querySelector('[data-f]');
      if (first) setFocus(first, { silent: true });

      if (step === 1) {
        // Here ✕ and ○ are being tested, so a tap only lights them; holding moves on.
        const lit = {};
        const armed = { 0: false, 1: false }; // ignore the press that opened this step
        const since = { 0: 0, 1: 0 };
        const HOLD = 800;
        Input.onRaw((raw, down) => {
          // Keycaps are labelled by role (confirm, back); in the Nintendo layout those
          // two roles sit on swapped physical buttons.
          const i = Input.confirmIndex === 1 && raw < 2 ? 1 - raw : raw;
          const k = panel.querySelector(`[data-b="${i}"]`);
          if (k && lit[i] !== down) { k.classList.toggle('lit', down); lit[i] = down; }
          if (i !== 0 && i !== 1) return;
          const bar = panel.querySelector(`.hold[data-h="${i}"]`);
          if (!down) { armed[i] = true; since[i] = 0; if (bar) bar.style.setProperty('--p', 0); return; }
          if (!armed[i]) return;
          const now = performance.now();
          if (!since[i]) since[i] = now;
          const p = Math.min(1, (now - since[i]) / HOLD);
          if (bar) bar.style.setProperty('--p', p);
          if (p >= 1) {
            armed[i] = false;
            if (i === 0) next(); else { step--; draw(); Sfx.play('back'); }
          }
        });
        const name = () => {
          const p = Input.pad;
          const n = panel.querySelector('.pad-name');
          if (n) n.textContent = p ? `${p.id.replace(/\s*\(.*$/, '')} connected` : 'No controller yet. Connect one over USB or Bluetooth, or use the keyboard.';
        };
        name();
        layer.padTimer = setInterval(() => { if (step !== 1 || el.hidden) clearInterval(layer.padTimer); else name(); }, 500);
      }
      if (step === 2) tweenCounts();
    }

    function tweenCounts() {
      const target = counts();
      const t0 = performance.now();
      const run = (t) => {
        const k = Math.min(1, (t - t0) / 700);
        const e = 1 - Math.pow(1 - k, 3);
        for (const [n, v] of Object.entries(target)) {
          const b = el.querySelector(`[data-n="${n}"]`);
          if (b) b.textContent = Math.round(v * e);
        }
        if (k < 1 && step === 2) requestAnimationFrame(run);
      };
      requestAnimationFrame(run);
    }
    layer.refresh = () => { if (step === 2) tweenCounts(); };
    draw();
  }

  // ---------- Input ----------

  Input.on((action, src) => {
    const top = S.layers[S.layers.length - 1];
    if (!$('#launch').hidden) {
      if (action === 'back' || action === 'home') { hideLaunch(); Sfx.play('back'); }
      return;
    }
    if (top && top.onAction && top.onAction(action, src)) return;
    if (action === 'up' || action === 'down' || action === 'left' || action === 'right') return move(action);
    if (action === 'accept') { if (focused) activate(focused); return; }
    if (action === 'volUp' || action === 'volDown') { api.pad(action === 'volUp' ? 'vol up' : 'vol down'); return; }
    if (top) {
      if (action === 'back' || (action === 'options' && top.el.id === 'settings') || (action === 'menu' && top.el.id === 'menu')) closeLayer();
      return;
    }
    switch (action) {
      case 'tabPrev': return switchTab(-1);
      case 'tabNext': return switchTab(1);
      case 'menu': return openItemMenu();
      case 'search': Sfx.play('open'); return openKeyboard({ search: true });
      case 'options': return openSettings();
      case 'home': goHome(null); return;
      case 'back': if (S.tab !== 'home') switchTab('home'); else Sfx.play('edge'); return;
    }
  });

  // ---------- Data ----------

  function apply(v) {
    S.items = v.items;
    S.settings = v.settings;
    if (v.version) S.version = v.version;
    if (!S.settings) return;
    Sfx.enabled = S.settings.sounds;
    Input.setLayout(S.settings.controllerLayout || 'auto');
    document.body.classList.toggle('reduced', S.settings.motion === 'reduced');
    const keep = S.focusKey;
    renderTabs();
    renderShelf();
    heroId = undefined;
    if (!S.layers.length) {
      S.tabFocus[S.tab] = keep || S.tabFocus[S.tab];
      restoreFocus();
    } else {
      const top = S.layers[S.layers.length - 1];
      if (top.refresh) top.refresh();
      const el = keep && $('#shelf').querySelector(`[data-key="${CSS.escape(keep)}"]`);
      if (el && S.layers[0]) S.layers[0].prev = el;
      showHero(el ? el.dataset.id : null);
    }
  }

  function setRunning(list) {
    const before = S.running.map((r) => r.hwnd).join();
    S.running = list || [];
    if (S.running.map((r) => r.hwnd).join() !== before && S.tab === 'home') rerender();
  }

  window.addEventListener('focus', async () => {
    if (!$('#launch').hidden) setTimeout(hideLaunch, 300);
    setRunning(await api.running());
  });
  window.addEventListener('resize', () => renderTabs());

  async function boot() {
    tickClock();
    setInterval(tickClock, 10000);
    updateHints();
    const v = await api.boot();
    S.setupDone = v.setupDone;
    apply(v);
    api.onLibrary(apply);
    api.onHome(goHome);
    api.onRunning(setRunning);
    api.onSocial(() => { clearTimeout(boot.socialT); boot.socialT = setTimeout(() => refreshSocial().then(syncMuteLed), 300); });
    syncMuteLed();
    showBattery(DS.state);
    if (!S.setupDone) openSetup();
  }

  // ---------- Toast: a passive notice, top right ----------

  let toastTimer = null;
  function toast(title, body, ms = 5000) {
    const el = $('#toast');
    el.innerHTML = `<div><b>${esc(title)}</b>${body ? `<span>${esc(body)}</span>` : ''}</div>`;
    el.hidden = false;
    el.classList.remove('out');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { el.classList.add('out'); setTimeout(() => { el.hidden = true; }, 300); }, ms);
  }

  // ---------- Steam and the PS button ----------
  // Steam also answers the PS button while its "Guide button focuses Steam" setting is
  // on. Sofa turns it off by itself while Steam is closed (main.js); with Steam open the
  // change needs a Steam restart, so Sofa asks first.

  const STEAM_RESULT = {
    'still-running': ["Steam didn't close", 'Close Steam yourself, then try again.'],
    'no-steam': ["Steam isn't installed", ''],
    'no-account': ['Sign in to Steam once first', 'Then Sofa can change the setting.'],
    write: ["Couldn't change Steam's setting", 'Nothing was changed. You can turn it off in Steam, Settings, Controller.'],
  };

  async function fixSteamGuide() {
    toast('Fixing Steam', 'Steam closes, changes one setting, and starts again in the background.', 30000);
    const r = await api.steamGuideFix();
    if (r && r.ok) {
      S.settings = await api.setSettings({ steamGuide: true });
      toast(r.changed ? 'Steam no longer takes the PS button' : 'Steam was already set up', r.restarted ? 'Steam restarted in the background.' : '');
    } else if (r && r.reason === 'game') toast(`Close ${r.name} first`, 'Sofa won’t restart Steam while a Steam game is running.');
    else { const [t, b] = STEAM_RESULT[r && r.reason] || STEAM_RESULT.write; toast(t, b, 8000); }
    return r;
  }

  // Asks before restarting Steam; with Steam closed it just fixes it.
  async function offerSteamGuideFix({ fromGrab = false } = {}) {
    const st = await api.steamGuide();
    if (!st || !st.installed || st.off) return st && st.off ? toast('Steam is already set up', 'Only Sofa answers the PS button.') : null;
    if (!st.running) return fixSteamGuide();
    Sfx.play('open');
    openLayer($('#confirm'), `<div class="panel">
      <h3>${fromGrab ? 'Steam is also using the PS button' : 'Keep Steam off the PS button?'}</h3>
      <p>Sofa can turn off Steam's <b>Guide button focuses Steam</b> for you. Steam closes and starts again in the background, so finish any Steam game first.</p>
      <div class="opt" data-f data-act="fix">${icon('check')}Fix it now</div>
      <div class="opt" data-f data-act="no">${icon('close')}Not now</div>
    </div>`, { acts: { fix: () => { closeLayer(true); fixSteamGuide(); }, no: () => closeLayer() } });
  }

  let steamTipShown = false;
  api.onSteamGrab(() => {
    if (steamTipShown || S.layers.length) return;
    steamTipShown = true;
    offerSteamGuideFix({ fromGrab: true });
  });
  api.onSteamFixed(() => toast('Steam no longer takes the PS button', 'Sofa turned off "Guide button focuses Steam" while Steam was closed.', 7000));

  // Exposed for the screenshot test run.
  window.__sofa = { S, move, switchTab, openSettings, openItemMenu, openKeyboard, goHome, openSetup, closeLayer, refreshLibrary: () => api.boot().then(apply) };
  boot();
})();
