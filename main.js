const { app, BrowserWindow, ipcMain, protocol, net, shell, nativeImage, globalShortcut, dialog, clipboard } = require('electron');
const path = require('path');
const fs = require('fs');
const { spawn, execFile } = require('child_process');
const { pathToFileURL } = require('url');
const { Store } = require('./src/store');
const { buildLibrary } = require('./src/library');
const { Discord, REDIRECT } = require('./src/social');
const steamGuide = require('./src/steamguide');

const SHOTS = process.argv.includes('--shots');
if (SHOTS) {
  // Every QA run starts as a first run, from an empty profile.
  const qaProfile = path.join(app.getPath('temp'), 'sofa-shots');
  fs.rmSync(qaProfile, { recursive: true, force: true });
  app.setPath('userData', qaProfile);
  // The test window sits off-screen; keep Chromium painting and animating it anyway.
  app.commandLine.appendSwitch('disable-features', 'CalculateNativeWinOcclusion');
  app.commandLine.appendSwitch('disable-renderer-backgrounding');
  app.commandLine.appendSwitch('disable-background-timer-throttling');
}

protocol.registerSchemesAsPrivileged([
  { scheme: 'sofa', privileges: { standard: true, secure: true, supportFetchAPI: true, corsEnabled: true } },
]);

if (!SHOTS && !app.requestSingleInstanceLock()) app.quit();

const ROOT = __dirname;
const SCRIPTS = app.isPackaged ? path.join(process.resourcesPath, 'scripts') : path.join(ROOT, 'scripts');
let win = null;
let webWin = null;
let store = null;
let library = { items: [], steamRoot: null };
let cacheDir = '';
let pad = null;

// ---------- Library ----------

function view() {
  const o = store.data.overrides;
  const items = library.items.map((it) => {
    const ov = o[it.id] || {};
    return {
      ...it,
      name: ov.name || it.name,
      art: ov.cover ? { ...it.art, capsule: ov.cover, custom: true } : it.art,
      category: ov.hidden ? 'hidden' : ov.category || it.category,
      favorite: !!ov.favorite,
      lastPlayed: Math.max(it.lastPlayed || 0, store.data.recent[it.id] || 0),
    };
  });
  return { items, settings: store.data.settings, setupDone: store.data.setupDone, version: app.getVersion() };
}

function pushView() {
  if (win && !win.isDestroyed()) win.webContents.send('library', view());
}

// Saves a program's icon as Explorer shows it, up to 256 px (scripts/icon.ps1), falling
// back to Electron's 32 px file icon. Cached: each program is extracted once.
function saveIcon(exe, out) {
  if (fs.existsSync(out)) return Promise.resolve(true);
  fs.mkdirSync(path.dirname(out), { recursive: true });
  return new Promise((resolve) => execFile('powershell.exe',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(SCRIPTS, 'icon.ps1'), '-Path', exe, '-Out', out],
    { windowsHide: true, timeout: 20000 }, async () => {
      if (fs.existsSync(out)) return resolve(true);
      try { fs.writeFileSync(out, (await app.getFileIcon(exe, { size: 'large' })).toPNG()); resolve(true); } catch { resolve(false); }
    }));
}

let scanning = null;
async function rescan({ fresh = true } = {}) {
  if (scanning) return scanning;
  scanning = (async () => {
    let startApps = null;
    if (!fresh) {
      try { startApps = JSON.parse(fs.readFileSync(path.join(cacheDir, 'startapps.json'), 'utf8').replace(/^ï»¿/, '')); } catch { /* scan */ }
    }
    library = await buildLibrary({ scriptsDir: SCRIPTS, cacheDir, nativeImage, startApps });
    // The Browser tile shows the browser Sofa will use.
    const browser = chosenBrowser();
    library.items = library.items.filter((it) => it.id !== 'web:browser' || browser);
    for (const it of library.items) {
      if (it.id === 'web:browser') { it.iconFrom = browser.exe; it.browser = browser.name; it.art = {}; }
      else if (it.source === 'web' && !it.launch.inApp) it.browser = browser ? browser.name : null;
    }
    // Games without cover art (like RecompLauncher's) use their own program icon, at the
    // 256 px size Windows keeps for Explorer, so it stays sharp in the hero.
    for (const it of library.items) {
      if (!it.iconFrom || it.art.capsule || it.art.icon) continue;
      const out = path.join(cacheDir, 'icons', `${it.id.replace(/[^a-z0-9]/gi, '_')}-${path.basename(it.iconFrom, '.exe')}-256.png`);
      if (await saveIcon(it.iconFrom, out)) it.art.icon = out;
    }
    fs.writeFileSync(path.join(cacheDir, 'library.json'), JSON.stringify(library));
    pushView();
    return view();
  })();
  try { return await scanning; } finally { scanning = null; }
}

