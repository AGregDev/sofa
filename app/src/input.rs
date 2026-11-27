//! Controller and keyboard input, turned into Sofa's actions. A port of `renderer/input.js`
//! and the reading half of `renderer/dualsense.js`:
//! - Xbox and other XInput pads through XInput (the Guide button through XInputGetStateEx);
//! - a DualSense read directly over HID on its own thread (USB, or Bluetooth after switching it
//!   to full reports), which keeps working while a game has focus, so the PS button always
//!   reaches Sofa;
//! - the keyboard, from the window's messages.

use std::sync::{Arc, Mutex};
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::HumanInterfaceDevice::*;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Input::XboxController::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::*;

pub const WM_PAD_HOME: u32 = 0x8000 + 2; // WM_APP + 2: the Home button, from any app
pub const WM_PAD_INPUT: u32 = 0x8000 + 3; // WM_APP + 3: the DualSense's buttons changed

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Accept,
    Back,
    Search,
    Menu,
    Options,
    TabPrev,
    TabNext,
    VolUp,
    VolDown,
    Home,
}

impl Action {
    fn is_dir(self) -> bool {
        matches!(self, Action::Up | Action::Down | Action::Left | Action::Right)
    }
}

/// Standard gamepad button order (as the browser Gamepad API numbers them).
const BUTTONS: [(usize, Action); 14] = [
    (0, Action::Accept),
    (1, Action::Back),
    (2, Action::Search),
    (3, Action::Menu),
    (4, Action::TabPrev),
    (5, Action::TabNext),
    (6, Action::VolDown),
    (7, Action::VolUp),
    (9, Action::Options),
    (12, Action::Up),
    (13, Action::Down),
    (14, Action::Left),
    (15, Action::Right),
    (16, Action::Home),
];

#[derive(Clone, Copy, Default)]
pub struct Pad {
    pub buttons: [bool; 18],
    pub axes: [f32; 4],
    pub at: f64,
}

/// What the DualSense reports besides its buttons.
#[derive(Clone, Copy, Default)]
pub struct PadInfo {
    pub connected: bool,
    pub battery: Option<u8>,
    pub charging: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PadKind {
    PlayStation,
    Xbox,
}

pub struct Input {
    layout: String, // auto | ps | xbox | nintendo
    pub kind: PadKind,
    held: [bool; 14],
    next_repeat: [f64; 14],
    held_since: [f64; 14],
    pad_active_at: f64,
    last_key_at: std::collections::HashMap<Action, f64>,
    last_pad_at: std::collections::HashMap<Action, f64>,
    get_state_ex: Option<unsafe extern "system" fn(u32, *mut XINPUT_STATE) -> u32>,
    xconnected: [bool; 4],
    xprobe_at: f64,
    home_held: bool,
    /// The PS (Guide) button is Home. Off, only Create + Options (Back + Start) is, and the PS
    /// button is left to Steam or the Game Bar (Settings, "Home button").
    pub ps_home: bool,
    pub ds: Arc<Mutex<(Pad, PadInfo)>>,
}

impl Input {
    /// `start` is the clock every timestamp is measured from (the UI's and the HID thread's).
    pub fn new(hwnd: HWND, start: std::time::Instant) -> Input {
        // XInputGetStateEx (ordinal 100) also reports the Guide button.
        // SAFETY: loading a system DLL and looking up a known export by ordinal.
        let get_state_ex = unsafe {
            LoadLibraryW(w!("xinput1_4.dll")).ok().and_then(|m| GetProcAddress(m, PCSTR(100 as *const u8))).map(|f| {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, unsafe extern "system" fn(u32, *mut XINPUT_STATE) -> u32>(f)
            })
        };
        let ds = Arc::new(Mutex::new((Pad::default(), PadInfo::default())));
        let shared = ds.clone();
        let h = hwnd.0 as isize;
        let _ = std::thread::Builder::new().name("dualsense".into()).spawn(move || dualsense_thread(shared, h, start));
        Input {
            layout: "auto".into(),
            kind: PadKind::PlayStation,
            held: [false; 14],
            next_repeat: [0.0; 14],
            held_since: [0.0; 14],
            pad_active_at: -1e9,
            last_key_at: Default::default(),
            last_pad_at: Default::default(),
            get_state_ex,
            xconnected: [true; 4],
            xprobe_at: 0.0,
            home_held: false,
            ps_home: true,
            ds,
        }
    }

