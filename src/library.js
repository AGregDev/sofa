// Builds Sofa's library: Steam games, every Start-menu app, streaming services
// and system actions, each sorted into a category.
const fs = require('fs');
const path = require('path');
const { execFile, execFileSync } = require('child_process');
const { parseVdf, get } = require('./vdf');

// ---------- Steam ----------

function steamRoot() {
  try {
    const out = execFileSync('reg', ['query', 'HKCU\\Software\\Valve\\Steam', '/v', 'SteamPath'], { encoding: 'utf8' });
    const m = out.match(/SteamPath\s+REG_SZ\s+(.+)/);
    if (m) return path.normalize(m[1].trim());
  } catch { /* Steam not installed */ }
  const fallback = 'C:\\Program Files (x86)\\Steam';
  return fs.existsSync(fallback) ? fallback : null;
}

const STEAM_SKIP = /redistributable|steamworks common|proton|steam linux runtime|steamvr|dedicated server|\bsdk\b/i;
const STEAM_SKIP_IDS = new Set(['228980', '1070560', '1391110', '1628350']);

function readVdf(file) {
  try { return parseVdf(fs.readFileSync(file, 'utf8')); } catch { return null; }
}

function steamLastPlayed(root) {
  const result = {};
  const userdata = path.join(root, 'userdata');
  let users = [];
  try { users = fs.readdirSync(userdata); } catch { return result; }
  for (const u of users) {
    const cfg = readVdf(path.join(userdata, u, 'config', 'localconfig.vdf'));
    const apps = get(get(get(get(get(cfg, 'UserLocalConfigStore'), 'Software'), 'Valve'), 'Steam'), 'apps');
    if (!apps) continue;
    for (const [id, a] of Object.entries(apps)) {
      const t = Number(get(a, 'LastPlayed') || 0) * 1000;
      if (t > (result[id] || 0)) result[id] = t;
    }
  }
  return result;
}

function steamArt(root, appid) {
  const dir = path.join(root, 'appcache', 'librarycache', appid);
  const art = {};
  let files = [];
  try { files = fs.readdirSync(dir, { recursive: true }); } catch { return art; }
  const want = { 'library_600x900.jpg': 'capsule', 'library_capsule.jpg': 'capsule', 'library_hero.jpg': 'hero', 'logo.png': 'logo', 'header.jpg': 'header', 'library_header.jpg': 'header' };
  for (const f of files) {
    const key = want[path.basename(String(f)).toLowerCase()];
    if (key && !art[key]) art[key] = path.join(dir, String(f));
  }
  return art;
}

function scanSteam() {
  const root = steamRoot();
  if (!root) return { root: null, items: [] };
  const lf = readVdf(path.join(root, 'steamapps', 'libraryfolders.vdf'));
  const libs = new Set([root]);
  const folders = get(lf, 'libraryfolders') || {};
  for (const v of Object.values(folders)) {
    const p = get(v, 'path');
    if (p) libs.add(path.normalize(p));
  }
  const played = steamLastPlayed(root);
  const items = [];
  const seen = new Set();
  for (const lib of libs) {
    const apps = path.join(lib, 'steamapps');
    let files = [];
    try { files = fs.readdirSync(apps).filter((f) => /^appmanifest_\d+\.acf$/.test(f)); } catch { continue; }
    for (const f of files) {
      const st = get(readVdf(path.join(apps, f)), 'AppState');
      const appid = String(get(st, 'appid') || '');
      const name = get(st, 'name');
      if (!appid || !name || seen.has(appid) || STEAM_SKIP_IDS.has(appid) || STEAM_SKIP.test(name)) continue;
      seen.add(appid);
      const installdir = get(st, 'installdir');
      items.push({
        id: `steam:${appid}`,
        name,
        source: 'steam',
        category: /wallpaper engine/i.test(name) ? 'apps' : 'games',
        launch: { type: 'url', url: `steam://rungameid/${appid}` },
        art: steamArt(root, appid),
        installPath: installdir ? path.join(apps, 'common', installdir).toLowerCase() : null,
        lastPlayed: played[appid] || 0,
      });
    }
  }
  return { root, items };
}

