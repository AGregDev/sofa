//! What the Performance panel (quick menu) and the performance overlay show: the memory in use,
//! each graphics card's video memory, Sofa's own memory, and how fast Sofa draws. Read once a
//! second, and only while the panel or the overlay is up.
//!
//! Windows: GlobalMemoryStatusEx, the process's working set, DXGI's adapters and the "GPU
//! Adapter Memory" performance counters (video memory in use by every program). Linux:
//! `/proc/meminfo`, `/proc/self/status`, NVIDIA's NVML (loaded at run time) and amdgpu's
//! `mem_info_vram_*` files. Nothing is changed anywhere, so a QA run reads them too.

use std::collections::VecDeque;

/// A graphics card's video memory (`used` None: the card does not say).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub used: Option<u64>,
    pub total: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub ram_used: u64,
    pub ram_total: u64,
    /// Sofa's own memory (working set, resident).
    pub sofa: u64,
    pub gpus: Vec<Gpu>,
}

pub fn read() -> Usage {
    let (ram_used, ram_total) = ram();
    Usage { ram_used, ram_total, sofa: own(), gpus: gpus() }
}

/// The frames Sofa drew in the last second and how long each took to draw.
#[derive(Default)]
pub struct Frames {
    times: VecDeque<(f64, f32)>,
}

impl Frames {
    pub fn record(&mut self, at: f64, ms: f32) {
        self.times.push_back((at, ms));
        while self.times.front().is_some_and(|(t, _)| at - t > 1000.0) {
            self.times.pop_front();
        }
    }

    /// Frames drawn in the second before `now`.
    pub fn fps(&self, now: f64) -> usize {
        self.times.iter().filter(|(t, _)| now - t <= 1000.0).count()
    }

    /// The average time a frame took to draw, over that second.
    pub fn ms(&self, now: f64) -> Option<f32> {
        let recent: Vec<f32> = self.times.iter().filter(|(t, _)| now - t <= 1000.0).map(|(_, ms)| *ms).collect();
        (!recent.is_empty()).then(|| recent.iter().sum::<f32>() / recent.len() as f32)
    }
}

/// "6.1 GB".
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

/// "64 MB".
pub fn mb(bytes: u64) -> String {
    format!("{} MB", bytes >> 20)
}

/// "6.1 of 15.9 GB".
pub fn of(used: u64, total: u64) -> String {
    format!("{:.1} of {}", used as f64 / (1u64 << 30) as f64, gb(total))
}

pub fn percent(used: u64, total: u64) -> u32 {
    if total == 0 { 0 } else { (used as f64 * 100.0 / total as f64).round() as u32 }
}

// ---------- Windows ----------

#[cfg(windows)]
fn ram() -> (u64, u64) {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: writes into a local struct whose size it is given.
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() { (m.ullTotalPhys.saturating_sub(m.ullAvailPhys), m.ullTotalPhys) } else { (0, 0) }
}

#[cfg(windows)]
fn own() -> u64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    // SAFETY: our own process; writes into a local struct whose size it is given.
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) }.is_ok() { c.WorkingSetSize as u64 } else { 0 }
}

#[cfg(windows)]
fn gpus() -> Vec<Gpu> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    let mut out = Vec::new();
    // SAFETY: plain factory creation.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    let usage = dedicated_usage();
    for i in 0..16 {
        // SAFETY: enumerating adapters until DXGI says there are no more.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        // SAFETY: a description copied into a local.
        let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
        // DXGI_ADAPTER_FLAG_SOFTWARE (Microsoft's software renderer), and adapters without
        // video memory of their own, are left out.
        if d.Flags & 2 != 0 || d.DedicatedVideoMemory == 0 {
            continue;
        }
        let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
        let used = usage.iter().find(|(l, _)| *l == (d.AdapterLuid.HighPart, d.AdapterLuid.LowPart)).map(|(_, u)| *u);
        out.push(Gpu { name: String::from_utf16_lossy(&d.Description[..n]).trim().to_owned(), used, total: d.DedicatedVideoMemory as u64 });
    }
    out
}

