// Keeps Steam off the PS button. Steam brings itself forward on the PS (guide) button
// while its "Guide button focuses Steam" setting is on. Steam keeps that setting per
// account in userdata/<id>/config/localconfig.vdf as "Controller_CheckGuideButton"
// (steamui.dll maps controller_guide_button_focus_steam to that key; absent means on).
//
// Steam rewrites localconfig.vdf from memory when it exits, so the file is only changed
// while Steam is closed. A running Steam is shut down first and started again after,
// and only when the person has said yes.
const fs = require('fs');
const path = require('path');
const { execFile, spawn } = require('child_process');
const { parseVdf } = require('./vdf');

const KEY = 'Controller_CheckGuideButton';
const LINE = /^\t"Controller_CheckGuideButton"\t\t"(\d+)"/m;

function configFiles(steamRoot) {
  const root = path.join(steamRoot, 'userdata');
  let ids = [];
  try { ids = fs.readdirSync(root, { withFileTypes: true }).filter((d) => d.isDirectory() && /^\d+$/.test(d.name)).map((d) => d.name); } catch { return []; }
  return ids.map((id) => path.join(root, id, 'config', 'localconfig.vdf')).filter((f) => fs.existsSync(f));
}

// '0' (off), '1' (on) or null (never changed: Steam's default, on).
function readValue(text) {
  const m = text.match(LINE);
  return m ? m[1] : null;
}

// Returns the file's text with the setting turned off, checked by parsing it back.
function withGuideOff(text) {
  let out;
  if (LINE.test(text)) out = text.replace(LINE, `\t"${KEY}"\t\t"0"`);
  else {
    // A new top-level key goes just before the closing brace of the root block.
    const end = text.lastIndexOf('}');
    if (end < 0) throw new Error('localconfig.vdf has no root block');
    const nl = text.includes('\r\n') ? '\r\n' : '\n';
    const before = text.slice(0, end).replace(/[\t ]*$/, '');
    out = `${before}\t"${KEY}"\t\t"0"${nl}${text.slice(end)}`;
  }
  const root = parseVdf(out);
  const store = root.UserLocalConfigStore || root[Object.keys(root)[0]];
  if (!store || store[KEY] !== '0') throw new Error('Could not verify the change');
  // Every other top-level section must survive untouched.
  const was = parseVdf(text);
  const wasStore = was.UserLocalConfigStore || was[Object.keys(was)[0]] || {};
  for (const k of Object.keys(wasStore)) if (k !== KEY && !(k in store)) throw new Error('Change would drop ' + k);
  return out;
}

function setOff(file) {
  const text = fs.readFileSync(file, 'utf8');
  if (readValue(text) === '0') return false;
  const out = withGuideOff(text);
  const backup = file + '.sofa-backup';
  if (!fs.existsSync(backup)) fs.copyFileSync(file, backup);
  const tmp = file + '.sofa-tmp';
  fs.writeFileSync(tmp, out);
  fs.renameSync(tmp, file);
  return true;
}

function steamRunning() {
  return new Promise((resolve) => execFile('tasklist', ['/FI', 'IMAGENAME eq steam.exe', '/NH'], { windowsHide: true },
    (err, out) => resolve(!err && /steam\.exe/i.test(String(out)))));
}

async function waitForExit(ms, isRunning = steamRunning) {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    if (!(await isRunning())) return true;
    await new Promise((r) => setTimeout(r, 700));
  }
  return false;
}

// isRunning can be swapped out by tests.
async function status(steamRoot, { isRunning = steamRunning } = {}) {
  if (!steamRoot || !fs.existsSync(path.join(steamRoot, 'steam.exe'))) return { installed: false };
  const files = configFiles(steamRoot);
  const values = files.map((f) => { try { return readValue(fs.readFileSync(f, 'utf8')); } catch { return null; } });
  return { installed: true, accounts: files.length, off: files.length > 0 && values.every((v) => v === '0'), running: await isRunning() };
}

// restart: allowed to close and reopen a running Steam (the person agreed).
async function fix(steamRoot, { restart = false, isRunning = steamRunning } = {}) {
  const s = await status(steamRoot, { isRunning });
  if (!s.installed) return { ok: false, reason: 'no-steam' };
  if (!s.accounts) return { ok: false, reason: 'no-account' };
  if (s.off) return { ok: true, changed: false };
  const exe = path.join(steamRoot, 'steam.exe');
  if (s.running) {
    if (!restart) return { ok: false, reason: 'running' };
    execFile(exe, ['-shutdown'], { windowsHide: true }, () => {});
    if (!(await waitForExit(45000, isRunning))) return { ok: false, reason: 'still-running' };
    await new Promise((r) => setTimeout(r, 800)); // let Steam finish writing its files
  }
  let changed = 0;
  for (const f of configFiles(steamRoot)) {
    try { if (setOff(f)) changed++; } catch (e) { return { ok: false, reason: 'write', detail: e.message }; }
  }
  if (s.running) spawn(exe, ['-silent'], { detached: true, stdio: 'ignore' }).unref();
  return { ok: true, changed: changed > 0, restarted: s.running };
}

module.exports = { status, fix, withGuideOff, readValue };
