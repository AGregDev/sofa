//! Controllers on Linux.
//!
//! - Every pad through evdev (`/dev/input/event*`, readable by the person at the seat): the
//!   kernel's drivers (hid-playstation, xpad, hid-nintendo) turn them into buttons and axes, and
//!   reading them does not depend on which window has the focus, so the PS (Guide) button
//!   reaches Sofa while a game is in front. Steam Input's virtual pads are skipped.
//! - A DualSense also through hidraw when Steam's udev rules allow it (`steam-devices`): the
//!   same reports and lights as on Windows (`input::dualsense_session`), plus its mute button,
//!   which the kernel driver keeps to itself. Without hidraw it is read through evdev like any
//!   pad, its battery from `/sys/class/power_supply`, and it has no lights or mute button.

use crate::input::{Hid, Lights, Pad, PadInfo, PadKind, WM_PAD_INPUT, dualsense_session};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the evdev pads report, for `Input`.
#[derive(Clone, Copy, Default)]
pub struct EvState {
    /// The pad last used, as Sofa's standard buttons.
    pub pad: Option<Pad>,
    pub kind: Option<PadKind>,
    /// A DualSense read through evdev (no hidraw): its charge, from sysfs.
    pub sony: Option<PadInfo>,
    pub count: usize,
}

// ioctl request numbers (the generic _IOC layout: x86-64 and ARM64).
const fn ioc(dir: u64, ty: u8, nr: u8, size: u64) -> u64 {
    (dir << 30) | (size << 16) | ((ty as u64) << 8) | nr as u64
}
const READ: u64 = 2;
const WRITE: u64 = 1;
const EVIOCGID: u64 = ioc(READ, b'E', 0x02, 8);
const fn eviocgbit(ev: u8, len: u64) -> u64 {
    ioc(READ, b'E', 0x20 + ev, len)
}
const fn eviocgabs(abs: u8) -> u64 {
    ioc(READ, b'E', 0x40 + abs, 24)
}
const fn hidiocgfeature(len: u64) -> u64 {
    ioc(READ | WRITE, b'H', 0x07, len)
}

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const BTN_SOUTH: u16 = 0x130;

const SONY: u16 = 0x054c;
const NINTENDO: u16 = 0x057e;
const DUALSENSE: [u16; 2] = [0x0ce6, 0x0df2];

#[derive(Clone, Copy, Default)]
struct Abs {
    min: i32,
    max: i32,
}

impl Abs {
    /// -1 to 1 from the centre.
    fn norm(&self, v: i32) -> f32 {
        let (mid, half) = ((self.min + self.max) as f32 / 2.0, ((self.max - self.min) as f32 / 2.0).max(1.0));
        ((v as f32 - mid) / half).clamp(-1.0, 1.0)
    }
    /// 0 to 1 (a trigger).
    fn amount(&self, v: i32) -> f32 {
        ((v - self.min) as f32 / ((self.max - self.min) as f32).max(1.0)).clamp(0.0, 1.0)
    }
}

struct Dev {
    file: File,
    path: PathBuf,
    vendor: u16,
    product: u16,
    abs: [Abs; 0x12],
    hat: (i32, i32),
    /// D-pad buttons (`BTN_DPAD_*`), for pads without a hat.
    dpad: [bool; 4],
    triggers: (f32, f32),
    sticks: [f32; 4],
    pad: Pad,
}

impl Dev {
    fn positional(&self) -> bool {
        matches!(self.vendor, SONY | NINTENDO)
    }

    /// An evdev button code as a standard gamepad button.
    fn button(&self, code: u16) -> Option<usize> {
        Some(match code {
            0x130 => 0,                                 // south: ✕, A
            0x131 => 1,                                 // east: ○, B
            0x133 => if self.positional() { 3 } else { 2 }, // north (△) here, xpad's X
            0x134 => if self.positional() { 2 } else { 3 }, // west (□) here, xpad's Y
            0x136 => 4,                                 // L1
            0x137 => 5,                                 // R1
            0x138 => 6,                                 // L2
            0x139 => 7,                                 // R2
            0x13a => 8,                                 // Create, Back
            0x13b => 9,                                 // Options, Start
            0x13d => 10,
            0x13e => 11,
            0x13c => 16, // PS, Guide
            _ => return None,
        })
    }

