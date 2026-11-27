//! The Start menu's apps (shell:AppsFolder) and program icons, as Windows lists and draws
//! them. Replaces Sofa 1.x's `apps.ps1` and `icon.ps1`. Runs on a background thread: the
//! enumeration and the 256 px icons take a few seconds on a full Start menu.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::core::*;

/// A stable short name for a cache file.
pub fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.to_lowercase().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Saves the icon Explorer shows for a program or Start menu entry (up to 256 px) as a PNG.
pub fn save_icon(parsing_name: &str, out: &Path) -> bool {
    if out.exists() {
        return true;
    }
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    // SAFETY: COM (initialized for this thread if it was not) and objects owned by this
    // function; the HBITMAP is deleted after encoding.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let name = HSTRING::from(parsing_name);
        let Ok(factory) = SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(&name, None) else { return false };
        let Ok(hbm) = factory.GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_BIGGERSIZEOK | SIIGBF_ICONONLY) else { return false };
        let ok = encode_png(hbm, out).is_ok();
        let _ = DeleteObject(hbm.into());
        ok
    }
}

/// Saves a program's own icon (from its file, up to 256 px) as a PNG. Unlike the shell's
/// image, this never turns into the generic file icon when the shell cannot read the file;
/// a program without an icon gives no PNG, and Sofa draws its app glyph instead.
pub fn save_exe_icon(exe: &str, out: &Path) -> bool {
    if out.exists() {
        return true;
    }
    if !exe.to_lowercase().ends_with(".exe") || !Path::new(exe).is_file() {
        return false;
    }
    let wide: Vec<u16> = exe.encode_utf16().collect();
    if wide.len() >= 260 {
        return save_icon(exe, out);
    }
    let mut name = [0u16; 260];
    name[..wide.len()].copy_from_slice(&wide);
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    // SAFETY: COM (initialized for this thread if it was not); the icon is destroyed after
    // encoding.
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON, PrivateExtractIconsW};
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let mut icons = [HICON::default()];
        let n = PrivateExtractIconsW(&name, 0, 256, 256, Some(&mut icons), None, 0);
        if n == 0 || n == u32::MAX || icons[0].is_invalid() {
            return false;
        }
        let ok = (|| -> Result<()> {
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let bmp = wic.CreateBitmapFromHICON(icons[0])?;
            write_png(&wic, &bmp, out)
        })()
        .is_ok();
        let _ = DestroyIcon(icons[0]);
        if !ok {
            let _ = std::fs::remove_file(out);
        }
        ok
    }
}

unsafe fn encode_png(hbm: windows::Win32::Graphics::Gdi::HBITMAP, out: &Path) -> Result<()> {
    // SAFETY: the caller owns `hbm`; WIC objects are created and released here.
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bmp = wic.CreateBitmapFromHBITMAP(hbm, windows::Win32::Graphics::Gdi::HPALETTE::default(), WICBitmapUsePremultipliedAlpha)?;
        write_png(&wic, &bmp, out)
    }
}

unsafe fn write_png(wic: &IWICImagingFactory, bmp: &IWICBitmap, out: &Path) -> Result<()> {
    // SAFETY: WIC objects are created and released here.
    unsafe {
        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(out.as_os_str()), GENERIC_WRITE_ACCESS)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut props: Option<windows::Win32::System::Com::StructuredStorage::IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut props)?;
        let frame = frame.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?;
        frame.Initialize(props.as_ref())?;
        let (mut w, mut h) = (0, 0);
        bmp.GetSize(&mut w, &mut h)?;
        frame.SetSize(w, h)?;
        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt)?;
        frame.WriteSource(bmp, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()
    }
}

const GENERIC_WRITE_ACCESS: u32 = 0x4000_0000;

/// Every Start menu entry: (name, AppID as `shell:AppsFolder\<AppID>` opens it).
fn enumerate() -> Vec<(String, String)> {
    let mut out = Vec::new();
    // SAFETY: shell enumeration on objects owned here; returned strings freed with
    // CoTaskMemFree after copying.
    unsafe {
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return out };
        let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut batch = [None];
            let mut fetched = 0u32;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else { break };
            let text = |kind: SIGDN| -> Option<String> {
                let p = item.GetDisplayName(kind).ok()?;
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as _));
                s
            };
            let (Some(name), Some(mut id)) = (text(SIGDN_NORMALDISPLAY), text(SIGDN_PARENTRELATIVEPARSING)) else { continue };
            // The AppsFolder's own prefix, when Windows includes it.
            if id.starts_with("::{")
                && let Some(i) = id.find('\\')
            {
                id = id[i + 1..].to_owned();
            }
            out.push((name, id));
        }
    }
    out
}

/// Lists the Start menu, extracts missing icons, and writes `cache\startmenu.json`
/// (`[{ name, appId, icon }]`). Run on a background thread.
pub fn scan(cache: &Path) -> usize {
    // SAFETY: COM for this thread, for the shell and WIC calls.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let icons = cache.join("icons");
    let apps = enumerate();
    let list: Vec<Value> = apps
        .iter()
        .map(|(name, id)| {
            let png: PathBuf = icons.join(format!("app-{}.png", hash(id)));
            let icon = save_icon(&format!("shell:AppsFolder\\{id}"), &png).then(|| png.to_string_lossy().to_string());
            json!({ "name": name, "appId": id, "icon": icon })
        })
        .collect();
    let n = list.len();
    if n > 0 {
        let tmp = cache.join("startmenu.json.tmp");
        if std::fs::write(&tmp, Value::Array(list).to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, cache.join("startmenu.json"));
        }
    }
    n
}
