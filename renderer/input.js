// Turns controller and keyboard input into a small set of actions.
(function () {
  const BUTTONS = {
    0: 'accept', 1: 'back', 2: 'search', 3: 'menu', 4: 'tabPrev', 5: 'tabNext',
    6: 'volDown', 7: 'volUp', 9: 'options', 12: 'up', 13: 'down', 14: 'left', 15: 'right', 16: 'home',
  };
  const KEYS = {
    ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right',
    Enter: 'accept', Escape: 'back', Backspace: 'back', Home: 'home',
    m: 'menu', f: 'search', '/': 'search', o: 'options', q: 'tabPrev', e: 'tabNext',
  };
  const DIRS = new Set(['up', 'down', 'left', 'right']);

  let handler = () => {};
  let rawHandler = null;
  const held = {};
  const nextRepeat = {};
  const heldSince = {};
  let padType = 'ps';
  let lastPad = null;
  let layout = 'auto'; // 'auto' | 'ps' | 'xbox' | 'nintendo' (Settings → Button layout)

  function applyType(t) {
    padType = t;
    for (const k of ['ps', 'xbox', 'nintendo']) document.body.classList.toggle('pad-' + k, t === k);
  }

  function setPadType(id) {
    if (layout !== 'auto') return;
    const t = /dualsense|dualshock|054c|wireless controller|playstation/i.test(id) ? 'ps' : 'xbox';
    if (t !== padType) applyType(t);
  }

  // Steam's desktop controller layout also turns the pad into arrow keys, Enter and
  // Esc, so each press can arrive twice. Keep whichever copy lands first.
  let padActiveAt = -1e9;
  const lastKeyAt = {};
  const lastPadAt = {};

  function emit(action, source) {
    const now = performance.now();
    if (source === 'key') {
      if (held[action] || now - padActiveAt < 400 || now - (lastPadAt[action] || -1e9) < 250) return;
      lastKeyAt[action] = now;
    } else if (source === 'pad') {
      if (now - (lastKeyAt[action] || -1e9) < 250) return;
      lastPadAt[action] = now;
    }
    document.body.classList.add('no-cursor');
    handler(action, source);
  }

  function poll(t) {
    requestAnimationFrame(poll);
    if (!document.hasFocus()) { for (const k in held) held[k] = false; return; }
    // A DualSense read directly over HID wins: in its full-report mode the browser's
    // Gamepad API can't see its buttons at all.
    const hid = window.DS && window.DS.pad;
    const pads = hid ? [hid] : navigator.getGamepads ? [...navigator.getGamepads()].filter((p) => p && p.connected) : [];
    if (!pads.length) { lastPad = null; return; }
    // The most recently used pad drives the UI.
    const p = pads.reduce((a, b) => (b.timestamp > a.timestamp ? b : a));
    lastPad = p;
    setPadType(p.id);

    const state = {};
    // Nintendo layout: the right-hand face button confirms and the bottom one goes back.
    const swap = layout === 'nintendo';
    for (const [i, action] of Object.entries(BUTTONS)) {
      const b = p.buttons[swap && i === '0' ? 1 : swap && i === '1' ? 0 : i];
      if (b && (b.pressed || b.value > 0.5)) state[action] = true;
    }
    const ax = p.axes[0] || 0, ay = p.axes[1] || 0;
    if (ay < -0.55) state.up = true;
    if (ay > 0.55) state.down = true;
    if (ax < -0.55) state.left = true;
    if (ax > 0.55) state.right = true;

    if (Object.keys(state).length) padActiveAt = t;
    // Create + Options together is the alternative Home button.
    if (p.buttons[8] && p.buttons[8].pressed && p.buttons[9] && p.buttons[9].pressed) { state.home = true; delete state.options; }
    // A raw handler may swap itself out mid-loop (the setup step it belongs to ends).
    for (let i = 0; i < p.buttons.length && rawHandler; i++) rawHandler(i, p.buttons[i].pressed);

    for (const action of new Set([...Object.keys(BUTTONS).map((i) => BUTTONS[i]), 'up', 'down', 'left', 'right'])) {
      const down = !!state[action];
      if (down && !held[action]) {
        heldSince[action] = t;
        nextRepeat[action] = t + 360;
        emit(action, 'pad');
      } else if (down && DIRS.has(action) && t >= nextRepeat[action]) {
        // Repeat speeds up the longer a direction is held.
        nextRepeat[action] = t + (t - heldSince[action] > 1200 ? 60 : 110);
        emit(action, 'pad');
      }
      held[action] = down;
    }
  }

  window.addEventListener('keydown', (e) => {
    if (Input.textMode && ((e.ctrlKey && e.key.toLowerCase() === 'v') || (e.shiftKey && e.key === 'Insert'))) {
      handler('paste', 'key');
      e.preventDefault();
      return;
    }
    if (Input.textMode && e.key.length === 1 && !e.ctrlKey && !e.altKey) {
      handler('char', e.key);
      e.preventDefault();
      return;
    }
    if (Input.textMode && e.key === 'Backspace') {
      handler('erase', 'key');
      e.preventDefault();
      return;
    }
    const action = KEYS[e.key] || KEYS[e.key.toLowerCase?.()];
    if (!action) return;
    e.preventDefault();
    if (e.repeat && !DIRS.has(action)) return;
    emit(action, 'key');
  });

  let cursorTimer = null;
  window.addEventListener('mousemove', () => {
    document.body.classList.remove('no-cursor');
    clearTimeout(cursorTimer);
    cursorTimer = setTimeout(() => document.body.classList.add('no-cursor'), 2500);
  });

  window.Input = {
    textMode: false,
    on(fn) { handler = fn; },
    onRaw(fn) { rawHandler = fn; },
    get padType() { return padType; },
    // The physical button index that confirms (0, or 1 in the Nintendo layout).
    get confirmIndex() { return layout === 'nintendo' ? 1 : 0; },
    setLayout(l) {
      layout = ['ps', 'xbox', 'nintendo'].includes(l) ? l : 'auto';
      if (layout !== 'auto') applyType(layout);
      else if (lastPad) { padType = ''; setPadType(lastPad.id); } else applyType('ps');
    },
    get pad() { return lastPad; },
    rumble(ms = 18, strength = 0.25) {
      if (lastPad && lastPad.hid) return window.DS.rumble(ms, strength);
      const a = lastPad && lastPad.vibrationActuator;
      if (a && a.playEffect) a.playEffect('dual-rumble', { duration: ms, weakMagnitude: strength, strongMagnitude: strength * 0.4 }).catch(() => {});
    },
  };
  requestAnimationFrame(poll);
})();