    fn apply(&mut self, ty: u16, code: u16, value: i32, now: f64) {
        match ty {
            EV_KEY => {
                if let Some(i) = self.button(code) {
                    self.pad.buttons[i] = value != 0;
                } else if (0x220..=0x223).contains(&code) {
                    self.dpad[(code - 0x220) as usize] = value != 0;
                }
            }
            EV_ABS => {
                let a = self.abs.get(code as usize).copied().unwrap_or_default();
                match code {
                    0x00 => self.sticks[0] = a.norm(value),
                    0x01 => self.sticks[1] = a.norm(value),
                    0x03 => self.sticks[2] = a.norm(value),
                    0x04 => self.sticks[3] = a.norm(value),
                    0x02 => self.triggers.0 = a.amount(value),
                    0x05 => self.triggers.1 = a.amount(value),
                    0x10 => self.hat.0 = value.signum(),
                    0x11 => self.hat.1 = value.signum(),
                    _ => {}
                }
            }
            _ => return,
        }
        self.pad.at = now;
    }

    /// The pad as Sofa reads it, once the report is complete (SYN_REPORT).
    fn report(&mut self) -> Pad {
        let mut p = self.pad;
        p.buttons[6] |= self.triggers.0 > 0.5;
        p.buttons[7] |= self.triggers.1 > 0.5;
        p.buttons[12] |= self.hat.1 < 0 || self.dpad[0];
        p.buttons[13] |= self.hat.1 > 0 || self.dpad[1];
        p.buttons[14] |= self.hat.0 < 0 || self.dpad[2];
        p.buttons[15] |= self.hat.0 > 0 || self.dpad[3];
        p.axes = self.sticks;
        p
    }
}

fn ioctl(fd: i32, req: u64, buf: *mut u8) -> bool {
    // SAFETY: `buf` points to a buffer at least as long as the size encoded in `req`.
    unsafe { libc::ioctl(fd, req as _, buf) >= 0 }
}

/// Opens an evdev node when it is a real gamepad (not a keyboard, not Steam's virtual pad).
fn open_pad(path: &Path) -> Option<Dev> {
    let name = path.file_name()?.to_str()?;
    // Virtual devices (uinput: Steam Input's pad, other remappers) live under /devices/virtual.
    let sys = std::fs::read_link(format!("/sys/class/input/{name}")).ok()?;
    if sys.to_string_lossy().contains("/virtual/") {
        return None;
    }
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path).ok()?;
    let fd = file.as_raw_fd();
    let mut keys = [0u8; 0x300 / 8];
    if !ioctl(fd, eviocgbit(EV_KEY as u8, keys.len() as u64), keys.as_mut_ptr()) || keys[(BTN_SOUTH / 8) as usize] & (1 << (BTN_SOUTH % 8)) == 0 {
        return None;
    }
    let mut id = [0u8; 8];
    ioctl(fd, EVIOCGID, id.as_mut_ptr());
    let (vendor, product) = (u16::from_ne_bytes([id[2], id[3]]), u16::from_ne_bytes([id[4], id[5]]));
    let mut abs = [Abs::default(); 0x12];
    for (code, a) in abs.iter_mut().enumerate() {
        let mut info = [0i32; 6];
        if ioctl(fd, eviocgabs(code as u8), info.as_mut_ptr().cast()) {
            *a = Abs { min: info[1], max: info[2] };
        }
    }
    Some(Dev { file, path: path.to_path_buf(), vendor, product, abs, hat: (0, 0), dpad: [false; 4], triggers: (0.0, 0.0), sticks: [0.0; 4], pad: Pad::default() })
}

/// A DualSense's charge from the kernel driver (`ps-controller-battery-*`).
fn sony_battery() -> PadInfo {
    let mut info = PadInfo { connected: true, ..PadInfo::default() };
    if let Ok(list) = std::fs::read_dir("/sys/class/power_supply") {
        for e in list.flatten() {
            if !e.file_name().to_string_lossy().starts_with("ps-controller-battery") {
                continue;
            }
            let p = e.path();
            info.battery = std::fs::read_to_string(p.join("capacity")).ok().and_then(|s| s.trim().parse().ok());
            info.charging = std::fs::read_to_string(p.join("status")).is_ok_and(|s| matches!(s.trim(), "Charging" | "Full"));
            break;
        }
    }
    info
}