// ---------- Custom covers ----------

const IMAGE_EXT = /\.(jpe?g|png|webp|gif|bmp)$/i;

// Folders Sofa offers pictures from when choosing a cover.
function pictureRoots() {
  return ['pictures', 'downloads', 'desktop'].map((n) => { try { return app.getPath(n); } catch { return null; } }).filter(Boolean);
}

function listPictures() {
  const found = [];
  const walk = (dir, depth) => {
    let entries = [];
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const e of entries) {
      const p = path.join(dir, e.name);
      if (e.isDirectory() && depth > 0 && !e.name.startsWith('.')) walk(p, depth - 1);
      else if (e.isFile() && IMAGE_EXT.test(e.name)) {
        try { found.push({ path: p, t: fs.statSync(p).mtimeMs }); } catch { /* skip */ }
      }
    }
  };
  for (const r of pictureRoots()) walk(r, 2);
  return found.sort((a, b) => b.t - a.t).slice(0, 120).map((f) => f.path);
}

function setCover(id, src) {
  const overrides = store.data.overrides;
  const cur = overrides[id] || {};
  if (cur.cover && inside(cur.cover, path.join(cacheDir, 'covers'))) { try { fs.unlinkSync(cur.cover); } catch { /* gone */ } }
  if (src) {
    const dir = path.join(cacheDir, 'covers');
    fs.mkdirSync(dir, { recursive: true });
    const out = path.join(dir, `${id.replace(/[^a-z0-9]/gi, '_')}-${Date.now()}${path.extname(src).toLowerCase()}`);
    fs.copyFileSync(src, out);
    overrides[id] = { ...cur, cover: out };
  } else {
    delete cur.cover;
    if (Object.keys(cur).length) overrides[id] = cur; else delete overrides[id];
  }
  store.save();
  return view();
}

// ---------- Protocol: sofa://app/... for UI files, sofa://img/?p=... for artwork ----------

function inside(p, dir) {
  if (!dir) return false;
  const rel = path.relative(dir, p);
  return rel && !rel.startsWith('..') && !path.isAbsolute(rel);
}

function registerProtocol() {
  protocol.handle('sofa', (req) => {
    const u = new URL(req.url);
    if (u.host === 'app') {
      const p = path.normalize(path.join(ROOT, decodeURIComponent(u.pathname)));
      if (!inside(p, path.join(ROOT, 'renderer')) && !inside(p, path.join(ROOT, 'assets'))) return new Response('', { status: 403 });
      return net.fetch(pathToFileURL(p).toString());
    }
    if (u.host === 'img') {
      const p = path.normalize(u.searchParams.get('p') || '');
      const steamCache = library.steamRoot && path.join(library.steamRoot, 'appcache', 'librarycache');
      const recompCovers = path.join(process.env.APPDATA || '', 'com.recomplauncher.app', 'covers');
      const roots = [cacheDir, steamCache, recompCovers, ...pictureRoots()];
      if (!roots.some((r) => inside(p, r)) || !IMAGE_EXT.test(p)) return new Response('', { status: 403 });
      return net.fetch(pathToFileURL(p).toString());
    }
    return new Response('', { status: 404 });
  });
}

// ---------- Launching ----------

// ---------- Browsers ----------

const BROWSER_NAMES = { firefox: 'Firefox', chrome: 'Chrome', edge: 'Edge' };

function installedBrowsers() {
  const pf = process.env.ProgramFiles, pf86 = process.env['ProgramFiles(x86)'], local = process.env.LOCALAPPDATA;
  const find = (list) => list.find((c) => c && fs.existsSync(c)) || null;
  return {
    firefox: find([pf && path.join(pf, 'Mozilla Firefox', 'firefox.exe'), pf86 && path.join(pf86, 'Mozilla Firefox', 'firefox.exe'),
      local && path.join(local, 'Mozilla Firefox', 'firefox.exe')]),
    chrome: find([pf && path.join(pf, 'Google', 'Chrome', 'Application', 'chrome.exe'), pf86 && path.join(pf86, 'Google', 'Chrome', 'Application', 'chrome.exe'),
      local && path.join(local, 'Google', 'Chrome', 'Application', 'chrome.exe')]),
    edge: find([pf86 && path.join(pf86, 'Microsoft', 'Edge', 'Application', 'msedge.exe'), pf && path.join(pf, 'Microsoft', 'Edge', 'Application', 'msedge.exe')]),
  };
}

// The browser Sofa uses: the one picked in settings, else Firefox, Chrome, then Edge.
function chosenBrowser() {
  const all = installedBrowsers();
  const pick = store.data.settings.browser;
  const key = (pick && pick !== 'auto' && all[pick]) ? pick : ['firefox', 'chrome', 'edge'].find((k) => all[k]);
  return key ? { key, exe: all[key], name: BROWSER_NAMES[key] } : null;
}