/// "luid_0x00000000_0x0000F3D8_phys_0": an adapter's LUID (high, low).
#[cfg(any(windows, test))]
fn parse_luid(name: &str) -> Option<(i32, u32)> {
    let rest = name.strip_prefix("luid_0x")?;
    let (high, rest) = rest.split_once("_0x")?;
    let low = rest.split('_').next()?;
    Some((u32::from_str_radix(high, 16).ok()? as i32, u32::from_str_radix(low, 16).ok()?))
}

/// Video memory in use on each adapter, by every program: the "GPU Adapter Memory" counters.
/// The query stays open for the next read.
#[cfg(windows)]
fn dedicated_usage() -> Vec<((i32, u32), u64)> {
    use std::cell::Cell;
    use windows::Win32::System::Performance::*;
    use windows::core::{PCWSTR, w};
    thread_local! {
        static QUERY: Cell<Option<(PDH_HQUERY, PDH_HCOUNTER)>> = const { Cell::new(None) };
    }
    let handles = QUERY.with(|q| {
        if q.get().is_none() {
            let (mut query, mut counter) = (PDH_HQUERY::default(), PDH_HCOUNTER::default());
            // SAFETY: the query and counter handles are written into locals; a failed counter
            // closes its query.
            unsafe {
                if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                    return None;
                }
                if PdhAddEnglishCounterW(query, w!("\\GPU Adapter Memory(*)\\Dedicated Usage"), 0, &mut counter) != 0 {
                    PdhCloseQuery(query);
                    return None;
                }
            }
            q.set(Some((query, counter)));
        }
        q.get()
    });
    let Some((query, counter)) = handles else { return Vec::new() };
    let mut out: Vec<((i32, u32), u64)> = Vec::new();
    // SAFETY: the counter's values are read into a buffer of the size PDH asks for (u64s, so its
    // items, which hold pointers and 64-bit values, are aligned); each item's name points into
    // that buffer and is read while it lives.
    unsafe {
        if PdhCollectQueryData(query) != 0 {
            return out;
        }
        let (mut size, mut count) = (0u32, 0u32);
        if PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut size, &mut count, None) != PDH_MORE_DATA || size == 0 {
            return out;
        }
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        if PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut size, &mut count, Some(items)) != 0 {
            return out;
        }
        for i in 0..count as usize {
            let item = &*items.add(i);
            let Some(luid) = item.szName.to_string().ok().and_then(|n| parse_luid(&n)) else { continue };
            let v = item.FmtValue.Anonymous.largeValue.max(0) as u64;
            match out.iter_mut().find(|(l, _)| *l == luid) {
                Some((_, sum)) => *sum += v,
                None => out.push((luid, v)),
            }
        }
    }
    out
}

// ---------- Linux ----------

#[cfg(target_os = "linux")]
fn ram() -> (u64, u64) {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()).unwrap_or(0) * 1024;
    let (total, available) = (field("MemTotal:"), field("MemAvailable:"));
    (total.saturating_sub(available), total)
}

#[cfg(target_os = "linux")]
fn own() -> u64 {
    std::fs::read_to_string("/proc/self/status").ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix("VmRSS:")?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok())).unwrap_or(0) * 1024
}

#[cfg(target_os = "linux")]
fn gpus() -> Vec<Gpu> {
    let mut out = nvml::gpus();
    // amdgpu reports its video memory in sysfs.
    if let Ok(list) = std::fs::read_dir("/sys/class/drm") {
        let mut cards: Vec<std::path::PathBuf> = list.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.strip_prefix("card").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())))).collect();
        cards.sort();
        for card in cards {
            let dev = card.join("device");
            let read = |f: &str| std::fs::read_to_string(dev.join(f)).ok().and_then(|s| s.trim().parse::<u64>().ok());
            let (Some(total), used) = (read("mem_info_vram_total"), read("mem_info_vram_used")) else { continue };
            if total == 0 {
                continue;
            }
            let name = std::fs::read_to_string(dev.join("product_name")).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).unwrap_or_else(|| "AMD Radeon".into());
            out.push(Gpu { name, used, total });
        }
    }
    out
}

/// NVIDIA's management library, loaded when there is one (the proprietary driver ships it).
#[cfg(target_os = "linux")]
mod nvml {
    use super::Gpu;
    use std::ffi::{c_char, c_int, c_uint, c_void};
    use std::sync::OnceLock;

    #[repr(C)]
    struct Memory {
        total: u64,
        free: u64,
        used: u64,
    }

