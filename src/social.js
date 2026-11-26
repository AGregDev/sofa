// Discord voice through Discord's local RPC pipe. Discord only lets an app control
// voice for its own developers, so each person uses a Discord application they made.
// The client secret and tokens are encrypted with Windows' data protection (safeStorage).
const net = require('net');
const crypto = require('crypto');
const { safeStorage } = require('electron');

function seal(s) {
  if (!s) return null;
  return safeStorage.isEncryptionAvailable() ? 'enc:' + safeStorage.encryptString(s).toString('base64') : 'raw:' + s;
}
function open(s) {
  if (!s) return null;
  if (s.startsWith('enc:')) { try { return safeStorage.decryptString(Buffer.from(s.slice(4), 'base64')); } catch { return null; } }
  return s.startsWith('raw:') ? s.slice(4) : null;
}

// ---------- Discord RPC ----------

const SCOPES = ['rpc', 'rpc.voice.read', 'rpc.voice.write', 'identify', 'guilds'];
const REDIRECT = 'http://127.0.0.1';

class Discord {
  constructor(store, onChange) {
    this.store = store;
    this.onChange = onChange;
    this.sock = null;
    this.buf = Buffer.alloc(0);
    this.pending = new Map();
    this.ready = false;
    this.user = null;
    this.error = null;
  }

  get cfg() { return (this.store.data.social ||= {}).discord ||= {}; }
  get configured() { return !!(this.cfg.clientId && this.cfg.secret); }

  // A sign-in belongs to one application: keep it unless the Client ID changes.
  setApp(clientId, secret) {
    const id = String(clientId || '').trim();
    if (id && id !== this.cfg.clientId) { this.cfg.clientId = id; delete this.cfg.token; }
    if (secret) this.cfg.secret = seal(String(secret).trim());
    this.error = null;
    this.store.save();
    this.close();
  }

  forget() {
    this.close();
    const recent = this.cfg.recent;
    this.store.data.social.discord = recent ? { recent } : {};
    this.store.save();
  }

  close() {
    this.watching = null;
    if (this.sock) this.sock.destroy();
    this.sock = null;
    this.ready = false;
  }

  pipe() {
    return new Promise((resolve, reject) => {
      let i = 0;
      const tryNext = () => {
        if (i > 9) return reject(new Error('Discord is not running'));
        const s = net.connect(`\\\\?\\pipe\\discord-ipc-${i++}`);
        s.once('connect', () => resolve(s));
        s.once('error', () => tryNext());
      };
      tryNext();
    });
  }

  send(op, data) {
    const body = Buffer.from(JSON.stringify(data));
    const head = Buffer.alloc(8);
    head.writeInt32LE(op, 0);
    head.writeInt32LE(body.length, 4);
    this.sock.write(Buffer.concat([head, body]));
  }

  onData(chunk) {
    this.buf = Buffer.concat([this.buf, chunk]);
    while (this.buf.length >= 8) {
      const op = this.buf.readInt32LE(0), len = this.buf.readInt32LE(4);
      if (this.buf.length < 8 + len) break;
      const msg = JSON.parse(this.buf.subarray(8, 8 + len).toString());
      this.buf = this.buf.subarray(8 + len);
      if (op === 2) { this.close(); this.error = msg.message || 'Discord closed the connection'; this.onChange(); continue; }
      if (msg.cmd === 'DISPATCH' && msg.evt === 'READY') { this.handshake && this.handshake(); continue; }
      if (msg.nonce && this.pending.has(msg.nonce)) {
        const { resolve, reject } = this.pending.get(msg.nonce);
        this.pending.delete(msg.nonce);
        if (msg.evt === 'ERROR') reject(new Error(msg.data?.message || 'Discord error'));
        else resolve(msg.data);
      } else if (msg.cmd === 'DISPATCH') this.onChange(msg.evt);
    }
  }