    pub fn set_layout(&mut self, layout: &str) {
        self.layout = layout.to_owned();
        match layout {
            "xbox" | "nintendo" => self.kind = PadKind::Xbox,
            "ps" => self.kind = PadKind::PlayStation,
            _ => {}
        }
    }

    pub fn layout(&self) -> &str {
        &self.layout
    }

    /// A key press from the window: the action, unless the same press already came from a
    /// pad (Steam's desktop layout turns the pad into keys, so each press can arrive twice).
    pub fn key(&mut self, action: Action, now: f64) -> Option<Action> {
        let i = BUTTONS.iter().position(|(_, a)| *a == action).unwrap_or(0);
        if self.held[i] || now - self.pad_active_at < 400.0 || now - self.last_pad_at.get(&action).copied().unwrap_or(-1e9) < 250.0 {
            return None;
        }
        self.last_key_at.insert(action, now);
        Some(action)
    }

    fn xinput(&mut self, now: f64) -> Option<Pad> {
        let mut best: Option<Pad> = None;
        let probe = now >= self.xprobe_at;
        if probe {
            self.xprobe_at = now + 2000.0; // probing empty slots is slow: every 2 s
        }
        for i in 0..4u32 {
            if !self.xconnected[i as usize] && !probe {
                continue;
            }
            let mut st = XINPUT_STATE::default();
            // SAFETY: writes into a local struct.
            let r = unsafe {
                match self.get_state_ex {
                    Some(f) => f(i, &mut st),
                    None => XInputGetState(i, &mut st),
                }
            };
            self.xconnected[i as usize] = r == 0;
            if r != 0 {
                continue;
            }
            let g = st.Gamepad;
            let b = g.wButtons.0;
            let on = |m: u16| b & m != 0;
            let mut p = Pad { at: now, ..Pad::default() };
            let map = [
                (0, 0x1000),
                (1, 0x2000),
                (2, 0x4000),
                (3, 0x8000),
                (4, 0x0100),
                (5, 0x0200),
                (8, 0x0020),
                (9, 0x0010),
                (10, 0x0040),
                (11, 0x0080),
                (12, 0x0001),
                (13, 0x0002),
                (14, 0x0004),
                (15, 0x0008),
                (16, 0x0400),
            ];
            for (idx, mask) in map {
                p.buttons[idx] = on(mask);
            }
            p.buttons[6] = g.bLeftTrigger > 128;
            p.buttons[7] = g.bRightTrigger > 128;
            p.axes = [f32::from(g.sThumbLX) / 32768.0, -f32::from(g.sThumbLY) / 32768.0, 0.0, 0.0];
            if p.buttons.iter().any(|&x| x) || p.axes[0].abs() > 0.55 || p.axes[1].abs() > 0.55 || best.is_none() {
                best = Some(p);
            }
        }
        best
    }

    /// Polls the pads and returns the actions to run, in order.
    pub fn poll(&mut self, now: f64, focused: bool) -> Vec<Action> {
        let ds = *self.ds.lock().unwrap_or_else(|e| e.into_inner());
        let hid = (ds.1.connected && now - ds.0.at < 1000.0).then_some(ds.0);
        let x = self.xinput(now);
        // The pad in use drives the UI; the DualSense over HID wins while it is active.
        let pad = match (hid, x) {
            (Some(h), Some(xp)) if xp.buttons.iter().any(|&b| b) && !h.buttons.iter().any(|&b| b) => {
                self.auto_kind(PadKind::Xbox);
                Some(xp)
            }
            (Some(h), _) => {
                self.auto_kind(PadKind::PlayStation);
                Some(h)
            }
            (None, Some(xp)) => {
                self.auto_kind(PadKind::Xbox);
                Some(xp)
            }
            _ => None,
        };
        let mut out = Vec::new();
        let Some(p) = pad else {
            self.held = [false; 14];
            return out;
        };
        let state = self.state(&p);
        let home_i = BUTTONS.iter().position(|(_, a)| *a == Action::Home).unwrap_or(13);
        if !focused {
            // In the background only the Home button counts, and only from XInput pads: the
            // DualSense thread reports its own (WM_PAD_HOME). Buttons held now do not fire
            // when Sofa comes forward.
            let home = x.is_some_and(|xp| (self.ps_home && xp.buttons[16]) || (xp.buttons[8] && xp.buttons[9]));
            if home && !self.home_held {
                out.push(Action::Home);
            }
            self.home_held = home;
            self.held = state;
            self.held[home_i] = true;
            return out;
        }
        self.home_held = false;
        if state.iter().any(|&s| s) {
            self.pad_active_at = now;
        }
        for (i, (_, action)) in BUTTONS.iter().enumerate() {
            let down = state[i];
            if down && !self.held[i] {
                self.held_since[i] = now;
                self.next_repeat[i] = now + 360.0;
                if now - self.last_key_at.get(action).copied().unwrap_or(-1e9) >= 250.0 {
                    self.last_pad_at.insert(*action, now);
                    out.push(*action);
                }
            } else if down && action.is_dir() && now >= self.next_repeat[i] {
                // Repeat speeds up the longer a direction is held.
                self.next_repeat[i] = now + if now - self.held_since[i] > 1200.0 { 60.0 } else { 110.0 };
                out.push(*action);
            }
            self.held[i] = down;
        }
        out
    }