// Firefox gets its own TV profile: kiosk full screen, no first-run pages, DRM on for streaming.
function firefoxProfile() {
  const dir = path.join(app.getPath('userData'), 'firefox');
  fs.mkdirSync(dir, { recursive: true });
  const prefs = {
    'browser.aboutwelcome.enabled': false,
    'browser.shell.checkDefaultBrowser': false,
    'browser.startup.homepage_override.mstone': 'ignore',
    'datareporting.policy.dataSubmissionPolicyBypassNotification': true,
    'toolkit.telemetry.reportingpolicy.firstRun': false,
    'trailhead.firstrun.didSeeAboutWelcome': true,
    'media.eme.enabled': true,
    'media.gmp-widevinecdm.enabled': true,
    'media.gmp-widevinecdm.visible': true,
    'full-screen-api.warning.timeout': 0,
    'browser.tabs.warnOnClose': false,
    'browser.sessionstore.resume_from_crash': false,
    'layout.css.devPixelsPerPx': '1.5',
  };
  const lines = Object.entries(prefs).map(([k, v]) => `user_pref("${k}", ${JSON.stringify(v)});`);
  fs.writeFileSync(path.join(dir, 'user.js'), lines.join('\n') + '\n');
  return dir;
}

function openBrowserApp(url) {
  const b = chosenBrowser();
  if (!b) return shell.openExternal(url || 'https://duckduckgo.com');
  if (b.key === 'firefox') {
    spawn(b.exe, ['-profile', firefoxProfile(), '-no-remote', '--kiosk', url || 'about:home'], { detached: true, stdio: 'ignore' }).unref();
    return;
  }
  // A separate profile keeps TV logins apart from the desktop browser.
  spawn(b.exe, [`--app=${url || 'https://www.google.com'}`, '--start-fullscreen', `--user-data-dir=${path.join(app.getPath('userData'), 'browser')}`],
    { detached: true, stdio: 'ignore' }).unref();
}

// Installs Firefox from Mozilla through Windows' package manager (the user chose this in Sofa).
function installFirefox() {
  return new Promise((resolve) => {
    stepDown(); // the installer may show its own window or a Windows permission prompt
    execFile('winget', ['install', '--id', 'Mozilla.Firefox', '--exact', '--silent', '--accept-package-agreements', '--accept-source-agreements'],
      { windowsHide: true, timeout: 600000 }, () => {
        if (win) { win.show(); win.focus(); }
        resolve(!!installedBrowsers().firefox);
      });
  });
}

const TV_UA = 'Mozilla/5.0 (SMART-TV; LINUX; Tizen 6.5) AppleWebKit/537.36 (KHTML, like Gecko) 85.0.4183.93/6.5 TV Safari/537.36';

function openWebWindow(url, itemId) {
  if (webWin && !webWin.isDestroyed()) { webWin.focus(); return; }
  webWin = new BrowserWindow({
    fullscreen: true,
    show: false,
    backgroundColor: '#000000',
    autoHideMenuBar: true,
    webPreferences: { preload: path.join(ROOT, 'preload-web.js'), contextIsolation: true, sandbox: true, partition: 'persist:web' },
  });
  webWin.sofaItem = itemId;
  const webHwnd = webWin.getNativeWindowHandle().readBigUInt64LE(0).toString();
  webWin.webContents.setUserAgent(TV_UA);
  webWin.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  webWin.loadURL(url);
  webWin.once('ready-to-show', () => webWin.show());
  webWin.on('closed', () => {
    webWin = null;
    running = running.filter((r) => r.hwnd !== webHwnd);
    pushRunning();
    if (win) { win.show(); win.focus(); }
  });
}

function runSystem(action, uri) {
  switch (action) {
    case 'sleep':
      execFile('powershell.exe', ['-NoProfile', '-Command',
        'Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Application]::SetSuspendState("Suspend", $false, $false)'],
      { windowsHide: true });
      break;
    case 'restart': execFile('shutdown.exe', ['/r', '/t', '0'], { windowsHide: true }); break;
    case 'shutdown': execFile('shutdown.exe', ['/s', '/t', '0'], { windowsHide: true }); break;
    case 'quit': app.quit(); break;
    case 'desktop': goDesktop(); break;
    case 'bigpicture': shell.openExternal('steam://open/bigpicture'); break;
    default: if (uri) shell.openExternal(uri);
  }
}

// ---------- Staying on top ----------
// While Sofa is the home screen it sits above every normal window, so Steam (which
// also answers the PS button) can only open underneath it. Sofa steps down whenever
// it hands the screen to something else.

