#!/usr/bin/env bash
# Builds Sofa-Setup-<version>-x86_64.AppImage from a release build: one file, like the Windows
# setup exe. Run once, it installs itself (see app/src/linux/install.rs). Sofa links nothing but
# glibc and loads the desktop's own X11, xkbcommon and PulseAudio libraries at run time, so the
# AppImage carries only the program, its desktop entry and its icon.
#
#   packaging/linux/build-appimage.sh [path/to/sofa] [out-dir]
#
# appimagetool is downloaded from AppImage's releases unless APPIMAGETOOL points to one.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
exe=$(realpath "${1:-${CARGO_TARGET_DIR:-$root/target}/release/sofa}")
out=$(realpath -m "${2:-$root/dist}")
version=$(grep -m1 '^version' "$root/app/Cargo.toml" | cut -d'"' -f2)
arch=${ARCH:-x86_64}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
app="$work/Sofa.AppDir"

mkdir -p "$app/usr/bin" "$app/usr/share/applications" "$app/usr/share/icons/hicolor/512x512/apps"
install -m755 "$exe" "$app/usr/bin/sofa"
install -m644 "$here/sofa.desktop" "$app/usr/share/applications/sofa.desktop"
install -m644 "$here/sofa.desktop" "$app/sofa.desktop"
install -m644 "$root/assets/icon.png" "$app/usr/share/icons/hicolor/512x512/apps/sofa.png"
install -m644 "$root/assets/icon.png" "$app/sofa.png"
ln -s sofa.png "$app/.DirIcon"
ln -s usr/bin/sofa "$app/AppRun"

tool=${APPIMAGETOOL:-$work/appimagetool}
if [ ! -x "$tool" ]; then
  curl -fsSL -o "$tool" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
  chmod +x "$tool"
fi
mkdir -p "$out"
target="$out/Sofa-Setup-$version-$arch.AppImage"
# Containers have no FUSE: appimagetool runs extracted.
ARCH=$arch APPIMAGE_EXTRACT_AND_RUN=1 "$tool" --no-appstream "$app" "$target"
echo "$target"