// ---------- RecompLauncher (N64 recompilations and PC ports) ----------

// The launcher shows project names; these are the games people know them as.
const RECOMP_TITLES = {
  longshot: 'The Legend of Zelda: Ocarina of Time',
  shipwright: 'The Legend of Zelda: Ocarina of Time',
  zelda64recomp: "The Legend of Zelda: Majora's Mask",
  '2ship2harkinian': "The Legend of Zelda: Majora's Mask",
  spaghettikart: 'Mario Kart 64',
  starship: 'Star Fox 64',
  banjorecomp: 'Banjo-Kazooie',
  'donkey-kong-64-recompiled': 'Donkey Kong 64',
  'perfect-dark': 'Perfect Dark',
  unleashedrecomp: 'Sonic Unleashed',
};

function scanRecomp() {
  const dataDir = path.join(process.env.APPDATA || '', 'com.recomplauncher.app');
  const launcher = path.join(process.env.LOCALAPPDATA || '', 'RecompLauncher', 'recomplauncher.exe');
  let lib;
  try { lib = JSON.parse(fs.readFileSync(path.join(dataDir, 'library.json'), 'utf8')); } catch { return []; }
  if (!fs.existsSync(launcher) || !lib || !lib.installed) return [];
  const items = [];
  for (const g of Object.values(lib.installed)) {
    if (!g || !g.id || !g.path) continue;
    // The game's own exe sits in its folder or one level down; it supplies the icon.
    let gameExe = null;
    if (g.exe) {
      const direct = path.join(g.path, g.exe);
      const nested = (g.files || []).find((f) => path.basename(f).toLowerCase() === g.exe.toLowerCase());
      gameExe = fs.existsSync(direct) ? direct : nested ? path.join(g.path, nested) : null;
    }
    const cover = g.cover_file ? (path.isAbsolute(g.cover_file) ? g.cover_file : path.join(dataDir, 'covers', g.cover_file)) : null;
    items.push({
      id: `recomp:${g.id}`,
      name: RECOMP_TITLES[g.id] || g.name || g.id,
      project: g.name,
      source: 'recomp',
      category: 'games',
      // Through the launcher, so its setup checks and play time still apply.
      launch: { type: 'exec', exe: launcher, args: ['--play', g.id] },
      art: cover && fs.existsSync(cover) ? { capsule: cover } : {},
      iconFrom: gameExe,
      installPath: path.normalize(g.path).toLowerCase(),
      lastPlayed: Number(g.last_played || 0) * 1000,
    });
  }
  return items;
}

// ---------- Start menu (shell:AppsFolder) ----------

function scanStartApps(scriptsDir, cacheDir) {
  const out = path.join(cacheDir, 'startapps.json');
  const icons = path.join(cacheDir, 'icons');
  return new Promise((resolve) => {
    execFile('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File',
      path.join(scriptsDir, 'apps.ps1'), '-OutFile', out, '-IconDir', icons],
    { windowsHide: true, timeout: 120000 }, () => {
      try { resolve(JSON.parse(fs.readFileSync(out, 'utf8').replace(/^\uFEFF/, ''))); } catch { resolve([]); }
    });
  });
}

const HIDE = new RegExp([
  'uninstall', 'readme', 'read me', 'documentation', 'manual', 'release notes', 'samples', 'reference', 'faq',
  'website', '\\bhelp\\b', 'support center', 'localization', 'updater', 'error report', 'bug report', 'crash',
  'install additional', 'command prompt', 'powershell', '^run$', 'registry', '^services$', 'task scheduler',
  'event viewer', 'performance monitor', 'resource monitor', 'system configuration', 'system information',
  'computer management', 'component services', 'disk cleanup', 'defragment', 'iscsi', 'odbc', 'local security',
  'memory diagnostic', 'firewall', 'recovery drive', 'windows tools', 'character map', 'magnifier', 'narrator',
  'voice access', 'live captions', 'on-screen keyboard', 'app recovery', 'check for updates', 'get started',
  'click to do', 'control panel', 'about java', 'configure java', 'desktop overlay host', '^idle', '^python',
  'node\\.js', '^git ', 'private browsing', 'skinned', 'reset preferences', 'remote desktop', 'module docs',
  'windows backup', '^terminal$', 'snipping tool', 'game bar', 'phone link', 'translucenttb',
].join('|'), 'i');

