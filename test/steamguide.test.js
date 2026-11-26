// `npm test`: checks src/steamguide.js, which edits Steam's localconfig.vdf. Runs on a
// made-up config in a temp folder, never on a real Steam install.
const fs = require('fs');
const os = require('os');
const path = require('path');
const g = require('../src/steamguide');

const results = [];
const check = (id, name, ok, detail = '') => results.push(`${ok ? 'PASS' : 'FAIL'}  ${id}  ${name}${detail ? '  (' + detail + ')' : ''}`);

// Shaped like a real file: tab indents, LF line endings, nested sections, a long value.
const base = [
  '"UserLocalConfigStore"',
  '{',
  '\t"Broadcast"',
  '\t{',
  '\t\t"Permissions"\t\t"1"',
  '\t}',
  '\t"friends"',
  '\t{',
  '\t\t"PersonaName"\t\t"Player"',
  '\t\t"Notes"\t\t"a \\"quoted\\" word"',
  '\t}',
  '\t"ControllerTypesUsed"\t\t"controller_ps5,"',
  '\t"apps"',
  '\t{',
  '\t\t"413080"',
  '\t\t{',
  '\t\t\t"UseSteamControllerConfig"\t\t"2"',
  '\t\t}',
  '\t}',
  '}',
  '',
].join('\n');
const withKey = (v) => base.replace(/\}\n$/, `\t"Controller_CheckGuideButton"\t\t"${v}"\n}\n`);

check('G1', 'A file without the key reads as Steam\'s default', g.readValue(base) === null);
const inserted = g.withGuideOff(base);
check('G2', 'Turning it off adds the key with 0', g.readValue(inserted) === '0');
check('G3', 'Adding the key changes nothing else', inserted.replace('\t"Controller_CheckGuideButton"\t\t"0"\n', '') === base);
check('G4', 'A file with the setting on reads 1', g.readValue(withKey('1')) === '1');
check('G5', 'Turning it off replaces only the value', g.withGuideOff(withKey('1')) === withKey('0'));
const crlf = g.withGuideOff(base.replace(/\n/g, '\r\n'));
check('G6', 'Windows line endings are kept', g.readValue(crlf) === '0' && !/[^\r]\n/.test(crlf));
let refused = false;
try { g.withGuideOff('"UserLocalConfigStore"'); } catch { refused = true; }
check('G7', 'A file it cannot verify is refused', refused);

(async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'sofa-steamguide-'));
  try {
    const cfg = path.join(tmp, 'userdata', '42', 'config');
    fs.mkdirSync(cfg, { recursive: true });
    fs.writeFileSync(path.join(cfg, 'localconfig.vdf'), withKey('1'));
    fs.writeFileSync(path.join(tmp, 'steam.exe'), '');
    const open = { isRunning: async () => true };
    const closed = { isRunning: async () => false };
    const read = () => fs.readFileSync(path.join(cfg, 'localconfig.vdf'), 'utf8');
    const s = await g.status(tmp, closed);
    check('G8', 'Status finds the account with the setting on', s.installed && s.accounts === 1 && s.off === false);
    const r1 = await g.fix(tmp, { restart: false, ...open });
    check('G9', 'With Steam open and no permission, nothing changes', !r1.ok && r1.reason === 'running' && g.readValue(read()) === '1');
    const r2 = await g.fix(tmp, { restart: false, ...closed });
    check('G10', 'With Steam closed, the fix writes the file', r2.ok && r2.changed && !r2.restarted && g.readValue(read()) === '0');
    check('G11', 'The original file is kept as a backup', g.readValue(fs.readFileSync(path.join(cfg, 'localconfig.vdf.sofa-backup'), 'utf8')) === '1');
    check('G12', 'No temp file is left behind', !fs.existsSync(path.join(cfg, 'localconfig.vdf.sofa-tmp')));
    const r3 = await g.fix(tmp, { restart: false, ...closed });
    check('G13', 'Running it again changes nothing', r3.ok && !r3.changed);
    check('G14', 'Status then reports it off', (await g.status(tmp, closed)).off === true);
    check('G15', 'A missing Steam is reported, not guessed', (await g.status(path.join(tmp, 'nope'), closed)).installed === false);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  const failed = results.filter((l) => l.startsWith('FAIL')).length;
  console.log(results.join('\n') + `\n\n${results.length - failed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
