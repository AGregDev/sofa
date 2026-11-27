//! The Steam settings editor, on a made-up config in a temp folder; never a real Steam install.
//! The same checks (G1 to G15) Sofa 1.x ran on its JS version.

use std::path::PathBuf;

/// Shaped like a real file: tab indents, LF line endings, nested sections, an escaped quote.
const BASE: &str = "\"UserLocalConfigStore\"\n{\n\t\"Broadcast\"\n\t{\n\t\t\"Permissions\"\t\t\"1\"\n\t}\n\t\"friends\"\n\t{\n\t\t\"PersonaName\"\t\t\"Player\"\n\t\t\"Notes\"\t\t\"a \\\"quoted\\\" word\"\n\t}\n\t\"ControllerTypesUsed\"\t\t\"controller_ps5,\"\n\t\"apps\"\n\t{\n\t\t\"413080\"\n\t\t{\n\t\t\t\"UseSteamControllerConfig\"\t\t\"2\"\n\t\t}\n\t}\n}\n";

fn with_key(v: &str) -> String {
    let i = BASE.rfind('}').expect("root block");
    format!("{}\t\"Controller_CheckGuideButton\"\t\t\"{v}\"\n{}", &BASE[..i], &BASE[i..])
}

fn temp_steam(config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sofa-steam-test-{}-{}", std::process::id(), config.len()));
    let cfg = dir.join("userdata").join("42").join("config");
    std::fs::create_dir_all(&cfg).expect("temp folder");
    std::fs::write(cfg.join("localconfig.vdf"), config).expect("write config");
    std::fs::write(dir.join("steam.exe"), b"").expect("fake steam.exe");
    dir
}

#[test]
fn g1_to_g7_editing() {
    assert_eq!(steam::read_value(BASE), None, "G1 a file without the key reads as Steam's default");
    let inserted = steam::with_guide_off(BASE).expect("G2");
    assert_eq!(steam::read_value(&inserted).as_deref(), Some("0"), "G2 turning it off adds the key with 0");
    assert_eq!(inserted.replace("\t\"Controller_CheckGuideButton\"\t\t\"0\"\n", ""), BASE, "G3 adding the key changes nothing else");
    assert_eq!(steam::read_value(&with_key("1")).as_deref(), Some("1"), "G4 a file with the setting on reads 1");
    assert_eq!(steam::with_guide_off(&with_key("1")).expect("G5"), with_key("0"), "G5 turning it off replaces only the value");
    let crlf = steam::with_guide_off(&BASE.replace('\n', "\r\n")).expect("G6");
    assert!(steam::read_value(&crlf).as_deref() == Some("0") && !crlf.replace("\r\n", "").contains('\n'), "G6 Windows line endings are kept");
    assert!(steam::with_guide_off("\"UserLocalConfigStore\"").is_err(), "G7 a file it cannot verify is refused");
}

#[test]
fn g8_to_g15_fixing() {
    let dir = temp_steam(&with_key("1"));
    let open = || true;
    let closed = || false;
    let file = dir.join("userdata").join("42").join("config").join("localconfig.vdf");
    let read = || std::fs::read_to_string(&file).expect("config");
    let s = steam::status(&dir, &closed);
    assert!(s.installed && s.accounts == 1 && !s.off, "G8 status finds the account with the setting on");
    assert_eq!(steam::fix(&dir, false, &open), steam::Fix::Running, "G9 with Steam open and no permission, nothing changes");
    assert_eq!(steam::read_value(&read()).as_deref(), Some("1"), "G9 the file is untouched");
    assert_eq!(steam::fix(&dir, false, &closed), steam::Fix::Changed { restarted: false }, "G10 with Steam closed, the fix writes the file");
    assert_eq!(steam::read_value(&read()).as_deref(), Some("0"), "G10 the setting reads off");
    let backup = std::fs::read_to_string(file.with_extension("vdf.sofa-backup")).expect("G11 backup exists");
    assert_eq!(steam::read_value(&backup).as_deref(), Some("1"), "G11 the original file is kept as a backup");
    assert!(!file.with_extension("vdf.sofa-tmp").exists(), "G12 no temp file is left behind");
    assert_eq!(steam::fix(&dir, false, &closed), steam::Fix::AlreadyOff, "G13 running it again changes nothing");
    assert!(steam::status(&dir, &closed).off, "G14 status then reports it off");
    assert!(!steam::status(&dir.join("nope"), &closed).installed, "G15 a missing Steam is reported, not guessed");
    let _ = std::fs::remove_dir_all(&dir);
}