let topTimer = null;
function setTop(on) {
  if (!win || win.isDestroyed() || SHOTS) return;
  clearTimeout(topTimer);
  if (on && !store.data.settings.keepInFront) on = false;
  if (win.isAlwaysOnTop() !== on) win.setAlwaysOnTop(on, 'screen-saver');
}

// Hand the screen to another window: step down now, come back up if nothing took over.
function stepDown() {
  padCommand('allow 20000'); // a Steam game may bring Steam forward while it starts
  setTop(false);
  topTimer = setTimeout(() => { if (win && win.isFocused()) setTop(true); }, 15000);
}

function launch(id) {
  const it = library.items.find((i) => i.id === id);
  if (!it || SHOTS) return false;
  const L = it.launch;
  if (L.type === 'system') {
    if (L.uri || L.action === 'bigpicture') stepDown();
    runSystem(L.action, L.uri);
    return true;
  }
  stepDown();
  store.data.recent[id] = Date.now();
  store.save();
  if (L.type === 'url') shell.openExternal(L.url);
  else if (L.type === 'appsfolder') spawn('explorer.exe', [`shell:AppsFolder\\${L.appId}`], { detached: true, stdio: 'ignore' }).unref();
  else if (L.type === 'exec') spawn(L.exe, L.args || [], { detached: true, stdio: 'ignore', cwd: path.dirname(L.exe) }).unref();
  else if (L.type === 'web') { if (L.inApp) openWebWindow(L.url, id); else openBrowserApp(L.url); }
  setTimeout(pushView, 400);
  return true;
}

// ---------- Controller watcher (PS button â†’ back to Sofa) ----------

function startPadWatcher() {
  if (SHOTS) return;
  const hwnd = win.getNativeWindowHandle().readBigUInt64LE(0).toString();
  pad = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File',
    path.join(SCRIPTS, 'padwatch.ps1'), '-Sofa', hwnd], { windowsHide: true, stdio: ['pipe', 'pipe', 'ignore'] });
  let buf = '';
  pad.stdout.setEncoding('utf8');
  pad.stdout.on('data', (chunk) => {
    buf += chunk;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i).replace(/\r$/, '');
      buf = buf.slice(i + 1);
      onPadLine(line);
    }
  });
  pad.on('exit', () => { pad = null; });
  padCommand(`guard ${store.data.settings.keepInFront ? 'on' : 'off'}`);
  padCommand(`chord ${store.data.settings.homeButton === 'combo' ? 'combo' : 'ps'}`);
}

function padCommand(cmd) {
  if (pad && pad.stdin.writable) pad.stdin.write(cmd + '\n');
}

let aliveWaiters = [];
function onPadLine(line) {
  let m;
  if ((m = line.match(/^home (\d+)\t([^\t]*)\t(\d*)\t(.*)$/))) {
    const [, hwnd, title, pid, exe] = m;
    if (hwnd !== '0') addRunning({ hwnd, title, pid, exe });
    comeHome(hwnd !== '0' ? hwnd : null);
  } else if (line === 'steamgrab') {
    // Steam also answered the PS button; the UI offers the one-time Steam fix.
    if (win && !win.isDestroyed()) win.webContents.send('steam:grab');
  } else if ((m = line.match(/^ended (\d+)$/))) {
    running = running.filter((r) => r.hwnd !== m[1]);
    pushRunning();
  } else if ((m = line.match(/^alive (.*)$/))) {
    const live = new Set(m[1].split(',').filter(Boolean));
    const w = aliveWaiters.shift();
    if (w) w(live);
  }
}

// ---------- Running apps (the PS5-style "currently playing" card) ----------

let running = []; // [{ hwnd, title, exe, itemId, icon }]

const KNOWN_FOLDERS = {
  '{6D809377-6AF0-444B-8957-A3773F02200E}': process.env.ProgramFiles,
  '{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}': process.env['ProgramFiles(x86)'],
  '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}': path.join(process.env.SystemRoot || 'C:\\Windows', 'System32'),
  '{D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27}': path.join(process.env.SystemRoot || 'C:\\Windows', 'SysWOW64'),
  '{F38BF404-1D43-42F2-9305-67DE0B28FC23}': process.env.SystemRoot || 'C:\\Windows',
};

