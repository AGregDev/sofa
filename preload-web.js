// Runs inside the in-app web window (YouTube's TV interface). Turns controller
// input into the arrow / Enter / Escape keys that TV web apps understand.
const { ipcRenderer } = require('electron');

const MAP = { 12: 'Up', 13: 'Down', 14: 'Left', 15: 'Right', 0: 'Enter', 1: 'Escape', 9: 'MediaPlayPause' };
const held = {};
const repeatAt = {};
let circleSince = 0;

function send(key) { ipcRenderer.send('web:key', key); }

function poll(t) {
  const pads = navigator.getGamepads ? [...navigator.getGamepads()].filter(Boolean) : [];
  const p = pads[0];
  if (p) {
    const ax = p.axes[0] || 0, ay = p.axes[1] || 0;
    const dirs = { Up: ay < -0.6, Down: ay > 0.6, Left: ax < -0.6, Right: ax > 0.6 };
    const keys = {};
    for (const [i, key] of Object.entries(MAP)) keys[key] = keys[key] || !!(p.buttons[i] && p.buttons[i].pressed);
    for (const [k, v] of Object.entries(dirs)) keys[k] = keys[k] || v;
    // The PS button is handled by Sofa's background helper: it goes to Home and
    // keeps this window running as the current app.

    for (const [key, down] of Object.entries(keys)) {
      if (down && !held[key]) { send(key); repeatAt[key] = t + 380; }
      else if (down && /Up|Down|Left|Right/.test(key) && t > repeatAt[key]) { send(key); repeatAt[key] = t + 110; }
      held[key] = down;
    }
    // Hold Circle for a second to leave the web app.
    if (keys.Escape) { if (!circleSince) circleSince = t; else if (t - circleSince > 1000) { send('Exit'); circleSince = Infinity; } }
    else circleSince = 0;
  }
  requestAnimationFrame(poll);
}
requestAnimationFrame(poll);