    /// Whether a direction is held (the loop must keep waking for its repeats).
    pub fn holding(&self) -> bool {
        BUTTONS.iter().zip(self.held).any(|((_, a), h)| h && a.is_dir())
    }

    /// The pad's buttons as Sofa's actions (layout swap, stick directions, the Home combo).
    fn state(&self, p: &Pad) -> [bool; 14] {
        let swap = self.layout == "nintendo";
        let mut state = [false; 14];
        for (i, (idx, _)) in BUTTONS.iter().enumerate() {
            let idx = match (*idx, swap) {
                (0, true) => 1,
                (1, true) => 0,
                (n, _) => n,
            };
            state[i] = p.buttons[idx];
        }
        let dir = |a: Action| BUTTONS.iter().position(|(_, x)| *x == a).unwrap_or(0);
        if p.axes[1] < -0.55 {
            state[dir(Action::Up)] = true;
        }
        if p.axes[1] > 0.55 {
            state[dir(Action::Down)] = true;
        }
        if p.axes[0] < -0.55 {
            state[dir(Action::Left)] = true;
        }
        if p.axes[0] > 0.55 {
            state[dir(Action::Right)] = true;
        }
        // The PS button is Home only when chosen; Create + Options together always is.
        if !self.ps_home {
            state[dir(Action::Home)] = false;
        }
        if p.buttons[8] && p.buttons[9] {
            state[dir(Action::Home)] = true;
            state[dir(Action::Options)] = false;
        }
        state
    }

    fn auto_kind(&mut self, k: PadKind) {
        if self.layout == "auto" {
            self.kind = k;
        }
    }

    pub fn info(&self) -> PadInfo {
        self.ds.lock().map(|g| g.1).unwrap_or_default()
    }