// Find the library item a running window belongs to, from its executable path.
function identify(exe, hwnd) {
  if (webWin && !webWin.isDestroyed() && webWin.getNativeWindowHandle().readBigUInt64LE(0).toString() === hwnd) return webWin.sofaItem || null;
  const e = (exe || '').toLowerCase();
  if (!e) return null;
  const base = path.basename(e, '.exe').replace(/[^a-z0-9]/g, '');
  for (const it of library.items) {
    if (it.installPath && e.startsWith(it.installPath + '\\')) return it.id;
    const L = it.launch || {};
    if (L.type !== 'appsfolder') continue;
    let id = L.appId;
    const g = id.match(/^(\{[0-9A-F-]+\})\\(.*)$/i);
    if (g && KNOWN_FOLDERS[g[1].toUpperCase()]) id = path.join(KNOWN_FOLDERS[g[1].toUpperCase()], g[2]);
    if (/^[a-z]:\\/i.test(id) && id.toLowerCase() === e) return it.id;
    const pkg = id.match(/^([^_!]+)_([a-z0-9]+)!/i); // Store apps: Name_publisher!App
    if (pkg && e.includes('\\windowsapps\\' + pkg[1].toLowerCase() + '_') && e.includes('__' + pkg[2].toLowerCase())) return it.id;
  }
  // Last resort: the executable is named like the app (discord.exe â†’ Discord).
  const byName = library.items.find((it) => it.source === 'start' && base.length > 2 && it.name.toLowerCase().replace(/[^a-z0-9]/g, '') === base);
  return byName ? byName.id : null;
}

async function addRunning(r) {
  const itemId = identify(r.exe, r.hwnd);
  let icon = null;
  if (!itemId && r.exe) {
    const out = path.join(cacheDir, 'icons', 'run-' + require('crypto').createHash('md5').update(r.exe.toLowerCase()).digest('hex') + '.png');
    try { if (await saveIcon(r.exe, out)) icon = nativeImage.createFromPath(out).toDataURL(); } catch { /* no icon */ }
  }
  running = [{ hwnd: r.hwnd, title: r.title, exe: r.exe, itemId, icon }, ...running.filter((x) => x.hwnd !== r.hwnd)];
  pushRunning();
}

function aliveCheck() {
  if (!pad || !running.length) return Promise.resolve(null);
  return new Promise((resolve) => {
    const t = setTimeout(() => { aliveWaiters = aliveWaiters.filter((w) => w !== done); resolve(null); }, 1500);
    const done = (live) => { clearTimeout(t); resolve(live); };
    aliveWaiters.push(done);
    padCommand('alive ' + running.map((r) => r.hwnd).join(' '));
  });
}

async function pruneRunning() {
  const live = await aliveCheck();
  if (live) running = running.filter((r) => live.has(r.hwnd));
  return running;
}

function pushRunning() {
  if (win && !win.isDestroyed()) win.webContents.send('running', running);
}

// "Go to desktop": Sofa steps aside but keeps listening, so the PS button brings it back.
let onDesktop = false;
function goDesktop() {
  if (!win || SHOTS) return;
  onDesktop = true;
  padCommand('guard off');
  setTop(false);
  win.minimize();
}

function comeHome(hwnd) {
  if (!win) return;
  if (onDesktop) {
    onDesktop = false;
    padCommand(`guard ${store.data.settings.keepInFront ? 'on' : 'off'}`);
  }
  if (win.isMinimized()) win.restore();
  win.show();
  win.focus();
  pruneRunning().then(() => win && win.webContents.send('home', { hwnd, running }));
}

// ---------- Window ----------

function createWindow() {
  const s = store.data.settings;
  win = new BrowserWindow({
    width: 1920,
    height: 1080,
    fullscreen: SHOTS ? false : s.fullscreen,
    show: false,
    frame: SHOTS ? false : !s.fullscreen,
    // Full screen, Sofa is the home screen rather than an app, so it stays off the taskbar.
    skipTaskbar: !SHOTS && s.fullscreen,
    backgroundColor: '#161513',
    autoHideMenuBar: true,
    title: 'Sofa',
    icon: path.join(ROOT, 'assets', 'icon.png'),
    webPreferences: {
      preload: path.join(ROOT, 'preload.js'),
      contextIsolation: true,
      sandbox: true,
      backgroundThrottling: !SHOTS,
    },
  });
  win.setMenu(null);
  // Let the UI talk to Sony controllers over HID (battery, lightbar, mute button).
  const ses = win.webContents.session;
  ses.setDevicePermissionHandler((d) => !SHOTS && d.deviceType === 'hid' && d.device.vendorId === 0x054c);
  ses.setPermissionCheckHandler(() => true);
  ses.on('select-hid-device', (event, details, callback) => {
    event.preventDefault();
    const d = details.deviceList.find((x) => x.vendorId === 0x054c);
    callback(d ? d.deviceId : '');
  });
  win.loadURL('sofa://app/renderer/index.html');
  if (!SHOTS) win.once('ready-to-show', () => win.show());
  else win.once('ready-to-show', () => { win.setPosition(-4000, 0); win.showInactive(); win.setSkipTaskbar(true); });
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  win.webContents.on('will-navigate', (e) => e.preventDefault());
  win.on('closed', () => { win = null; });
  // Back in front means Sofa is home again. Losing focus only steps it down if the
  // other window keeps it: Steam's brief grab after the PS button is undone sooner.
  win.on('focus', () => setTop(true));
  win.on('blur', () => {
    clearTimeout(topTimer);
    topTimer = setTimeout(() => { if (win && !win.isFocused()) setTop(false); }, 700);
  });
}

