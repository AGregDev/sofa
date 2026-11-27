//! Gives sofa.exe its icon and version details (a Windows resource compiled with windres).
//! Without windres the build still works, with a warning and Windows' default icon.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../assets/icon.ico");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let icon = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("../assets/icon.ico");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts: Vec<u32> = version.split(['.', '-']).filter_map(|p| p.parse().ok()).collect();
    parts.resize(4, 0);
    let nums = format!("{},{},{},{}", parts[0], parts[1], parts[2], parts[3]);
    let rc = format!(
        r#"1 ICON "sofa.ico"
1 VERSIONINFO
FILEVERSION {nums}
PRODUCTVERSION {nums}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "AGregDev"
      VALUE "FileDescription", "Sofa"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "Sofa"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "Sofa.exe"
      VALUE "ProductName", "Sofa"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    // windres runs the C preprocessor, which mishandles paths with spaces: work inside OUT_DIR
    // with relative names only.
    if std::fs::copy(&icon, out.join("sofa.ico")).is_err() || std::fs::write(out.join("sofa.rc"), rc).is_err() {
        println!("cargo:warning=could not prepare the icon resource");
        return;
    }
    let ok = Command::new("windres").current_dir(&out).args(["sofa.rc", "-O", "coff", "-o", "sofa-res.o"]).status().is_ok_and(|s| s.success());
    if ok {
        println!("cargo:rustc-link-arg-bins={}", out.join("sofa-res.o").display());
    } else {
        println!("cargo:warning=windres not found or failed: sofa.exe gets no icon");
    }
}