    type Count = unsafe extern "C" fn(*mut c_uint) -> c_int;
    type Handle = unsafe extern "C" fn(c_uint, *mut *mut c_void) -> c_int;
    type MemInfo = unsafe extern "C" fn(*mut c_void, *mut Memory) -> c_int;
    type Name = unsafe extern "C" fn(*mut c_void, *mut c_char, c_uint) -> c_int;

    struct Lib {
        count: Count,
        handle: Handle,
        memory: MemInfo,
        name: Name,
    }

    // SAFETY: plain function pointers into a library that stays loaded.
    unsafe impl Send for Lib {}
    // SAFETY: as above; NVML's calls are thread-safe.
    unsafe impl Sync for Lib {}

    fn lib() -> Option<&'static Lib> {
        static LIB: OnceLock<Option<Lib>> = OnceLock::new();
        LIB.get_or_init(|| {
            // SAFETY: dlopen and dlsym with NUL-terminated names; each symbol is cast to the type
            // nvml.h declares for it, and nvmlInit_v2 runs once before any other call.
            unsafe {
                let h = libc::dlopen(c"libnvidia-ml.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if h.is_null() {
                    return None;
                }
                let sym = |n: &std::ffi::CStr| {
                    let p = libc::dlsym(h, n.as_ptr());
                    (!p.is_null()).then_some(p)
                };
                let init = std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> c_int>(sym(c"nvmlInit_v2")?);
                if init() != 0 {
                    return None;
                }
                Some(Lib {
                    count: std::mem::transmute::<*mut c_void, Count>(sym(c"nvmlDeviceGetCount_v2")?),
                    handle: std::mem::transmute::<*mut c_void, Handle>(sym(c"nvmlDeviceGetHandleByIndex_v2")?),
                    memory: std::mem::transmute::<*mut c_void, MemInfo>(sym(c"nvmlDeviceGetMemoryInfo")?),
                    name: std::mem::transmute::<*mut c_void, Name>(sym(c"nvmlDeviceGetName")?),
                })
            }
        })
        .as_ref()
    }

    pub fn gpus() -> Vec<Gpu> {
        let Some(l) = lib() else { return Vec::new() };
        let mut out = Vec::new();
        let mut n: c_uint = 0;
        // SAFETY: NVML writes into our locals and buffers, each passed with its size.
        unsafe {
            if (l.count)(&mut n) != 0 {
                return out;
            }
            for i in 0..n.min(8) {
                let mut dev: *mut c_void = std::ptr::null_mut();
                if (l.handle)(i, &mut dev) != 0 {
                    continue;
                }
                let mut m = Memory { total: 0, free: 0, used: 0 };
                if (l.memory)(dev, &mut m) != 0 {
                    continue;
                }
                let mut buf = [0 as c_char; 96];
                let name = if (l.name)(dev, buf.as_mut_ptr(), buf.len() as c_uint) == 0 {
                    std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
                } else {
                    "NVIDIA".into()
                };
                out.push(Gpu { name, used: Some(m.used), total: m.total });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_count_the_last_second() {
        let mut f = Frames::default();
        for i in 0..60 {
            f.record(1000.0 + f64::from(i) * 16.0, 3.0);
        }
        assert_eq!(f.fps(1950.0), 60);
        assert!((f.ms(1950.0).unwrap_or(0.0) - 3.0).abs() < 1e-6);
        assert_eq!(f.fps(3500.0), 0, "nothing drawn for a second: idle");
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(gb(6_549_825_536), "6.1 GB");
        assert_eq!(mb(64 << 20), "64 MB");
        assert_eq!(of(3 << 30, 16 << 30), "3.0 of 16.0 GB");
        assert_eq!(percent(1, 4), 25);
        assert_eq!(percent(1, 0), 0);
    }

    #[test]
    fn adapter_luids_are_read_from_counter_names() {
        assert_eq!(parse_luid("luid_0x00000000_0x0000F3D8_phys_0"), Some((0, 0xF3D8)));
        assert_eq!(parse_luid("something else"), None);
    }

    #[test]
    fn this_computer_has_memory() {
        let u = read();
        assert!(u.ram_total > 0 && u.ram_used <= u.ram_total);
        assert!(u.sofa > 0);
    }
}
