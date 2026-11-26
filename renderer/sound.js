// Quiet interface sounds, synthesised so there are no audio files to ship.
(function () {
  let ctx = null;
  let enabled = true;

  function tone(freq, start, dur, gain, type) {
    const t = ctx.currentTime + start;
    const o = ctx.createOscillator();
    const g = ctx.createGain();
    o.type = type || 'sine';
    o.frequency.setValueAtTime(freq, t);
    g.gain.setValueAtTime(0, t);
    g.gain.linearRampToValueAtTime(gain, t + 0.006);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    o.connect(g).connect(ctx.destination);
    o.start(t);
    o.stop(t + dur + 0.02);
  }

  const SOUNDS = {
    move: () => tone(1320, 0, 0.045, 0.018, 'triangle'),
    tab: () => { tone(660, 0, 0.06, 0.02, 'triangle'); tone(990, 0.03, 0.06, 0.015, 'triangle'); },
    select: () => { tone(740, 0, 0.08, 0.03); tone(1110, 0.05, 0.12, 0.025); },
    back: () => { tone(880, 0, 0.07, 0.025); tone(587, 0.04, 0.1, 0.02); },
    open: () => { tone(523, 0, 0.1, 0.02); tone(784, 0.05, 0.14, 0.02); },
    launch: () => { tone(392, 0, 0.18, 0.03); tone(587, 0.08, 0.2, 0.025); tone(784, 0.16, 0.32, 0.02); },
    edge: () => tone(220, 0, 0.06, 0.02, 'triangle'),
  };

  window.Sfx = {
    set enabled(v) { enabled = !!v; },
    play(name) {
      if (!enabled || !SOUNDS[name]) return;
      try {
        if (!ctx) ctx = new AudioContext();
        if (ctx.state === 'suspended') ctx.resume();
        SOUNDS[name]();
      } catch { /* audio unavailable */ }
    },
  };
})();