/// Reads every evdev pad on the `pads` thread; wakes the UI when the buttons changed.
pub fn evdev_thread(state: Arc<Mutex<EvState>>, ds_hid: Arc<AtomicBool>, start: Instant) {
    let now = || start.elapsed().as_secs_f64() * 1000.0;
    let mut devs: Vec<Dev> = Vec::new();
    let (mut scan_at, mut battery_at) = (0.0f64, 0.0f64);
    let mut last_sig = u64::MAX;
    let mut sony_info: Option<PadInfo> = None;
    let mut buf = [0u8; 24 * 64];
    loop {
        let t = now();
        if t >= scan_at {
            scan_at = t + 2000.0;
            if let Ok(list) = std::fs::read_dir("/dev/input") {
                for e in list.flatten() {
                    let p = e.path();
                    if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event")) && !devs.iter().any(|d| d.path == p)
                        && let Some(d) = open_pad(&p)
                    {
                        crate::log(&format!("pad: {} ({:04x}:{:04x})", p.display(), d.vendor, d.product));
                        devs.push(d);
                    }
                }
            }
        }
        // The DualSense goes through hidraw while that works: its evdev node would be the same
        // presses twice.
        let hid = ds_hid.load(Ordering::Relaxed);
        let is_ds = |d: &Dev| d.vendor == SONY && DUALSENSE.contains(&d.product);
        let mut fds: Vec<libc::pollfd> = devs.iter().map(|d| libc::pollfd { fd: d.file.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
        // SAFETY: `fds` holds one valid pollfd per open device for the call.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 1000) };
        let mut gone = Vec::new();
        let mut changed = false;
        if n > 0 {
            for (i, f) in fds.iter().enumerate() {
                if f.revents == 0 {
                    continue;
                }
                if f.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    gone.push(i);
                    continue;
                }
                let d = &mut devs[i];
                loop {
                    match d.file.read(&mut buf) {
                        Ok(0) => break,
                        Ok(len) => {
                            for ev in buf[..len].as_chunks::<24>().0 {
                                let ty = u16::from_ne_bytes([ev[16], ev[17]]);
                                let code = u16::from_ne_bytes([ev[18], ev[19]]);
                                let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                                if ty == EV_SYN {
                                    changed = true;
                                } else {
                                    d.apply(ty, code, value, now());
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            gone.push(i);
                            break;
                        }
                    }
                }
            }
        }
        for i in gone.into_iter().rev() {
            crate::log(&format!("pad gone: {}", devs[i].path.display()));
            devs.remove(i);
            changed = true;
        }
        let t = now();
        let sony_here = devs.iter().any(|d| is_ds(d) || d.vendor == SONY);
        if sony_here && !hid && (sony_info.is_none() || t >= battery_at) {
            battery_at = t + 30_000.0;
            sony_info = Some(sony_battery());
            changed = true;
        } else if (!sony_here || hid) && sony_info.is_some() {
            sony_info = None;
            changed = true;
        }
        if !changed {
            continue;
        }
        // The pad used last drives the UI.
        let mut best: Option<(Pad, PadKind)> = None;
        for d in devs.iter_mut().filter(|d| !(hid && is_ds(d))) {
            let p = d.report();
            let kind = if d.vendor == SONY { PadKind::PlayStation } else { PadKind::Xbox };
            let active = p.buttons.iter().any(|&b| b) || p.axes[0].abs() > 0.55 || p.axes[1].abs() > 0.55;
            if best.is_none_or(|(b, _)| active && (p.at > b.at || !b.buttons.iter().any(|&x| x))) {
                best = Some((p, kind));
            }
        }
        let count = devs.iter().filter(|d| !(hid && is_ds(d))).count();
        let sig = best.map_or(0, |(p, _)| {
            let mut s = p.buttons.iter().enumerate().fold(1u64, |s, (i, &b)| s | (u64::from(b) << (i + 1)));
            for (i, a) in p.axes[..2].iter().enumerate() {
                s |= (u64::from(*a < -0.55) << (40 + 2 * i)) | (u64::from(*a > 0.55) << (41 + 2 * i));
            }
            s
        }) ^ ((count as u64) << 56)
            ^ sony_info.map_or(0, |i| u64::from(i.battery.unwrap_or(0)) << 48);
        if let Ok(mut g) = state.lock() {
            *g = EvState { pad: best.map(|b| b.0), kind: best.map(|b| b.1), sony: sony_info, count };
        }
        if sig != last_sig {
            last_sig = sig;
            crate::post(0, WM_PAD_INPUT, 0);
        }
    }
}

// ---------- DualSense over hidraw ----------

struct LinuxHid(File);

impl Hid for LinuxHid {
    fn read(&mut self, buf: &mut [u8]) -> Option<usize> {
        self.0.read(buf).ok().filter(|&n| n > 0)
    }
    fn write(&mut self, report: &[u8]) -> bool {
        self.0.write_all(report).is_ok()
    }
}

/// The first DualSense on hidraw that Sofa may open, and whether it is on Bluetooth.
fn find_hidraw(denied: &mut Vec<String>) -> Option<(File, bool)> {
    let list = std::fs::read_dir("/sys/class/hidraw").ok()?;
    for e in list.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(uevent) = std::fs::read_to_string(e.path().join("device/uevent")) else { continue };
        let Some(id) = uevent.lines().find_map(|l| l.strip_prefix("HID_ID=")) else { continue };
        // HID_ID=0005:0000054C:00000CE6 (bus, vendor, product).
        let parts: Vec<u32> = id.split(':').filter_map(|p| u32::from_str_radix(p, 16).ok()).collect();
        let [bus, vendor, product] = parts[..] else { continue };
        if vendor != u32::from(SONY) || !DUALSENSE.contains(&(product as u16)) {
            continue;
        }
        match std::fs::OpenOptions::new().read(true).write(true).open(format!("/dev/{name}")) {
            Ok(f) => return Some((f, bus == 5)),
            Err(err) => {
                if !denied.contains(&name) {
                    crate::log(&format!("dualsense: /dev/{name}: {err} (install Steam's udev rules, steam-devices, for its lights and mute button)"));
                    denied.push(name);
                }
            }
        }
    }
    None
}

/// The `dualsense` thread: a DualSense over hidraw, as on Windows (buttons, battery, the mute
/// button, and its lights written only on change).
pub fn dualsense_thread(shared: Arc<Mutex<(Pad, PadInfo)>>, wanted: Arc<Mutex<Lights>>, ds_hid: Arc<AtomicBool>, start: Instant) {
    let mut denied = Vec::new();
    loop {
        let Some((file, bt)) = find_hidraw(&mut denied) else {
            std::thread::sleep(Duration::from_secs(3));
            continue;
        };
        if bt {
            // Reading the calibration report switches a Bluetooth DualSense to full reports
            // (the kernel driver has usually done it already).
            let mut feature = [0u8; 41];
            feature[0] = 0x05;
            ioctl(file.as_raw_fd(), hidiocgfeature(feature.len() as u64), feature.as_mut_ptr());
        }
        ds_hid.store(true, Ordering::Relaxed);
        crate::log(&format!("dualsense: hidraw, {}", if bt { "Bluetooth" } else { "USB" }));
        dualsense_session(&mut LinuxHid(file), bt, 128, 0, &shared, &wanted, 0, start);
        ds_hid.store(false, Ordering::Relaxed);
        if let Ok(mut g) = shared.lock() {
            g.1 = PadInfo::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_the_kernel_headers() {
        assert_eq!(EVIOCGID, 0x8008_4502);
        assert_eq!(eviocgbit(1, 96), 0x8060_4521);
        assert_eq!(eviocgabs(0), 0x8018_4540);
        assert_eq!(hidiocgfeature(41), 0xC029_4807);
    }

    #[test]
    fn sticks_and_triggers_are_scaled() {
        let a = Abs { min: 0, max: 255 };
        assert!(a.norm(0) < -0.99 && a.norm(255) > 0.99 && a.norm(128).abs() < 0.01);
        assert!((a.amount(255) - 1.0).abs() < 1e-6);
    }
}