  call(cmd, args = {}, evt) {
    if (!this.sock) return Promise.reject(new Error('Not connected to Discord'));
    const nonce = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      this.pending.set(nonce, { resolve, reject });
      this.send(1, { cmd, args, evt, nonce });
      setTimeout(() => { if (this.pending.delete(nonce)) reject(new Error(`Discord did not answer ${cmd}`)); }, cmd === 'AUTHORIZE' ? 120000 : 8000);
    });
  }

  async token(code) {
    const body = new URLSearchParams({ client_id: this.cfg.clientId, client_secret: open(this.cfg.secret) || '' });
    if (code) { body.set('grant_type', 'authorization_code'); body.set('code', code); body.set('redirect_uri', REDIRECT); }
    else { body.set('grant_type', 'refresh_token'); body.set('refresh_token', open(this.cfg.token?.refresh) || ''); }
    const r = await fetch('https://discord.com/api/oauth2/token', { method: 'POST', body, headers: { 'Content-Type': 'application/x-www-form-urlencoded' } });
    if (!r.ok) throw new Error(`Discord refused the sign-in (${r.status}). Check the client secret and the ${REDIRECT} redirect.`);
    const t = await r.json();
    this.cfg.token = { access: seal(t.access_token), refresh: seal(t.refresh_token), expires: Date.now() + t.expires_in * 1000 - 60000 };
    this.store.save();
    return t.access_token;
  }

  // Several callers ask at once on startup; share one connection attempt so Discord
  // never shows its Authorize prompt twice.
  // Only an explicit Connect may show Discord's Authorize prompt; background refreshes
  // use the saved sign-in and report an error instead of prompting behind Sofa.
  connect(interactive = false) {
    if (this.ready) return Promise.resolve(true);
    if (!this.configured) return Promise.resolve(false);
    if (!this.connecting) this.connecting = this.doConnect(interactive).finally(() => { this.connecting = null; });
    return this.connecting;
  }

  async doConnect(interactive) {
    this.error = null;
    try {
      this.close();
      const sock = await this.pipe();
      this.sock = sock;
      sock.on('data', (c) => this.onData(c));
      sock.on('close', () => { if (this.sock !== sock) return; this.ready = false; this.sock = null; this.onChange(); });
      await new Promise((resolve, reject) => {
        this.handshake = resolve;
        this.send(0, { v: 1, client_id: this.cfg.clientId });
        setTimeout(() => reject(new Error('Discord did not answer')), 6000);
      });
      // Saved sign-in first, then a silent refresh; Discord's own "Authorize" prompt
      // only appears when neither works (the first time, or after access was removed).
      const t = this.cfg.token;
      const tryAuth = async (access) => (access ? this.call('AUTHENTICATE', { access_token: access }).catch(() => null) : null);
      let auth = t && t.expires > Date.now() ? await tryAuth(open(t.access)) : null;
      if (!auth && t && t.refresh) auth = await tryAuth(await this.token(null).catch(() => null));
      if (!auth && !interactive) throw new Error('Sign in again from Social, Connect Discord');
      if (!auth) {
        if (this.onAuthorize) this.onAuthorize();
        this.authorizing = true;
        this.onChange();
        try {
          const { code } = await this.call('AUTHORIZE', { client_id: this.cfg.clientId, scopes: SCOPES });
          auth = await this.call('AUTHENTICATE', { access_token: await this.token(code) });
        } finally { this.authorizing = false; }
      }
      this.user = auth.user;
      this.ready = true;
      for (const evt of ['VOICE_CHANNEL_SELECT', 'VOICE_SETTINGS_UPDATE']) this.call('SUBSCRIBE', {}, evt).catch(() => {});
      this.onChange();
      return true;
    } catch (e) {
      this.error = e.message;
      this.close();
      this.onChange();
      return false;
    }
  }

  async state(interactive = false) {
    if (!this.configured) return { configured: false };
    if (!(await this.connect(interactive))) return { configured: true, error: this.error, notRunning: this.error === 'Discord is not running' };
    const [guilds, current, voice] = await Promise.all([
      this.call('GET_GUILDS').then((d) => d.guilds).catch(() => []),
      this.call('GET_SELECTED_VOICE_CHANNEL').catch(() => null),
      this.call('GET_VOICE_SETTINGS').catch(() => null),
    ]);
    if (current && current.id && this.watching !== current.id) {
      // Follow who joins and leaves the channel you're in.
      this.watching = current.id;
      for (const evt of ['VOICE_STATE_CREATE', 'VOICE_STATE_DELETE', 'VOICE_STATE_UPDATE']) {
        this.call('SUBSCRIBE', { channel_id: current.id }, evt).catch(() => {});
      }
    }
    return {
      configured: true,
      user: this.user && { id: this.user.id, name: this.user.global_name || this.user.username },
      guilds: (guilds || []).map((g) => ({ id: g.id, name: g.name, icon: g.icon_url || null })),
      current: current && current.id ? {
        id: current.id,
        name: current.name,
        guildId: current.guild_id,
        members: (current.voice_states || []).map((v) => ({
          id: v.user.id,
          name: v.nick || v.user.global_name || v.user.username,
          avatar: v.user.avatar ? `https://cdn.discordapp.com/avatars/${v.user.id}/${v.user.avatar}.png?size=128` : null,
          muted: !!(v.voice_state?.self_mute || v.voice_state?.mute),
        })),
      } : null,
      voice: voice ? { mute: !!voice.mute, deaf: !!voice.deaf } : null,
    };
  }

  async guildVoice(guildId) {
    if (!(await this.connect())) return [];
    const { channels } = await this.call('GET_CHANNELS', { guild_id: guildId });
    const voice = (channels || []).filter((c) => c.type === 2 || c.type === 13);
    const full = await Promise.all(voice.map((c) => this.call('GET_CHANNEL', { channel_id: c.id }).catch(() => c)));
    return full.map((c) => ({
      id: c.id,
      name: c.name,
      members: (c.voice_states || []).map((v) => v.nick || v.user.global_name || v.user.username),
    }));
  }

  async join(channelId) {
    const r = await this.call('SELECT_VOICE_CHANNEL', { channel_id: channelId || null, force: true });
    if (channelId && r) {
      // Remember it for the Home screen's quick-join row.
      const list = (this.cfg.recent || []).filter((c) => c.id !== channelId);
      this.cfg.recent = [{ id: channelId, guildId: r.guild_id || null }, ...list].slice(0, 6);
      this.store.save();
    }
    return r;
  }

  // Recently joined voice channels with who is in them now.
  async recent() {
    if (!this.configured || !(await this.connect())) return [];
    const guilds = await this.call('GET_GUILDS').then((d) => d.guilds || []).catch(() => []);
    const names = Object.fromEntries(guilds.map((g) => [g.id, g.name]));
    const out = await Promise.all((this.cfg.recent || []).map((c) => this.call('GET_CHANNEL', { channel_id: c.id }).then((ch) => ({
      id: ch.id,
      name: ch.name,
      guild: names[ch.guild_id] || '',
      members: (ch.voice_states || []).map((v) => ({
        name: v.nick || v.user.global_name || v.user.username,
        avatar: v.user.avatar ? `https://cdn.discordapp.com/avatars/${v.user.id}/${v.user.avatar}.png?size=128` : null,
      })),
    })).catch(() => null)));
    return out.filter(Boolean);
  }
  setVoice(patch) { return this.call('SET_VOICE_SETTINGS', patch); }

  // Points Discord's voice output at the first device whose name matches and returns
  // the device it used before, so it can be put back. Null when Discord isn't set up
  // or has no such device.
  async setOutputMatching(re) {
    if (!this.configured || !(await this.connect())) return null;
    const v = await this.call('GET_VOICE_SETTINGS');
    const out = (v && v.output) || {};
    const dev = (out.available_devices || []).find((d) => re.test(d.name || ''));
    if (!dev) return null;
    if (out.device_id !== dev.id) await this.call('SET_VOICE_SETTINGS', { output: { device_id: dev.id } });
    return { prev: out.device_id || 'default', name: dev.name };
  }

  async setOutputDevice(id) {
    if (!this.configured || !(await this.connect())) return false;
    await this.call('SET_VOICE_SETTINGS', { output: { device_id: id || 'default' } });
    return true;
  }
}

module.exports = { seal, open, Discord, REDIRECT };