const GAME_NAMES = /^ea$|riot client|league of legends|valorant|rockstar games launcher|minecraft|ankama|epic games|ubisoft connect|battle\.net|gog galaxy|^xbox$|recomplauncher|playnite|heroic|itch|retroarch|dolphin|pcsx2|rpcs3|ryujinx|cemu|ppsspp|duckstation|launcher$/i;
const GAME_PATHS = /\\(games|steamlibrary|steamapps|epic games|gog games|riot games)\\/i;
const MEDIA_NAMES = /spotify|vlc|netflix|plex|kodi|jellyfin|disney|prime video|youtube|twitch|^photos$|media player|movies & tv|apple tv|apple music|itunes|crunchyroll|hulu|^max$|deezer|tidal|amazon music|mpc-|potplayer|stremio|audible|soundcloud/i;
const SKIP_DIRS = new Set(['bin', 'binaries', 'x64', 'win64', 'win32', 'game', 'retail', 'common']);

function friendlyName(name, appId) {
  const clean = name.replace(/[®™]/g, '').replace(/\uA789/g, ':').replace(/\s+/g, ' ').trim();
  const looksLikeExe = !/\s/.test(clean) && (/_/.test(clean) || clean.length <= 4);
  if (!looksLikeExe || !/[\\/]/.test(appId)) return clean;
  const parts = appId.split(/[\\/]/).slice(0, -1).reverse();
  const dir = parts.find((p) => p && !SKIP_DIRS.has(p.toLowerCase()) && !/^\{/.test(p) && !/:$/.test(p));
  return dir || clean;
}

function categorizeStart(app) {
  const id = app.appId || '';
  const name = app.name || '';
  if (/^(https?|steam):/i.test(id) || /\.(url|pdf|txt|html?|chm|msc)$/i.test(id) || HIDE.test(name)) return 'hidden';
  if (MEDIA_NAMES.test(name)) return 'media';
  if (GAME_NAMES.test(name) || GAME_PATHS.test(id)) return 'games';
  return 'apps';
}

// ---------- Streaming + system ----------

const WEB = [
  { key: 'youtube', name: 'YouTube', url: 'https://www.youtube.com/tv', inApp: true, color: '#ff2d55' },
  { key: 'netflix', name: 'Netflix', url: 'https://www.netflix.com', color: '#e50914' },
  { key: 'disney', name: 'Disney+', url: 'https://www.disneyplus.com', color: '#1f5bff' },
  { key: 'prime', name: 'Prime Video', url: 'https://www.primevideo.com', color: '#00a8e1' },
  { key: 'twitch', name: 'Twitch', url: 'https://www.twitch.tv', color: '#9146ff' },
  { key: 'browser', name: 'Browser', url: null, category: 'apps', color: '#f0a24b' },
];

const SYSTEM = [
  { key: 'sleep', name: 'Sleep', color: '#6d7bff' },
  { key: 'restart', name: 'Restart', color: '#22c3ee', confirm: true },
  { key: 'shutdown', name: 'Shut down', color: '#ff4d6d', confirm: true },
  { key: 'desktop', name: 'Go to desktop', color: '#a78bfa' },
  { key: 'quit', name: 'Quit Sofa', color: '#8a847a' },
  { key: 'bigpicture', name: 'Steam Big Picture', color: '#1b9cff' },
  { key: 'settings', name: 'Sofa settings', color: '#ff7ab6' },
  { key: 'rescan', name: 'Rescan library', color: '#34d399' },
  { key: 'display', name: 'Display settings', color: '#fbbf24', uri: 'ms-settings:display' },
  { key: 'sound', name: 'Sound output', color: '#f472b6' },
  { key: 'bluetooth', name: 'Bluetooth & controllers', color: '#60a5fa', uri: 'ms-settings:bluetooth' },
];

// ---------- Colour ----------

function vibrantColor(nativeImage, file) {
  try {
    const img = nativeImage.createFromPath(file);
    if (img.isEmpty()) return null;
    const small = img.resize({ width: 12, height: 12, quality: 'good' });
    const buf = small.toBitmap(); // BGRA
    let x = 0, y = 0, wsum = 0;
    for (let i = 0; i < buf.length; i += 4) {
      const b = buf[i] / 255, g = buf[i + 1] / 255, r = buf[i + 2] / 255, a = buf[i + 3] / 255;
      const max = Math.max(r, g, b), min = Math.min(r, g, b);
      const sat = max === 0 ? 0 : (max - min) / max;
      const w = a * sat * sat * max + 0.0001;
      let h = 0;
      if (max !== min) {
        if (max === r) h = ((g - b) / (max - min)) % 6;
        else if (max === g) h = (b - r) / (max - min) + 2;
        else h = (r - g) / (max - min) + 4;
      }
      h *= Math.PI / 3;
      x += Math.cos(h) * w; y += Math.sin(h) * w; wsum += w;
    }
    const strength = Math.min(1, Math.hypot(x, y) / wsum);
    const hue = ((Math.atan2(y, x) * 180) / Math.PI + 360) % 360;
    const sat = strength < 0.15 ? 0.45 : 0.85;
    return `hsl(${Math.round(hue)} ${Math.round(sat * 100)}% 58%)`;
  } catch { return null; }
}

// ---------- Build ----------

async function buildLibrary({ scriptsDir, cacheDir, nativeImage, startApps }) {
  const steam = scanSteam();
  const apps = startApps || (await scanStartApps(scriptsDir, cacheDir));
  const recomp = scanRecomp();
  const steamPaths = [...steam.items, ...recomp].map((g) => g.installPath).filter(Boolean);
  const items = [...steam.items, ...recomp];
  const seenNames = new Set();

  for (const a of apps) {
    const id = a.appId || '';
    if (/^steam:/i.test(id)) continue; // Steam shortcuts duplicate the Steam scan
    const lid = id.toLowerCase();
    if (steamPaths.some((p) => lid.startsWith(p))) continue;
    const name = friendlyName(a.name || '', id);
    const key = name.toLowerCase();
    if (seenNames.has(key)) continue;
    seenNames.add(key);
    items.push({
      id: `start:${id}`,
      name,
      source: 'start',
      category: categorizeStart(a),
      launch: { type: 'appsfolder', appId: id },
      art: a.icon ? { icon: a.icon } : {},
      lastPlayed: 0,
    });
  }

  for (const w of WEB) {
    items.push({ id: `web:${w.key}`, name: w.name, source: 'web', category: w.category || 'media', launch: { type: 'web', url: w.url, inApp: !!w.inApp }, art: {}, color: w.color, brand: w.key, lastPlayed: 0 });
  }
  for (const s of SYSTEM) {
    items.push({ id: `sys:${s.key}`, name: s.name, source: 'system', category: 'system', launch: { type: 'system', action: s.key, uri: s.uri }, confirm: !!s.confirm, art: {}, color: s.color, glyph: s.key, lastPlayed: 0 });
  }

  // Colours are cached by file path because decoding images is the slow part.
  const colorFile = path.join(cacheDir, 'colors.json');
  let colors = {};
  try { colors = JSON.parse(fs.readFileSync(colorFile, 'utf8')); } catch { /* none yet */ }
  for (const it of items) {
    if (it.color) continue;
    const src = it.art.capsule || it.art.header || it.art.icon;
    if (!src) { it.color = '#7c6cff'; continue; }
    if (!(src in colors) && nativeImage) colors[src] = vibrantColor(nativeImage, src);
    it.color = colors[src] || '#7c6cff';
  }
  try { fs.writeFileSync(colorFile, JSON.stringify(colors)); } catch { /* cache only */ }

  return { steamRoot: steam.root, items, scannedAt: Date.now() };
}

module.exports = { buildLibrary, scanStartApps, categorizeStart, friendlyName };