    /// Whether an XInput pad is connected: XInput has no events, so only then does the UI
    /// have to poll at 60 Hz. The DualSense wakes the UI itself (WM_PAD_INPUT).
    pub fn xinput_active(&self) -> bool {
        self.xconnected.iter().any(|&c| c)
    }
}

// ---------- DualSense over HID ----------

const SONY: u16 = 0x054c;
const MODELS: [u16; 2] = [0x0ce6, 0x0df2]; // DualSense, DualSense Edge

fn find_dualsense() -> Option<(HANDLE, bool, usize)> {
    // SAFETY: SetupAPI enumeration with buffers sized as the API reports; handles closed on
    // every path that does not return them.
    unsafe {
        let guid = HidD_GetHidGuid();
        let set = SetupDiGetClassDevsW(Some(&guid), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE).ok()?;
        let mut found = None;
        let mut index = 0;
        loop {
            let mut iface = SP_DEVICE_INTERFACE_DATA { cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInterfaces(set, None, &guid, index, &mut iface).is_err() {
                break;
            }
            index += 1;
            let mut needed = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &iface, None, 0, Some(&mut needed), None);
            if needed == 0 {
                continue;
            }
            let mut buf = vec![0u8; needed as usize];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &iface, Some(detail), needed, None, None).is_err() {
                continue;
            }
            let path = PCWSTR((*detail).DevicePath.as_ptr());
            let Ok(h) = CreateFileW(path, (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) else {
                continue;
            };
            let mut attrs = HIDD_ATTRIBUTES { Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
            let ours = HidD_GetAttributes(h, &mut attrs) && attrs.VendorID == SONY && MODELS.contains(&attrs.ProductID);
            if !ours {
                let _ = CloseHandle(h);
                continue;
            }
            let mut pre = PHIDP_PREPARSED_DATA::default();
            let mut caps = HIDP_CAPS::default();
            let ok = HidD_GetPreparsedData(h, &mut pre) && HidP_GetCaps(pre, &mut caps).is_ok();
            if pre.0 != 0 {
                let _ = HidD_FreePreparsedData(pre);
            }
            // Bluetooth exposes the long output report (0x31): 78 bytes.
            if !ok || caps.UsagePage != 1 {
                let _ = CloseHandle(h);
                continue;
            }
            let bt = caps.OutputReportByteLength >= 78;
            found = Some((h, bt, caps.InputReportByteLength as usize));
            break;
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        found
    }
}

fn dualsense_thread(shared: Arc<Mutex<(Pad, PadInfo)>>, hwnd: isize, start: std::time::Instant) {
    let now = || start.elapsed().as_secs_f64() * 1000.0;
    let post = |msg: u32, w: usize| {
        // SAFETY: posting to our window; a closed window drops it.
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(w), LPARAM(0));
        }
    };
    loop {
        let Some((h, bt, len)) = find_dualsense() else {
            std::thread::sleep(std::time::Duration::from_secs(3));
            continue;
        };
        if bt {
            // Reading the calibration report switches a Bluetooth DualSense to full reports.
            let mut feature = [0u8; 41];
            feature[0] = 0x05;
            // SAFETY: the buffer is the feature report's size.
            let _ = unsafe { HidD_GetFeature(h, feature.as_mut_ptr().cast(), feature.len() as u32) };
        }
        let mut buf = vec![0u8; len.max(78)];
        let (mut ps_held, mut combo_held, mut last_sig) = (false, false, 0u32);
        loop {
            let mut read = 0u32;
            // SAFETY: blocking read into our buffer.
            let ok = unsafe { ReadFile(h, Some(&mut buf[..len.max(1)]), Some(&mut read), None) }.is_ok();
            if !ok || read == 0 {
                break;
            }
            let base = match buf[0] {
                0x31 => 2,                       // Bluetooth, full mode
                0x01 if !bt && read >= 64 => 1,  // USB
                _ => continue,                   // Bluetooth basic mode: switching
            };
            let v = &buf[base..];
            let (b0, b1, b2) = (v[7], v[8], v[9]);
            let hat = b0 & 0x0f;
            let mut p = Pad { at: now(), ..Pad::default() };
            let on = [
                b0 & 0x20 != 0, // cross
                b0 & 0x40 != 0, // circle
                b0 & 0x10 != 0, // square
                b0 & 0x80 != 0, // triangle
                b1 & 0x01 != 0,
                b1 & 0x02 != 0,
                b1 & 0x04 != 0,
                b1 & 0x08 != 0,
                b1 & 0x10 != 0, // create
                b1 & 0x20 != 0, // options
                b1 & 0x40 != 0,
                b1 & 0x80 != 0,
                hat == 7 || hat == 0 || hat == 1,
                (3..=5).contains(&hat),
                (5..=7).contains(&hat),
                (1..=3).contains(&hat),
                b2 & 0x01 != 0, // PS
                b2 & 0x02 != 0, // touchpad
            ];
            p.buttons = on;
            p.axes = [(f32::from(v[0]) - 128.0) / 128.0, (f32::from(v[1]) - 128.0) / 128.0, (f32::from(v[2]) - 128.0) / 128.0, (f32::from(v[3]) - 128.0) / 128.0];
            let status = v[52];
            let info = PadInfo { connected: true, battery: Some(((status & 0x0f) * 10 + 5).min(100)), charging: matches!(status >> 4, 1 | 2) };
            if let Ok(mut g) = shared.lock() {
                *g = (p, info);
            }
            // Reports arrive hundreds of times a second (motion sensors); the UI only wakes
            // when a button or a stick direction actually changed.
            let mut sig = on.iter().enumerate().fold(0u32, |s, (i, &b)| s | (u32::from(b) << i));
            for (i, a) in p.axes[..2].iter().enumerate() {
                sig |= (u32::from(*a < -0.55) << (20 + 2 * i)) | (u32::from(*a > 0.55) << (21 + 2 * i));
            }
            if sig != last_sig {
                last_sig = sig;
                post(WM_PAD_INPUT, 0);
            }
            // The PS button (or Create + Options) reaches Sofa even while a game has focus.
            let combo = on[8] && on[9];
            if (on[16] && !ps_held) || (combo && !combo_held) {
                post(WM_PAD_HOME, usize::from(!(on[16] && !ps_held)));
            }
            ps_held = on[16];
            combo_held = combo;
        }
        // SAFETY: closing the handle this loop opened.
        unsafe {
            let _ = CloseHandle(h);
        }
        if let Ok(mut g) = shared.lock() {
            g.1 = PadInfo::default();
        }
    }
}