function ipc() {
  ipcMain.handle('boot', () => view());
  ipcMain.handle('rescan', () => rescan({ fresh: true }));
  ipcMain.handle('launch', (_e, id) => launch(id));
  ipcMain.handle('item:set', (_e, id, patch) => {
    const cur = store.data.overrides[id] || {};
    const next = { ...cur, ...patch };
    for (const k of Object.keys(next)) if (next[k] === null || next[k] === false || next[k] === '') delete next[k];
    if (Object.keys(next).length) store.data.overrides[id] = next; else delete store.data.overrides[id];
    store.save();
    return view();
  });
  ipcMain.handle('settings:set', (_e, patch) => {
    const s = Object.assign(store.data.settings, patch);
    if ('launchAtLogin' in patch && !SHOTS) app.setLoginItemSettings({ openAtLogin: !!s.launchAtLogin });
    if ('fullscreen' in patch && win && !SHOTS) { win.setFullScreen(!!s.fullscreen); win.setSkipTaskbar(!!s.fullscreen); }
    if ('keepInFront' in patch) { padCommand(`guard ${s.keepInFront ? 'on' : 'off'}`); setTop(!!s.keepInFront && win && win.isFocused()); }
    if ('browser' in patch) rescan({ fresh: false });
    if ('homeButton' in patch) padCommand(`chord ${s.homeButton === 'combo' ? 'combo' : 'ps'}`);
    store.save();
    return s;
  });
  ipcMain.handle('setup:done', () => {
    store.data.setupDone = true;
    store.save();
    if (!SHOTS) app.setLoginItemSettings({ openAtLogin: !!store.data.settings.launchAtLogin });
    return true;
  });
  // ---- Social ----
  const discord = new Discord(store, () => { if (win && !win.isDestroyed()) win.webContents.send('social:changed'); });
  const quiet = (p) => p.catch((e) => ({ error: e.message }));
  // Test runs never talk to the real Discord.
  ipcMain.handle('social:discord', () => (SHOTS ? { configured: false } : quiet(discord.state())));
  // Discord shows its "Authorize" prompt in its own window: step aside until it's answered.
  let authorizing = false;
  discord.onAuthorize = () => { authorizing = true; stepDown(); if (win) win.minimize(); };
  ipcMain.handle('social:discordApp', async (_e, clientId, secret) => {
    if (SHOTS) return { configured: false };
    discord.setApp(clientId, secret);
    const r = await quiet(discord.state(true));
    if (authorizing && win) { authorizing = false; win.restore(); win.show(); win.focus(); }
    return r;
  });
  ipcMain.handle('social:discordForget', () => { discord.forget(); return { configured: false }; });
  ipcMain.handle('social:discordConfig', () => ({ clientId: discord.cfg.clientId || '', hasSecret: !!discord.cfg.secret, signedIn: !!discord.cfg.token, redirect: REDIRECT }));
  ipcMain.handle('social:recentVoice', () => (SHOTS ? [] : discord.recent().catch(() => [])));
  ipcMain.handle('social:guild',(_e, id) => quiet(discord.guildVoice(String(id))));
  ipcMain.handle('social:join', (_e, id) => quiet(discord.join(id ? String(id) : null)));
  ipcMain.handle('social:voice', (_e, patch) => quiet(discord.setVoice({ ...(('mute' in patch) ? { mute: !!patch.mute } : {}), ...(('deaf' in patch) ? { deaf: !!patch.deaf } : {}) })));
  ipcMain.handle('social:open', (_e, url) => {
    const u = String(url);
    const ok = u === 'https://discord.com/developers/applications';
    if (!ok || SHOTS) return;
    stepDown();
    shell.openExternal(u);
  });
  // readText is asynchronous in this Electron version; await works either way.
  ipcMain.handle('clipboard:read', async () => String((await clipboard.readText()) || '').slice(0, 200));
  const audio = (args) => new Promise((resolve) => execFile('powershell.exe',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(SCRIPTS, 'audio.ps1'), ...args],
    { windowsHide: true, timeout: 20000 }, (err, out) => resolve(err ? null : String(out).trim())));
  ipcMain.handle('audio:list', async () => { try { return JSON.parse(await audio(['-List'])); } catch { return []; } });

  // Like a PS5: headphones in the controller take Windows sound and Discord voice, and
  // unplugging them gives both back. Windows only has the jack while the controller is
  // on USB, as an audio device named after it.
  const PAD_AUDIO = /wireless controller|dualsense/i;
  let padPrev = null; // { windows, windowsName, discord } to restore on unplug
  ipcMain.handle('audio:pad', async (_e, plugged) => {
    if (SHOTS) return plugged ? { ok: true, name: 'Wireless Controller', discord: false } : { ok: true, restored: 'Speakers' };
    let list = [];
    try { list = JSON.parse(await audio(['-List'])); } catch { /* no devices */ }
    if (plugged) {
      const pad = list.find((d) => PAD_AUDIO.test(d.name));
      if (!pad) return { ok: false, reason: 'no-device' };
      const cur = list.find((d) => d.default);
      if (!padPrev) padPrev = { windows: cur && cur.id !== pad.id ? cur.id : null, windowsName: cur && cur.id !== pad.id ? cur.name : '', discord: undefined };
      if (!pad.default) await audio(['-Set', pad.id]);
      const dc = await discord.setOutputMatching(PAD_AUDIO).catch(() => null);
      if (dc && padPrev.discord === undefined) padPrev.discord = dc.prev;
      return { ok: true, name: pad.name, discord: !!dc };
    }
    const prev = padPrev;
    padPrev = null;
    if (!prev) return { ok: true, restored: null };
    if (prev.windows && list.some((d) => d.id === prev.windows)) await audio(['-Set', prev.windows]);
    if (prev.discord !== undefined) await discord.setOutputDevice(prev.discord).catch(() => false);
    return { ok: true, restored: prev.windowsName || null };
  });
  // ---- Browser ----
  ipcMain.handle('browser:list', () => {
    const all = installedBrowsers();
    const c = chosenBrowser();
    return { installed: Object.keys(all).filter((k) => all[k]), chosen: c ? c.key : null, setting: store.data.settings.browser || 'auto' };
  });
  ipcMain.handle('browser:installFirefox', async () => {
    if (SHOTS) return false;
    const ok = await installFirefox();
    if (ok) { store.data.settings.browser = 'firefox'; store.save(); rescan({ fresh: true }); }
    return ok;
  });

  // ---- Uninstall ----
  const uninstallScript = (it, check) => new Promise((resolve) => execFile('powershell.exe',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(SCRIPTS, 'uninstall.ps1'),
      '-Name', it.name, '-AppId', it.launch.appId || '', ...(check ? ['-Check'] : [])],
    { windowsHide: true, timeout: 120000 }, (err, out) => resolve(err ? 'error ' + err.message : String(out).trim())));
  ipcMain.handle('uninstall:check', async (_e, id) => {
    const it = library.items.find((i) => i.id === id);
    if (!it) return { kind: 'none' };
    if (it.source === 'steam') return { kind: 'steam' };
    if (it.source === 'recomp') return { kind: 'recomp' };
    if (it.source !== 'start') return { kind: 'none' };
    const r = await uninstallScript(it, true);
    const m = r.match(/^(appx|program) (.*)$/);
    return m ? { kind: m[1], name: m[2] } : { kind: r === 'system' ? 'system' : 'notfound' };
  });
  ipcMain.handle('uninstall:run', async (_e, id) => {
    const it = library.items.find((i) => i.id === id);
    if (!it || SHOTS) return 'none';
    const later = () => setTimeout(() => rescan({ fresh: true }), 20000);
    if (it.source === 'steam') { stepDown(); shell.openExternal(`steam://uninstall/${it.id.slice(6)}`); later(); return 'started'; }
    if (it.source === 'recomp') { stepDown(); spawn(it.launch.exe, [], { detached: true, stdio: 'ignore' }).unref(); return 'started'; }
    if (it.source !== 'start') return 'none';
    const r = await uninstallScript(it, false);
    if (r === 'started') { stepDown(); later(); }
    if (r === 'removed') rescan({ fresh: true });
    return r;
  });
  ipcMain.handle('audio:mic',async (_e, mode) => audio(['-Mic', mode === 'toggle' ? 'toggle' : 'get']));
  ipcMain.handle('audio:set', async (_e, id) => {
    if (!/^\{[0-9.]+\}\.\{[0-9a-f-]+\}$/i.test(String(id))) return false;
    return (await audio(['-Set', String(id)])) === 'ok';
  });
  ipcMain.handle('covers:list', () => listPictures());
  ipcMain.handle('covers:set', (_e, id, src) => {
    if (!library.items.some((i) => i.id === id)) return view();
    if (src && (!IMAGE_EXT.test(src) || !fs.existsSync(src) || !pictureRoots().some((r) => inside(path.normalize(src), r)))) return view();
    return setCover(id, src || null);
  });
  ipcMain.handle('covers:browse', async (_e, id) => {
    const r = await dialog.showOpenDialog(win, {
      title: 'Choose a cover',
      defaultPath: pictureRoots()[0],
      properties: ['openFile'],
      filters: [{ name: 'Images', extensions: ['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp'] }],
    });
    if (r.canceled || !r.filePaths[0]) return null;
    return setCover(id, r.filePaths[0]);
  });
  ipcMain.handle('running:get', async () => pruneRunning());
  ipcMain.handle('running:resume', (_e, hwnd) => {
    if (!/^\d+$/.test(hwnd)) return;
    stepDown();
    padCommand(`focus ${hwnd}`);
  });
  ipcMain.handle('running:close', (_e, hwnd, force) => {
    if (!/^\d+$/.test(hwnd)) return;
    padCommand(`${force ? 'kill' : 'close'} ${hwnd}`);
    setTimeout(async () => { await pruneRunning(); pushRunning(); }, force ? 800 : 2000);
  });
  ipcMain.handle('desktop', () => goDesktop());
  // ---- Steam and the PS button (src/steamguide.js) ----
  ipcMain.handle('steam:guide', () => (SHOTS ? { installed: true, accounts: 1, off: false, running: true } : steamGuide.status(library.steamRoot)));
  ipcMain.handle('steam:guideFix', async () => {
    if (SHOTS) return { ok: true, changed: true, restarted: true };
    // Never restart Steam under a Steam game that is running.
    const game = running.map((r) => library.items.find((i) => i.id === identify(r.exe, r.hwnd))).find((it) => it && it.source === 'steam');
    if (game) return { ok: false, reason: 'game', name: game.name };
    return steamGuide.fix(library.steamRoot, { restart: true });
  });
  ipcMain.handle('setup:reset', () => { store.data.setupDone = false; store.save(); return true; });
  ipcMain.handle('open', (_e, uri) => {
    if (SHOTS) return; // the screenshot run never opens anything outside Sofa
    if (/^(ms-settings:|steam:\/\/)/i.test(String(uri))) { stepDown(); shell.openExternal(uri); }
  });
  ipcMain.on('pad:home', (_e, kind) => {
    const want = store.data.settings.homeButton === 'combo' ? 'combo' : 'ps';
    if (kind === want) padCommand('home');
  });
  ipcMain.on('pad', (_e, cmd) => {
    if (/^vol (up|down|mute)$/.test(String(cmd))) padCommand(cmd);
  });
  ipcMain.on('web:key', (e, key) => {
    if (!webWin || e.sender !== webWin.webContents) return;
    if (key === 'Exit') { webWin.close(); return; }
    if (!/^(Up|Down|Left|Right|Enter|Escape|Backspace|MediaPlayPause)$/.test(key)) return;
    webWin.webContents.sendInputEvent({ type: 'keyDown', keyCode: key });
    webWin.webContents.sendInputEvent({ type: 'keyUp', keyCode: key });
  });
}

