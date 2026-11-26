// Minimal parser for Valve's text KeyValues format (.vdf / .acf files).
function parseVdf(text) {
  let i = 0;
  const n = text.length;

  function skip() {
    while (i < n) {
      const c = text[i];
      if (c === ' ' || c === '\t' || c === '\r' || c === '\n') i++;
      else if (c === '/' && text[i + 1] === '/') {
        while (i < n && text[i] !== '\n') i++;
      } else break;
    }
  }

  function str() {
    if (text[i] !== '"') {
      // Unquoted token
      const start = i;
      while (i < n && !/\s|[{}"]/.test(text[i])) i++;
      return text.slice(start, i);
    }
    i++;
    let out = '';
    while (i < n && text[i] !== '"') {
      if (text[i] === '\\' && i + 1 < n) {
        const e = text[i + 1];
        out += e === 'n' ? '\n' : e === 't' ? '\t' : e;
        i += 2;
      } else out += text[i++];
    }
    i++;
    return out;
  }

  function obj() {
    const o = {};
    for (;;) {
      skip();
      if (i >= n) return o;
      if (text[i] === '}') { i++; return o; }
      const key = str();
      skip();
      if (text[i] === '{') { i++; o[key] = obj(); }
      else o[key] = str();
    }
  }

  return obj();
}

// Case-insensitive property lookup (Valve files are inconsistent about casing).
function get(o, key) {
  if (!o || typeof o !== 'object') return undefined;
  if (key in o) return o[key];
  const lk = key.toLowerCase();
  for (const k of Object.keys(o)) if (k.toLowerCase() === lk) return o[k];
  return undefined;
}

module.exports = { parseVdf, get };
