// Tiny JSON store kept in Sofa's userData folder.
const fs = require('fs');
const path = require('path');

const DEFAULTS = {
  setupDone: false,
  settings: {
    launchAtLogin: true, // applied when setup finishes, after the user has seen it
    fullscreen: true,
    keepInFront: true, // console-style: Sofa returns whenever nothing else is open
    homeButton: 'ps', // 'ps' | 'combo' (Create + Options, for when Steam keeps the PS button)
    controllerLayout: 'auto', // 'auto' | 'ps' | 'xbox' | 'nintendo'
    steamGuide: true, // keep Steam's "Guide button focuses Steam" off (src/steamguide.js)
    padAudio: true, // headphones in the controller take the sound (USB only on Windows)
    controllerLight: 'follow', // 'follow' (the selected game's colour) | 'amber' | 'off'
    browser: 'auto', // 'auto' | 'firefox' | 'chrome' | 'edge'
    sounds: true,
    motion: 'full', // 'full' | 'reduced'
    showHidden: false,
  },
  overrides: {}, // itemId -> { category?, hidden?, name?, favorite? }
  recent: {}, // itemId -> last launch timestamp (ms)
};

class Store {
  constructor(dir) {
    this.file = path.join(dir, 'sofa.json');
    this.data = structuredClone(DEFAULTS);
    try {
      const saved = JSON.parse(fs.readFileSync(this.file, 'utf8'));
      this.data = { ...this.data, ...saved, settings: { ...DEFAULTS.settings, ...(saved.settings || {}) } };
    } catch { /* first run */ }
    this.timer = null;
  }

  save() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.flush(), 300);
  }

  flush() {
    clearTimeout(this.timer);
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    fs.writeFileSync(this.file, JSON.stringify(this.data, null, 2));
  }
}

module.exports = { Store };
