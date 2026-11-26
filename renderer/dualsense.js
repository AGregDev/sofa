// Talks to a DualSense directly over HID (no drivers): battery, lightbar, player
// light, the mute button and its LED. Report layouts follow Linux's hid-playstation.
// Sofa only writes to the controller while it is in front, so games keep control.
(function () {
  const SONY = 0x054c;
  const MODELS = [0x0ce6, 0x0df2]; // DualSense, DualSense Edge
  const listeners = {};
  let dev = null;
  let bt = false;
  let seq = 0;
  let muteHeld = false;
  let psHeld = false;
  let comboHeld = false;
  let hidPad = null;
  let rumbleUntil = 0;
  let lastSent = '';
  let flushTimer = null;
  let lightOutDone = false;
  const want = { r: 0, g: 0, b: 0, mute: false, motor: 0 };
  const state = { connected: false, battery: null, charging: false, headphones: null, mic: false, bt: false };
  let jackSeen;
  let jackTimer = null;

  // The headphone jack settles for a moment before it counts, so plugging in doesn't
  // flicker. The first reading after connecting only counts if something is plugged in.
  function settleJack(hp, mic) {
    if (hp === state.headphones) return;
    const first = state.headphones == null;
    state.headphones = hp;
    state.mic = mic;
    if (first && !hp) return;
    emit('jack', { headphones: hp, mic, bt });
  }

  const emit = (evt, v) => (listeners[evt] || []).forEach((fn) => fn(v));

  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  const crc32 = (u8) => {
    let c = 0xffffffff;
    for (const x of u8) c = table[(c ^ x) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };

  function onInput(e) {
    const v = new Uint8Array(e.data.buffer, e.data.byteOffset, e.data.byteLength);
    let off;
    if (e.reportId === 0x31) off = 1;          // Bluetooth, full mode
    else if (e.reportId === 0x01 && !bt && v.length >= 63) off = 0; // USB
    else return;                                // Bluetooth basic mode: no battery data
    const status = v[off + 52];
    const level = Math.min(100, (status & 0x0f) * 10 + 5);
    const hi = status >> 4;
    const charging = hi === 1 || hi === 2 || (status & 0x10) !== 0;
    if (level !== state.battery || charging !== state.charging) {
      state.battery = level;
      state.charging = charging;
      emit('battery', { ...state });
    }
    const mute = (v[off + 9] & 0x04) !== 0;
    if (mute && !muteHeld) emit('mute');
    muteHeld = mute;

    // Headphone jack: the status byte after the battery, bit 0 headphones, bit 1 a mic.
    const jack = v[off + 53];
    const hp = (jack & 0x01) !== 0;
    if (hp !== jackSeen) {
      jackSeen = hp;
      clearTimeout(jackTimer);
      jackTimer = setTimeout(() => settleJack(hp, (jack & 0x02) !== 0), 300);
    }

    // In full-report mode Windows' generic controller support (and so the browser's
    // Gamepad API) can't read the buttons, so Sofa builds the pad state itself,
    // in the standard gamepad button order.
    const b0 = v[off + 7], b1 = v[off + 8], b2 = v[off + 9];
    const hat = b0 & 0x0f;
    const up = hat === 7 || hat === 0 || hat === 1;
    const right = hat >= 1 && hat <= 3;
    const down = hat >= 3 && hat <= 5;
    const left = hat >= 5 && hat <= 7;
    const on = [
      b0 & 0x20, b0 & 0x40, b0 & 0x10, b0 & 0x80, // cross, circle, square, triangle
      b1 & 0x01, b1 & 0x02, b1 & 0x04, b1 & 0x08, // L1, R1, L2, R2
      b1 & 0x10, b1 & 0x20, b1 & 0x40, b1 & 0x80, // create, options, L3, R3
      up, down, left, right,
      b2 & 0x01, b2 & 0x02,                        // PS, touchpad
    ];
    const axis = (x) => (x - 128) / 128;
    hidPad = {
      id: 'DualSense Wireless Controller (HID)',
      connected: true,
      timestamp: performance.now(),
      axes: [axis(v[off]), axis(v[off + 1]), axis(v[off + 2]), axis(v[off + 3])],
      buttons: on.map((x) => ({ pressed: !!x, value: x ? 1 : 0 })),
      hid: true,
    };
    const ps = !!(b2 & 0x01);
    if (ps && !psHeld) emit('ps', 'ps');
    psHeld = ps;
    const combo = (b1 & 0x30) === 0x30; // Create + Options
    if (combo && !comboHeld) emit('ps', 'combo');
    comboHeld = combo;
  }

  async function connect() {
    if (dev || !navigator.hid) return;
    const list = (await navigator.hid.getDevices()).filter((d) => d.vendorId === SONY && MODELS.includes(d.productId));
    const d = list[0];
    if (!d) return;
    try {
      if (!d.opened) await d.open();
      bt = d.collections.some((c) => (c.outputReports || []).some((r) => r.reportId === 0x31));
      // Reading the calibration report switches a Bluetooth DualSense to full reports.
      if (bt) await d.receiveFeatureReport(0x05).catch(() => {});
      d.addEventListener('inputreport', onInput);
      dev = d;
      state.connected = true;
      state.bt = bt;
      lastSent = '';
      lightOutDone = false;
      emit('battery', { ...state });
      flush();
    } catch { dev = null; }
  }

  async function send() {
    if (!dev || !document.hasFocus()) return;
    const key = `${want.r},${want.g},${want.b},${want.mute},${want.motor}`;
    if (key === lastSent) return;
    const c = new Uint8Array(47);
    c[0] = 0x01;                    // rumble motors are being set
    c[2] = want.motor; c[3] = want.motor;
    c[1] = 0x01 | 0x04 | 0x10;      // mute LED, lightbar and player light are being set
    c[8] = want.mute ? 1 : 0;
    if (!lightOutDone) { c[38] = 0x02; c[41] = 0x02; } // fade out the blue start-up light once
    c[43] = 0x04;                   // player 1: the centre light
    c[44] = want.r; c[45] = want.g; c[46] = want.b;
    try {
      if (bt) {
        const data = new Uint8Array(77);
        data[0] = (seq << 4) & 0xf0;
        seq = (seq + 1) & 0x0f;
        data[1] = 0x10;
        data.set(c, 2);
        const pre = new Uint8Array(75);
        pre[0] = 0xa2; pre[1] = 0x31;
        pre.set(data.subarray(0, 73), 2);
        const crc = crc32(pre);
        data[73] = crc & 255; data[74] = (crc >>> 8) & 255; data[75] = (crc >>> 16) & 255; data[76] = (crc >>> 24) & 255;
        await dev.sendReport(0x31, data);
      } else {
        await dev.sendReport(0x02, c);
      }
      lastSent = key;
      lightOutDone = true;
    } catch { /* controller went away */ }
  }

  function flush() {
    clearTimeout(flushTimer);
    flushTimer = setTimeout(send, 60);
  }

  // Accepts '#rrggbb', 'rgb()' or 'hsl()' and dims it so the light stays calm.
  const probe = document.createElement('canvas').getContext('2d');
  function toRgb(color, scale = 0.7) {
    probe.fillStyle = '#000';
    probe.fillStyle = color;
    probe.fillRect(0, 0, 1, 1);
    const [r, g, b] = probe.getImageData(0, 0, 1, 1).data;
    return { r: Math.round(r * scale), g: Math.round(g * scale), b: Math.round(b * scale) };
  }

  if (navigator.hid) {
    navigator.hid.addEventListener('connect', () => connect());
    navigator.hid.addEventListener('disconnect', (e) => {
      if (e.device === dev) {
        dev = null;
        state.connected = false;
        state.battery = null;
        emit('battery', { ...state });
        // Headphones go with the controller: give the sound back.
        clearTimeout(jackTimer);
        jackSeen = undefined;
        if (state.headphones) emit('jack', { headphones: false, mic: false, bt });
        state.headphones = null;
      }
    });
  }
  window.addEventListener('focus', () => { lastSent = ''; flush(); });

  window.DS = {
    get state() { return { ...state }; },
    on(evt, fn) { (listeners[evt] ||= []).push(fn); },
    // For the QA run, which has no real controller.
    test(evt, v) { emit(evt, v); },
    setLight(color) {
      const c = color ? toRgb(color) : { r: 0, g: 0, b: 0 };
      want.r = c.r; want.g = c.g; want.b = c.b;
      flush();
    },
    setMuteLed(on) { want.mute = !!on; flush(); },
    // The controller as a gamepad object, while it is sending full reports.
    get pad() { return hidPad && performance.now() - hidPad.timestamp < 1000 ? hidPad : null; },
    rumble(ms = 20, strength = 0.25) {
      want.motor = Math.round(Math.min(1, strength) * 255);
      send();
      rumbleUntil = performance.now() + ms;
      setTimeout(() => { if (performance.now() >= rumbleUntil) { want.motor = 0; send(); } }, ms);
    },
    connect,
  };
  connect();
  setInterval(connect, 5000);
})();
