//! Windows' sound devices, for the quick menu and the DualSense's mute button: the outputs and
//! which one is the default, the output volume, and the microphone's mute. Reads and the
//! volume go through the documented endpoint APIs (IMMDeviceEnumerator, IAudioEndpointVolume).
//! Making a device the default goes through the policy config interface Windows' own Sound
//! settings use; it is undocumented but unchanged since Windows 7 and used by every sound
//! switcher.
//!
//! Every call runs on the UI thread (COM is initialised there) and takes a few milliseconds.
//! In a QA run `ui.rs` never calls the functions that change anything.

use std::ffi::c_void;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToStringAlloc, PropVariantToUInt32};
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::*;

/// What a sound output is, for its icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// HDMI or DisplayPort: the TV or monitor.
    Tv,
    Headphones,
    Speakers,
    /// The DualSense's own jack (USB only).
    Controller,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Output {
    pub id: String,
    /// "LG TV", "Headphones": the device's name without the adapter in brackets.
    pub name: String,
    /// The adapter ("NVIDIA High Definition Audio"), to tell two outputs of the same name apart.
    pub detail: String,
    pub kind: Kind,
}

const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };

fn enumerator() -> Option<IMMDeviceEnumerator> {
    // SAFETY: plain COM object creation on a thread with COM initialised.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()
}

/// A string Windows allocated, copied and freed.
///
/// # Safety
/// `p` must be a CoTaskMem string or null.
unsafe fn take(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the caller passes a valid CoTaskMem string; it is freed once, after copying.
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s
    }
}

fn prop<T>(store: &IPropertyStore, key: &PROPERTYKEY, read: impl FnOnce(&PROPVARIANT) -> Option<T>) -> Option<T> {
    // SAFETY: the PROPVARIANT is read, then cleared once.
    unsafe {
        let mut v = store.GetValue(key).ok()?;
        let out = read(&v);
        let _ = PropVariantClear(&mut v);
        out
    }
}

/// Splits "LG TV (NVIDIA High Definition Audio)" into the name and the adapter.
pub(crate) fn split_name(full: &str) -> (String, String) {
    match (full.find(" ("), full.ends_with(')')) {
        (Some(i), true) if i > 0 => (full[..i].trim().to_owned(), full[i + 2..full.len() - 1].trim().to_owned()),
        _ => (full.trim().to_owned(), String::new()),
    }
}

fn kind(full: &str, form: Option<u32>) -> Kind {
    let l = full.to_lowercase();
    if l.contains("wireless controller") || l.contains("dualsense") {
        return Kind::Controller;
    }
    // EndpointFormFactor: 3 headphones, 5 headset, 9 digital display (HDMI, DisplayPort).
    match form {
        Some(3 | 5) => Kind::Headphones,
        Some(9) => Kind::Tv,
        _ if l.contains("headphone") || l.contains("headset") || l.contains("earphone") => Kind::Headphones,
        _ => Kind::Speakers,
    }
}

/// The active outputs, in Windows' order, and the default one's id.
pub fn outputs() -> (Vec<Output>, Option<String>) {
    let Some(e) = enumerator() else { return (Vec::new(), None) };
    let mut list = Vec::new();
    // SAFETY: COM calls on objects this function owns; strings freed by `take`.
    unsafe {
        let default = e.GetDefaultAudioEndpoint(eRender, eConsole).ok().and_then(|d| d.GetId().ok()).map(|p| take(p));
        if let Ok(all) = e.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
            for i in 0..all.GetCount().unwrap_or(0) {
                let Ok(d) = all.Item(i) else { continue };
                let Ok(id) = d.GetId() else { continue };
                let id = take(id);
                let Ok(store) = d.OpenPropertyStore(STGM_READ) else { continue };
                let full = prop(&store, &PKEY_DEVICE_FRIENDLY_NAME, |v| PropVariantToStringAlloc(v).ok().map(|p| take(p))).unwrap_or_default();
                let form = prop(&store, &PKEY_AudioEndpoint_FormFactor, |v| PropVariantToUInt32(v).ok());
                let (name, detail) = split_name(&full);
                list.push(Output { id, name, detail, kind: kind(&full, form) });
            }
        }
        // Two outputs with the same short name keep their adapter's name.
        let names: Vec<String> = list.iter().map(|o| o.name.to_lowercase()).collect();
        for o in &mut list {
            if names.iter().filter(|n| **n == o.name.to_lowercase()).count() > 1 && !o.detail.is_empty() {
                o.name = format!("{} ({})", o.name, o.detail);
            }
        }
        (list, default)
    }
}

