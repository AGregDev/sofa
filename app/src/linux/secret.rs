//! Secrets on Linux (sign-ins for Discord and Spotify), where Windows has DPAPI: sealed with
//! AES-256-GCM under a key in `~/.config/sofa/key`, a file only the person's account may read
//! (0600, in a 0700 folder). `sofa.json` itself then holds nothing readable, so it can be shared
//! or backed up without the sign-ins in it.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

/// Random bytes from the kernel.
pub fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    b
}

fn key_file() -> PathBuf {
    // Tests never touch the person's own key.
    if cfg!(test) {
        return std::env::temp_dir().join(format!("sofa-test-{}", std::process::id())).join("key");
    }
    crate::store::data_dir().join("key")
}

/// The key, made once (32 random bytes).
fn key(create: bool) -> Option<Vec<u8>> {
    let path = key_file();
    if let Ok(k) = std::fs::read(&path)
        && k.len() == 32
    {
        return Some(k);
    }
    if !create {
        return None;
    }
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let k = random(32);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).ok()?;
    std::io::Write::write_all(&mut f, &k).ok()?;
    Some(k)
}

/// `aes:` + base64 of nonce and ciphertext.
pub fn seal(s: &str) -> Option<String> {
    let k = key(true)?;
    let cipher = Aes256Gcm::new_from_slice(&k).ok()?;
    let nonce = random(12);
    let mut out = nonce.clone();
    out.extend(cipher.encrypt(Nonce::from_slice(&nonce), s.as_bytes()).ok()?);
    Some(format!("aes:{}", crate::social::b64_encode(&out)))
}

/// What `seal` sealed (None: another key, or not a sealed value).
pub fn open(sealed: &[u8]) -> Option<String> {
    if sealed.len() < 12 + 16 {
        return None;
    }
    let k = key(false)?;
    let cipher = Aes256Gcm::new_from_slice(&k).ok()?;
    let (nonce, ct) = sealed.split_at(12);
    String::from_utf8(cipher.decrypt(Nonce::from_slice(nonce), ct).ok()?).ok()
}

/// SHA-256 (PKCE's code challenge).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}
