//! Sofa's Linux half. The modules that replace a Windows one whole live next to these files and
//! are picked in `main.rs` (`gfx.rs`, `platform.rs`, `audio.rs`, `media.rs`, `install.rs`,
//! `startmenu.rs`); these are the parts they share.

pub mod app;
pub mod clock;
pub mod desktop;
pub mod net;
pub mod pads;
pub mod proc;
pub mod pulse;
pub mod secret;
pub mod text;
pub mod wm;
pub mod x11;

/// `$XDG_CONFIG_HOME` (`~/.config`).
pub fn config_home() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".config"))
}

/// `$XDG_DATA_HOME` (`~/.local/share`).
pub fn data_home() -> std::path::PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".local/share"))
}

pub fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default()
}