/// Makes `id` the default output for games, media and calls.
pub fn set_output(id: &str) -> bool {
    const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    const IID_POLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    // IPolicyConfig: IUnknown's three methods, ten more, then SetDefaultEndpoint(id, role).
    type SetDefaultEndpoint = unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT;
    // SAFETY: the object is queried for IPolicyConfig and the 14th vtable entry is its
    // SetDefaultEndpoint (stable layout since Windows 7); the queried reference is released.
    unsafe {
        let Ok(unknown) = CoCreateInstance::<_, IUnknown>(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL) else { return false };
        let mut raw: *mut c_void = std::ptr::null_mut();
        if unknown.query(&IID_POLICY_CONFIG, &mut raw).is_err() || raw.is_null() {
            return false;
        }
        let config = IUnknown::from_raw(raw);
        let vtable = *(raw as *const *const usize);
        let set: SetDefaultEndpoint = std::mem::transmute(*vtable.add(13));
        let wide = HSTRING::from(id);
        let mut ok = true;
        for role in [eConsole, eMultimedia, eCommunications] {
            ok &= set(raw, PCWSTR(wide.as_ptr()), role).is_ok();
        }
        drop(config);
        ok
    }
}

fn endpoint_volume(flow: EDataFlow) -> Option<IAudioEndpointVolume> {
    let e = enumerator()?;
    // SAFETY: COM calls on objects this function owns.
    unsafe { e.GetDefaultAudioEndpoint(flow, eConsole).ok()?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok() }
}

/// The default output's volume (0 to 1) and whether it is muted.
pub fn volume() -> Option<(f32, bool)> {
    let v = endpoint_volume(eRender)?;
    // SAFETY: plain getters.
    unsafe { Some((v.GetMasterVolumeLevelScalar().ok()?, v.GetMute().ok()?.as_bool())) }
}

pub fn set_volume(level: f32) -> bool {
    let Some(v) = endpoint_volume(eRender) else { return false };
    // SAFETY: plain setters; null event context.
    unsafe { v.SetMute(false, std::ptr::null()).is_ok() && v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null()).is_ok() }
}

pub fn set_muted(muted: bool) -> bool {
    let Some(v) = endpoint_volume(eRender) else { return false };
    // SAFETY: plain setter; null event context.
    unsafe { v.SetMute(muted, std::ptr::null()).is_ok() }
}

/// Whether the default microphone is muted (None: there is no microphone).
pub fn mic_muted() -> Option<bool> {
    let v = endpoint_volume(eCapture)?;
    // SAFETY: plain getter.
    unsafe { v.GetMute().ok().map(|b| b.as_bool()) }
}

pub fn set_mic_muted(muted: bool) -> bool {
    let Some(v) = endpoint_volume(eCapture) else { return false };
    // SAFETY: plain setter; null event context.
    unsafe { v.SetMute(muted, std::ptr::null()).is_ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_lose_the_adapter() {
        assert_eq!(split_name("LG TV (NVIDIA High Definition Audio)"), ("LG TV".into(), "NVIDIA High Definition Audio".into()));
        assert_eq!(split_name("Speakers (Realtek(R) Audio)"), ("Speakers".into(), "Realtek(R) Audio".into()));
        assert_eq!(split_name("Headphones"), ("Headphones".into(), String::new()));
    }

    #[test]
    fn kinds_follow_the_form_factor() {
        assert_eq!(kind("LG TV (NVIDIA High Definition Audio)", Some(9)), Kind::Tv);
        assert_eq!(kind("Headset Earphone (Wireless Controller)", Some(5)), Kind::Controller);
        assert_eq!(kind("Speakers (Realtek(R) Audio)", Some(1)), Kind::Speakers);
        assert_eq!(kind("Arctis 7 Game", None), Kind::Speakers);
        assert_eq!(kind("Headphones (WH-1000XM4)", None), Kind::Headphones);
    }
}
