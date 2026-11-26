// `npm run qa`: drives Sofa through its screens with simulated controller and keyboard
// input, checks each step (see QA.md), and saves a screenshot of each screen to ./shots
// (or $SOFA_SHOTS). Uses a throwaway profile and never launches or opens anything.
// Results go to shots/qa.txt; the process exits 0 only when every check passes.
const fs = require('fs');
const path = require('path');

exports.run = (win) => {
  const out = process.env.SOFA_SHOTS || path.join(process.cwd(), 'shots');
  fs.mkdirSync(out, { recursive: true });
  const wc = win.webContents;
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const results = [];
  const errors = [];
  const js = (code) => wc.executeJavaScript(code);

  wc.on('console-message', (e) => {
    const level = e.level ?? e;
    if (level === 'error' || level === 3) errors.push(e.message || String(e));
  });

  function check(id, name, ok, detail = '') {
    results.push(`${ok ? 'PASS' : 'FAIL'}  ${id}  ${name}${detail ? '  (' + detail + ')' : ''}`);
  }

  async function key(k, n = 1, modifiers) {
    for (let i = 0; i < n; i++) {
      wc.sendInputEvent({ type: 'keyDown', keyCode: k, modifiers });
      wc.sendInputEvent({ type: 'keyUp', keyCode: k, modifiers });
      await wait(240);
    }
  }
  async function shot(name, settle = 700) {
    await wait(settle);
    const img = await wc.capturePage();
    fs.writeFileSync(path.join(out, name + '.png'), img.resize({ width: 1600 }).toPNG());
    return !img.isEmpty();
  }
  async function pad(buttons, hold = 120) {
    await js(`window.__fake = ${JSON.stringify(buttons)}`);
    await wait(hold);
    await js('window.__fake = []');
    await wait(250);
  }
  const q = (expr) => js(`(() => { try { return ${expr}; } catch (e) { return 'ERR ' + e.message; } })()`);

  // SOFA_DEMO=1: record a short tour of Home as numbered frames in shots/demo, for the
  // README's animated preview (assembled with ffmpeg).
  async function recordDemo() {
    const dir = path.join(out, 'demo');
    fs.rmSync(dir, { recursive: true, force: true });
    fs.mkdirSync(dir, { recursive: true });
    let n = 0;
    let recording = true;
    const t0 = Date.now();
    const times = [];
    const grab = (async () => {
      while (recording) {
        const img = await wc.capturePage();
        times.push(Date.now() - t0);
        fs.writeFileSync(path.join(dir, `f${String(n++).padStart(4, '0')}.png`), img.resize({ width: 1280 }).toPNG());
      }
    })();
    const steps = [['Right', 520], ['Right', 520], ['Right', 520], ['Right', 700], ['Down', 700], ['Right', 520], ['Right', 900],
      ['E', 900], ['Right', 520], ['Right', 520], ['Down', 900], ['E', 700], ['E', 700], ['E', 1100], ['Q', 500], ['Q', 500], ['Q', 900], ['O', 900], ['Down', 400], ['Down', 900], ['Escape', 900]];
    await wait(600);
    for (const [k, ms] of steps) { await key(k); await wait(ms - 240); }
    recording = false;
    await grab;
    fs.writeFileSync(path.join(dir, 'times.json'), JSON.stringify(times));
    await key('Up', 3);
  }
  const layerOpen = (id) => q(`!document.querySelector('#${id}').hidden`);

  wc.once('did-finish-load', async () => {
    try {
      // A test window never has focus and has no real controller: fake both.
      await js(`(() => {
        document.hasFocus = () => true;
        window.__fake = [];
        const mk = () => ({ id: 'DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)', connected: true,
          timestamp: performance.now(), axes: [0, 0, 0, 0],
          buttons: Array.from({ length: 17 }, (_, i) => ({ pressed: window.__fake.includes(i), value: window.__fake.includes(i) ? 1 : 0 })) });
        navigator.getGamepads = () => [mk()];
      })()`);

      // ---- Setup ----
      await shot('01-setup-start', 2500);
      check('S1', 'First run opens setup', await layerOpen('setup'));
      await pad([0]);
      await pad([0]); await pad([1]);
      check('S2', 'Taps on the controller check only light buttons', await q(`document.querySelector('.setup h1').textContent === 'Check your controller'`));
      await js('window.__fake = [0, 2]');
      await shot('02-setup-controller', 400);
      await js('window.__fake = []');
      await wait(300);
      await pad([0], 1100);
      check('S3', 'Holding cross moves on', await q(`document.querySelector('.setup h1').textContent === 'Your library'`));
      await shot('03-setup-library', 8000);
      await key('Return');
      await shot('04-setup-prefs');
      await key('Down', 8); await key('Return');
      await shot('05-setup-browser', 1500);
      await key('Down', 8); await key('Return');
      await shot('06-setup-tweaks');
      check('S4', 'Setup recommends the Steam guide button fix', await q(`document.querySelector('.setup').textContent.includes('Guide button focuses Steam')`));
      await key('Down', 8); await key('Return');
      await key('Return');
      await wait(800);
      check('S5', 'Finishing setup shows Home', await q(`document.querySelector('#setup').hidden && document.querySelector('.tab.on').textContent === 'Home'`));

      // ---- Home ----
      await shot('07-home', 1500);
      if (process.env.SOFA_DEMO) await recordDemo();
      const tabs = await q(`[...document.querySelectorAll('.tab')].map((t) => t.textContent).join(',')`);
      check('H1', 'Tabs are Home, Games, Media, Apps, Social, System', tabs === 'Home,Games,Media,Apps,Social,System', tabs);
      const rowMax = await q(`Math.max(...[...document.querySelectorAll('.row .track')].map((t) => t.children.length))`);
      check('H2', 'Home rows stay short (15 tiles at most)', rowMax <= 15, `longest row ${rowMax}`);
      check('H3', 'A tile has focus and the hero shows its name', await q(`!!document.querySelector('.tile.focus') && document.querySelector('#heroText').textContent.trim().length > 0`));

      // Smoothness: move across a row and down through the rows, and record any main-thread
      // task long enough to drop frames (art decoding, re-rendering, layout).
      const perf = await js(`new Promise((resolve) => {
        const long = [];
        const obs = new PerformanceObserver((l) => l.getEntries().forEach((e) => long.push(Math.round(e.duration))));
        obs.observe({ type: 'longtask', buffered: false });
        const keys = [...Array(12).fill('ArrowRight'), ...Array(3).fill('ArrowDown'), ...Array(3).fill('ArrowUp'), ...Array(12).fill('ArrowLeft')];
        let i = 0;
        const step = () => {
          const t0 = performance.now();
          window.dispatchEvent(new KeyboardEvent('keydown', { key: keys[i] }));
          long.push(-Math.round(performance.now() - t0)); // negative: synchronous handler time
          if (++i < keys.length) setTimeout(step, 140); else setTimeout(() => { obs.disconnect(); resolve(long); }, 900);
        };
        step();
      })`);
      const handler = Math.max(0, ...perf.filter((v) => v <= 0).map((v) => -v));
      const tasks = perf.filter((v) => v > 0);
      check('P1', 'Each move is handled in under 16 ms', handler < 16, `slowest move ${handler} ms`);
      check('P2', 'No main-thread task over 100 ms while browsing', !tasks.some((v) => v > 100), tasks.length ? `long tasks ${tasks.join(', ')} ms` : 'none');

      // "All games" tile at the end of a long row opens the Games tab.
      const hasNav = await q(`!!document.querySelector('[data-id^="nav:"]')`);
      if (hasNav) {
        await js(`(() => { const el = document.querySelector('[data-id^="nav:"]'); el.click(); })()`);
        await wait(500);
        check('H4', 'The "All ..." tile opens its tab', await q(`document.querySelector('.tab.on').textContent !== 'Home'`));
        await key('Escape');
        await wait(400);
      } else check('H4', 'The "All ..." tile opens its tab', true, 'library too small to need one');

      // ---- Running app (PS button from a game) ----
      await js(`window.__sofa.goHome({ hwnd: '4242', running: [{ hwnd: '4242', title: 'Red Dead Redemption 2', exe: '', itemId: 'steam:1174180', icon: null }] })`);
      await shot('20-home-running', 1200);
      check('R1', 'PS from a game puts it first, focused', await q(`document.querySelector('.tile.focus')?.dataset.id === 'run:4242'`));
      await key('M');
      await shot('21-running-menu');
      check('R2', 'Triangle on it offers resume, close, force close', await q(`['Resume','Close','Force close'].every((w) => document.querySelector('#menu').textContent.includes(w))`));
      await key('Escape');
      await key('Right', 2);
      await key('Return');
      await shot('22-switch-prompt');
      check('R3', 'Starting another game asks about the running one', await layerOpen('confirm'));
      await key('Escape');

      // ---- Settings ----
      await key('O');
      await shot('17-settings');
      check('T1', 'Settings has Go to desktop and Quit Sofa', await q(`['Go to desktop','Quit Sofa','Button layout'].every((w) => document.querySelector('#settings').textContent.includes(w))`));
      // Focus two rows down, refresh the library underneath, then one more Down must land
      // on the fourth row (a lost focus would jump back to the first).
      await key('Down', 2);
      const fourth = await q(`[...document.querySelectorAll('#settings .opt')][3].textContent`);
      await js('window.__sofa.refreshLibrary()');
      await wait(500);
      await key('Down');
      const now = await q(`document.querySelector('#settings .focus')?.textContent`);
      check('T2', 'A library refresh under an open menu keeps its focus', now === fourth, now);
      await key('Escape');

      // ---- Social ----
      await key('E', 4);
      await shot('23-social', 1500);
      const social = await q(`document.querySelector('#shelf').textContent`);
      check('D1', 'Social offers Connect Discord and nothing from Steam', /Connect Discord/.test(social) && !/Steam/.test(social), social.slice(0, 60));
      await key('Return');
      await shot('25-discord-setup');
      check('D2', 'Discord setup shows five numbered steps', await q(`document.querySelectorAll('#menu .opt.step').length === 5`));
      require('electron').clipboard.writeText('1449231011723214000');
      await js(`document.querySelector('#menu [data-act="id"]').click()`);
      await wait(500);
      check('D3', 'Client ID is picked up from the clipboard', await q(`document.querySelector('#menu [data-act="id"] .val')?.textContent.startsWith('144923')`));
      await shot('25b-discord-setup-step', 300);
      await key('Escape');

      // ---- System ----
      await key('E');
      await shot('27-system', 1000);
      check('Y1', 'System has Go to desktop and Quit Sofa tiles', await q(`['Go to desktop','Quit Sofa'].every((w) => document.querySelector('#shelf').textContent.includes(w))`));

      // ---- Button layouts ----
      await js("Input.setLayout('nintendo')");
      await key('O');
      await shot('28-settings-nintendo');
      check('L1', 'Nintendo layout shows A to select', await q(`getComputedStyle(document.querySelector('#settings .foot .key .nt')).display !== 'none' && document.querySelector('#settings .foot .key .nt').textContent === 'A'`));
      await key('Escape');
      await js("Input.setLayout('xbox')");
      check('L2', 'Xbox layout shows A to select', await q(`document.querySelector('#hints .key .xb').textContent === 'A' && getComputedStyle(document.querySelector('#hints .key .xb')).display !== 'none'`));
      await js("Input.setLayout('auto')");

      // ---- Item options + cover ----
      await key('Q', 5);
      await key('Down');
      await key('M');
      check('I1', 'Triangle opens the item menu', await layerOpen('menu'));
      await key('Down', 4);
      await key('Return');
      await shot('24-cover', 2000);
      check('I2', 'Change cover opens the picture picker', await layerOpen('cover'));
      await key('Escape');

      // ---- Keyboard ----
      await key('F');
      require('electron').clipboard.writeText('1449231011723214');
      await key('Down', 2); await key('Right', 2); await pad([0]);
      await wait(300);
      check('K1', 'On-screen Paste key types the clipboard', await q(`document.querySelector('.kb-text').textContent === '1449231011723214'`));
      await key('Escape');
      await key('F');
      await key('V', 1, ['control']);
      await wait(400);
      check('K2', 'Ctrl+V pastes into the keyboard', await q(`document.querySelector('.kb-text').textContent === '1449231011723214'`));
      await shot('26-keyboard');
      await key('Escape');
      await key('F');
      await key('S'); await key('T');
      check('K3', 'Typing in search finds library items', await q(`document.querySelectorAll('.kb-results .tile').length > 0`), await q(`document.querySelector('.kb-title').textContent`));
      await shot('26b-search');
      await key('Escape');

      // ---- Controller headphones (simulated jack; the QA run never changes real devices) ----
      const toastTitle = () => q(`document.querySelector('#toast').hidden ? '' : document.querySelector('#toast b').textContent`);
      await js(`DS.test('jack', { headphones: true, mic: false, bt: true })`);
      await wait(400);
      check('A1', 'Headphones on a Bluetooth controller explain the USB cable', (await toastTitle()) === 'Headphones need a USB cable');
      await shot('29-headphones-bt', 200);
      await js(`DS.test('jack', { headphones: true, mic: false, bt: false })`);
      await wait(600);
      check('A2', 'Headphones on a USB controller move the sound there', (await toastTitle()) === 'Sound on the controller');
      await js(`DS.test('jack', { headphones: false, mic: false, bt: false })`);
      await wait(600);
      check('A3', 'Unplugging gives the sound back', /^Sound back on/.test(await toastTitle()), await toastTitle());

      // ---- Input ----
      const dbl = await js(`(() => {
        const before = document.querySelector('.tile.focus')?.dataset.key;
        window.__fake = [15];
        return new Promise((r) => setTimeout(() => {
          window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
          setTimeout(() => { window.__fake = []; r([before, document.querySelector('.tile.focus')?.dataset.key]); }, 150);
        }, 60));
      })()`);
      const tiles = await q(`[...document.querySelectorAll('#shelf .tile')].map((t) => t.dataset.key)`);
      const moved = tiles.indexOf(dbl[1]) - tiles.indexOf(dbl[0]);
      check('N1', 'A press that arrives as pad and key moves once', moved === 1, `moved ${moved}`);

      check('E1', 'No errors in the console', errors.length === 0, errors.slice(0, 3).join(' | '));
    } catch (e) {
      results.push('FAIL  X  Run crashed: ' + (e && e.stack || e));
    }
    const failed = results.filter((r) => r.startsWith('FAIL')).length;
    results.push('', `${results.length - failed} passed, ${failed} failed`);
    fs.writeFileSync(path.join(out, 'qa.txt'), results.join('\n') + '\n');
    require('electron').app.exit(failed ? 1 : 0);
  });
};