// While Steam is closed (Sofa usually starts first at sign-in), turn off its "Guide
// button focuses Steam" setting so only Sofa answers the PS button. With Steam open the
// UI asks first, because the fix restarts Steam. Settings can turn this off.
async function keepSteamOffPsButton() {
  if (SHOTS || store.data.settings.steamGuide === false || !library.steamRoot) return;
  const r = await steamGuide.fix(library.steamRoot, { restart: false }).catch(() => null);
  if (r && r.ok && r.changed && win && !win.isDestroyed()) win.webContents.send('steam:fixed');
}

app.on('second-instance', () => comeHome(null));

app.whenReady().then(async () => {
  cacheDir = path.join(app.getPath('userData'), 'cache');
  fs.mkdirSync(cacheDir, { recursive: true });
  store = new Store(app.getPath('userData'));
  try { library = JSON.parse(fs.readFileSync(path.join(cacheDir, 'library.json'), 'utf8')); } catch { /* first run */ }
  registerProtocol();
  ipc();
  createWindow();
  win.webContents.once('did-finish-load', () => {
    startPadWatcher();
    // Show the cached library at once, then refresh it in the background.
    rescan({ fresh: !library.items.length ? false : true }).then(keepSteamOffPsButton).catch(() => {});
  });
  // Keyboard fallback for getting back to Sofa from anywhere.
  globalShortcut.register('Control+Alt+H', () => comeHome(null));
  if (SHOTS) require('./src/shots').run(win, store);
});

app.on('will-quit', () => {
  globalShortcut.unregisterAll();
  if (pad) pad.kill();
  if (store) store.flush();
});
app.on('window-all-closed', () => app.quit());
