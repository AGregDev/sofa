//! What the Performance panel (quick menu) and the overlay show: CPU and GPU use, the memory in
//! use, each graphics card's video memory, and the frame rate of the program in front. Read once
//! a second, and only while the panel or the overlay is up (the quick menu's card: memory alone).
//!
//! What video memory and GPU use are read from is slow to open (PDH's counters take about half a
//! second to add, NVML about a tenth to start), so the first read opens it on a short-lived
//! thread and the UI thread never waits; reading it after that takes under a millisecond.
//!
//! Windows: GetSystemTimes, GlobalMemoryStatusEx, DXGI's adapters, the "GPU Adapter Memory" and
//! "GPU Engine" performance counters, and frames from an ETW session of Sofa's own (`frames`).
//! Linux: `/proc/stat`, `/proc/meminfo`, NVIDIA's NVML (loaded at run time) and amdgpu's files;
//! no frame rate (nothing there tells Sofa another program's frames). Reading changes nothing,
//! so a QA run reads them too; it never starts the ETW session.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// A graphics card: its video memory (`used` None: the card does not say) and how busy it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub used: Option<u64>,
    pub total: u64,
    /// In percent (None: the card does not say).
    pub busy: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// CPU use since the last read, in percent (None: the first read).
    pub cpu: Option<u32>,
    pub ram_used: u64,
    pub ram_total: u64,
    /// None: not asked for, or still being opened.
    pub gpus: Option<Vec<Gpu>>,
}

/// CPU use and the memory in use now, and each card's video memory and use when `video` is set.
pub fn read(video: bool) -> Usage {
    let (ram_used, ram_total) = ram();
    Usage { cpu: cpu(), ram_used, ram_total, gpus: if video { gpus() } else { None } }
}

/// The card games run on: the one with the most video memory.
pub fn main_gpu(gpus: &[Gpu]) -> Option<&Gpu> {
    gpus.iter().max_by_key(|g| g.total)
}

/// CPU use since the last call, from the system's busy and total time.
fn cpu() -> Option<u32> {
    static LAST: Mutex<Option<(u64, u64)>> = Mutex::new(None);
    let (busy, all) = cpu_times()?;
    let mut last = LAST.lock().ok()?;
    let out = last.and_then(|(b, a)| (all > a).then(|| (busy.saturating_sub(b) as f64 * 100.0 / (all - a) as f64).round().min(100.0) as u32));
    *last = Some((busy, all));
    out
}

/// Each card's video memory and use, or None while what they are read from is being opened.
fn gpus() -> Option<Vec<Gpu>> {
    static SOURCE: OnceLock<Source> = OnceLock::new();
    static OPENING: AtomicBool = AtomicBool::new(false);
    if let Some(source) = SOURCE.get() {
        return Some(source.read());
    }
    if !OPENING.swap(true, Ordering::AcqRel)
        && std::thread::Builder::new()
            .name("perf".into())
            .spawn(|| {
                let _ = SOURCE.set(Source::open());
            })
            .is_err()
    {
        OPENING.store(false, Ordering::Release);
    }
    None
}

/// A card's name with its spaces tidied ("AMD  Radeon RX 7800 XT " is "AMD Radeon RX 7800 XT").
fn tidy(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "6.1 GB".
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

/// "6.1 of 15.9 GB".
pub fn of(used: u64, total: u64) -> String {
    format!("{:.1} of {}", used as f64 / (1u64 << 30) as f64, gb(total))
}

pub fn percent(used: u64, total: u64) -> u32 {
    if total == 0 { 0 } else { (used as f64 * 100.0 / total as f64).round() as u32 }
}

/// The frame rate of the program in front (None: not known, or Sofa itself is in front).
pub fn frame_rate() -> Option<u32> {
    #[cfg(windows)]
    {
        frames::in_front()
    }
    #[cfg(target_os = "linux")]
    {
        None
    }
}

/// Watches the frames programs present while `on` (the overlay is up; never in a QA run).
pub fn watch_frames(on: bool) {
    #[cfg(windows)]
    frames::watch(on);
    #[cfg(target_os = "linux")]
    let _ = on;
}

/// Whether Sofa may watch frames: None where it has no way to (Linux), Some(false) when Windows
/// refused (Sofa is not in Performance Log Users).
pub fn frames_allowed() -> Option<bool> {
    #[cfg(windows)]
    {
        Some(frames::allowed() && frames::state() != frames::State::NotAllowed)
    }
    #[cfg(target_os = "linux")]
    {
        None
    }
}

/// Asks the system to let Sofa watch frames (Windows' administrator prompt; from the next
/// sign-in). False when it could not ask.
pub fn allow_frames() -> bool {
    #[cfg(windows)]
    {
        frames::allow()
    }
    #[cfg(target_os = "linux")]
    {
        false
    }
}

// ---------- Windows ----------

#[cfg(windows)]
fn ram() -> (u64, u64) {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: writes into a local struct whose size it is given.
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() { (m.ullTotalPhys.saturating_sub(m.ullAvailPhys), m.ullTotalPhys) } else { (0, 0) }
}

/// Busy and total CPU time (kernel time includes idle time).
#[cfg(windows)]
fn cpu_times() -> Option<(u64, u64)> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetSystemTimes;
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: three local FILETIMEs written by the call.
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let v = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    let all = v(kernel) + v(user);
    Some((all.saturating_sub(v(idle)), all))
}

/// The cards DXGI lists (their names, LUIDs and video memory, read once) and PDH's query of the
/// "GPU Adapter Memory" and "GPU Engine" counters, which stays open for the next read.
#[cfg(windows)]
struct Source {
    cards: Vec<(String, Luid, u64)>,
    /// The query and its two counters (the engines' may be missing), as the addresses PDH's
    /// handles are: PDH may be called with them from any thread, and it locks each query itself.
    counters: Option<(usize, usize, Option<usize>)>,
}

#[cfg(windows)]
impl Source {
    fn open() -> Source {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        use windows::Win32::System::Performance::*;
        use windows::core::{PCWSTR, w};
        let mut cards = Vec::new();
        // SAFETY: plain factory creation.
        if let Ok(factory) = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() } {
            for i in 0..16 {
                // SAFETY: enumerating adapters until DXGI says there are no more.
                let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
                // SAFETY: a description copied into a local.
                let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
                // DXGI_ADAPTER_FLAG_SOFTWARE (Microsoft's software renderer), and adapters
                // without video memory of their own, are left out.
                if d.Flags & 2 != 0 || d.DedicatedVideoMemory == 0 {
                    continue;
                }
                let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
                cards.push((tidy(&String::from_utf16_lossy(&d.Description[..n])), (d.AdapterLuid.HighPart, d.AdapterLuid.LowPart), d.DedicatedVideoMemory as u64));
            }
        }
        let (mut query, mut memory, mut engines) = (PDH_HQUERY::default(), PDH_HCOUNTER::default(), PDH_HCOUNTER::default());
        // SAFETY: the query and counter handles are written into locals; a failed memory counter
        // closes its query. The first collection starts the engines' rate, so the next read
        // has one.
        let counters = unsafe {
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                None
            } else if PdhAddEnglishCounterW(query, w!("\\GPU Adapter Memory(*)\\Dedicated Usage"), 0, &mut memory) != 0 {
                PdhCloseQuery(query);
                None
            } else {
                let engines = (PdhAddEnglishCounterW(query, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut engines) == 0).then_some(engines.0 as usize);
                let _ = PdhCollectQueryData(query);
                Some((query.0 as usize, memory.0 as usize, engines))
            }
        };
        Source { cards, counters }
    }

    fn read(&self) -> Vec<Gpu> {
        let (memory, busy) = self.counters.map(|(q, m, e)| sample(q, m, e)).unwrap_or_default();
        self.cards
            .iter()
            .map(|(name, luid, total)| Gpu {
                name: name.clone(),
                used: memory.iter().find(|(l, _)| l == luid).map(|(_, u)| *u),
                total: *total,
                busy: busy.iter().find(|(l, _)| l == luid).map(|(_, b)| *b),
            })
            .collect()
    }
}

/// An adapter's LUID (high, low), as DXGI and the counters' names give it.
#[cfg(any(windows, test))]
type Luid = (i32, u32);

/// Each adapter's video memory in use, and how busy it is.
#[cfg(windows)]
type Sample = (Vec<(Luid, u64)>, Vec<(Luid, u32)>);

/// "luid_0x00000000_0x0000F3D8_phys_0": an adapter's LUID (high, low).
#[cfg(any(windows, test))]
fn parse_luid(name: &str) -> Option<Luid> {
    let rest = name.strip_prefix("luid_0x")?;
    let (high, rest) = rest.split_once("_0x")?;
    let low = rest.split('_').next()?;
    Some((u32::from_str_radix(high, 16).ok()? as i32, u32::from_str_radix(low, 16).ok()?))
}

/// "pid_1234_luid_0x00000000_0x0000F3D8_phys_0_eng_0_engtype_3D": the engine's adapter and type.
#[cfg(any(windows, test))]
fn engine_of(name: &str) -> Option<(Luid, &str)> {
    let luid = parse_luid(&name[name.find("luid_")?..])?;
    let ty = &name[name.find("engtype_")? + "engtype_".len()..];
    Some((luid, ty))
}

/// Each adapter's use from the engines' counters, as Task Manager counts it: every program's
/// use of an engine type added up, and the busiest type taken.
#[cfg(any(windows, test))]
fn busy_by_adapter<'a>(engines: impl Iterator<Item = (&'a str, f64)>) -> Vec<(Luid, u32)> {
    let mut sums: Vec<((Luid, &str), f64)> = Vec::new();
    for (name, v) in engines {
        let Some(key) = engine_of(name) else { continue };
        match sums.iter_mut().find(|(k, _)| *k == key) {
            Some((_, s)) => *s += v,
            None => sums.push((key, v)),
        }
    }
    let mut out: Vec<(Luid, u32)> = Vec::new();
    for ((luid, _), s) in sums {
        let p = s.round().clamp(0.0, 100.0) as u32;
        match out.iter_mut().find(|(l, _)| *l == luid) {
            Some((_, b)) => *b = (*b).max(p),
            None => out.push((luid, p)),
        }
    }
    out
}

/// Video memory in use on each adapter, by every program, and each adapter's use (about half a
/// millisecond with a few hundred engines).
#[cfg(windows)]
fn sample(query: usize, memory: usize, engines: Option<usize>) -> Sample {
    use windows::Win32::System::Performance::*;
    // SAFETY: handles PDH gave Source::open, which stay open.
    if unsafe { PdhCollectQueryData(PDH_HQUERY(query as *mut _)) } != 0 {
        return Default::default();
    }
    let mut used: Vec<(Luid, u64)> = Vec::new();
    for (name, v) in values(memory) {
        let Some(luid) = parse_luid(&name) else { continue };
        let v = v.max(0.0) as u64;
        match used.iter_mut().find(|(l, _)| *l == luid) {
            Some((_, sum)) => *sum += v,
            None => used.push((luid, v)),
        }
    }
    let engines = engines.map(values).unwrap_or_default();
    (used, busy_by_adapter(engines.iter().map(|(n, v)| (n.as_str(), *v))))
}

/// A counter's value for each of its instances, by name (none when the counter has no value
/// yet, as a rate before its second collection).
#[cfg(windows)]
fn values(counter: usize) -> Vec<(String, f64)> {
    use windows::Win32::System::Performance::*;
    let counter = PDH_HCOUNTER(counter as *mut _);
    let mut out = Vec::new();
    // SAFETY: the values are read into a buffer of the size PDH asks for (u64s, so its items,
    // which hold pointers and 64-bit values, are aligned); each item's name points into that
    // buffer and is read while it lives.
    unsafe {
        let (mut size, mut count) = (0u32, 0u32);
        if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None) != PDH_MORE_DATA || size == 0 {
            return out;
        }
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
            return out;
        }
        for i in 0..count as usize {
            let item = &*items.add(i);
            if let Ok(name) = item.szName.to_string() {
                out.push((name, item.FmtValue.Anonymous.doubleValue));
            }
        }
    }
    out
}

/// The frame rate of the program in front, from the frames Windows sees programs present:
/// DXGI's and Direct3D 9's present events (games made with Direct3D 9 to 12; Vulkan and OpenGL
/// present otherwise), through an ETW session of Sofa's own that runs while the overlay is up.
/// Starting one needs the Performance Log Users group or an administrator, as Game Bar's frame
/// rate does.
#[cfg(windows)]
pub mod frames {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SUCCESS};
    use windows::Win32::System::Diagnostics::Etw::*;
    use windows::core::{GUID, HSTRING, PCWSTR, PWSTR};

    const NAME: &str = "Sofa frame rate";
    /// Microsoft-Windows-DXGI: Present_Start (42) and PresentMultiplaneOverlay_Start (55).
    const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
    /// Microsoft-Windows-D3D9: Present_Start (1).
    const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
    /// The keyword both providers' present events carry.
    const PRESENTS_KEYWORD: u64 = 0x8000_0000_0000_0002;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum State {
        Off,
        On,
        /// Windows refused: not an administrator, not in Performance Log Users.
        NotAllowed,
        Failed,
    }

    static STATE: Mutex<State> = Mutex::new(State::Off);
    /// The session while it runs.
    static SESSION: Mutex<Option<u64>> = Mutex::new(None);
    /// Each process's presents in the last few seconds, as performance-counter ticks.
    static PRESENTS: Mutex<Option<HashMap<u32, VecDeque<i64>>>> = Mutex::new(None);

    pub fn state() -> State {
        STATE.lock().map_or(State::Failed, |s| *s)
    }

    fn set(s: State) {
        if let Ok(mut st) = STATE.lock() {
            *st = s;
        }
    }

    fn ticks() -> (i64, i64) {
        use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
        let (mut now, mut freq) = (0i64, 0i64);
        // SAFETY: two locals written by the calls.
        unsafe {
            let _ = QueryPerformanceCounter(&mut now);
            let _ = QueryPerformanceFrequency(&mut freq);
        }
        (now, freq.max(1))
    }

    /// EVENT_TRACE_PROPERTIES followed by room for the session's name, in u64s so it is aligned.
    fn properties() -> Vec<u64> {
        let head = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
        let size = head + (NAME.len() + 1) * 2;
        let mut buf = vec![0u64; size.div_ceil(8)];
        let p = buf.as_mut_ptr().cast::<EVENT_TRACE_PROPERTIES>();
        // SAFETY: the buffer is at least as large as the struct and aligned for it.
        unsafe {
            (*p).Wnode.BufferSize = size as u32;
            (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            // Time stamps from the performance counter, as `ticks` reads it.
            (*p).Wnode.ClientContext = 1;
            (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
            // Events reach Sofa at least once a second.
            (*p).FlushTimer = 1;
            (*p).LoggerNameOffset = head as u32;
        }
        buf
    }

    /// Whether Windows lets this account start the session: Performance Log Users, or Sofa
    /// running as an administrator (read once; it changes only at the next sign-in).
    pub fn allowed() -> bool {
        use std::sync::OnceLock;
        use windows::Win32::Security::{AllocateAndInitializeSid, CheckTokenMembership, FreeSid, PSID, SID_IDENTIFIER_AUTHORITY};
        static ALLOWED: OnceLock<bool> = OnceLock::new();
        *ALLOWED.get_or_init(|| {
            // S-1-5-32-559 (Performance Log Users) and S-1-5-32-544 (Administrators, only when
            // enabled: an elevated Sofa).
            let nt = SID_IDENTIFIER_AUTHORITY { Value: [0, 0, 0, 0, 0, 5] };
            [559u32, 544].into_iter().any(|rid| {
                let mut sid = PSID::default();
                let mut member = windows::core::BOOL(0);
                // SAFETY: a SID Windows allocates and we free; membership written into a local.
                unsafe {
                    if AllocateAndInitializeSid(&nt, 2, 32, rid, 0, 0, 0, 0, 0, 0, &mut sid).is_err() {
                        return false;
                    }
                    let ok = CheckTokenMembership(None, sid, &mut member).is_ok();
                    let _ = FreeSid(sid);
                    ok && member.as_bool()
                }
            })
        })
    }

    /// Starts or stops watching (only while the overlay is up). Stopping also ends a session of
    /// the same name that a Sofa which was ended (by an update, say) left behind.
    pub fn watch(on: bool) {
        if on {
            if state() == State::Off {
                start();
            }
            return;
        }
        let session = SESSION.lock().ok().and_then(|mut s| s.take());
        let mut props = properties();
        // SAFETY: stops our own session (its consumer thread then returns), or the one left
        // behind, by name.
        unsafe {
            let _ = match session {
                Some(s) => ControlTraceW(CONTROLTRACE_HANDLE { Value: s }, PCWSTR::null(), props.as_mut_ptr().cast(), EVENT_TRACE_CONTROL_STOP),
                None => ControlTraceW(CONTROLTRACE_HANDLE::default(), &HSTRING::from(NAME), props.as_mut_ptr().cast(), EVENT_TRACE_CONTROL_STOP),
            };
        }
        if let Ok(mut p) = PRESENTS.lock() {
            *p = None;
        }
        if state() == State::On {
            set(State::Off);
        }
    }

    fn start() {
        let name = HSTRING::from(NAME);
        let mut handle = CONTROLTRACE_HANDLE::default();
        let mut props = properties();
        // SAFETY: the properties buffer has room for the name; a session of the same name left
        // by a Sofa that crashed is stopped first.
        let mut err = unsafe { StartTraceW(&mut handle, &name, props.as_mut_ptr().cast()) };
        if err == ERROR_ALREADY_EXISTS {
            let mut stop = properties();
            // SAFETY: as above.
            unsafe {
                let _ = ControlTraceW(CONTROLTRACE_HANDLE::default(), &name, stop.as_mut_ptr().cast(), EVENT_TRACE_CONTROL_STOP);
                props = properties();
                err = StartTraceW(&mut handle, &name, props.as_mut_ptr().cast());
            }
        }
        if err != ERROR_SUCCESS {
            crate::log(&format!("frame rate: no ETW session ({})", err.0));
            set(if err == ERROR_ACCESS_DENIED { State::NotAllowed } else { State::Failed });
            return;
        }
        for provider in [DXGI, D3D9] {
            // SAFETY: enables a provider in our session.
            unsafe {
                let _ = EnableTraceEx2(handle, &provider, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, 4, PRESENTS_KEYWORD, 0, 0, None);
            }
        }
        if let Ok(mut s) = SESSION.lock() {
            *s = Some(handle.Value);
        }
        set(State::On);
        let spawned = std::thread::Builder::new().name("frames".into()).spawn(|| {
            let mut name: Vec<u16> = NAME.encode_utf16().chain([0]).collect();
            let mut log = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(name.as_mut_ptr()), ..Default::default() };
            log.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
            log.Anonymous2.EventRecordCallback = Some(on_event);
            // SAFETY: `log` and the name it points at live until ProcessTrace returns, which it
            // does when the session stops.
            unsafe {
                let trace = OpenTraceW(&mut log);
                if trace.Value != u64::MAX {
                    let _ = ProcessTrace(&[trace], None, None);
                    let _ = CloseTrace(trace);
                }
            }
        });
        if spawned.is_err() {
            watch(false);
            set(State::Failed);
        }
    }

    /// One event from the session: a present is noted for its process.
    unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
        // SAFETY: ETW hands the callback a valid record for the duration of the call.
        let r = unsafe { &*record };
        let h = &r.EventHeader;
        let id = h.EventDescriptor.Id;
        let present = if h.ProviderId == DXGI && (id == 42 || id == 55) {
            // The swap chain's address, then the flags: DXGI_PRESENT_TEST (1) shows no frame.
            let ptr = if h.Flags & 0x20 != 0 { 4 } else { 8 };
            let flags = if usize::from(r.UserDataLength) >= ptr + 4 && !r.UserData.is_null() {
                // SAFETY: inside the event's data, as its length says.
                unsafe { std::ptr::read_unaligned(r.UserData.cast::<u8>().add(ptr).cast::<u32>()) }
            } else {
                0
            };
            flags & 1 == 0
        } else {
            h.ProviderId == D3D9 && id == 1
        };
        if !present {
            return;
        }
        let (_, freq) = ticks();
        let Ok(mut map) = PRESENTS.lock() else { return };
        let map = map.get_or_insert_with(HashMap::new);
        let times = map.entry(h.ProcessId).or_default();
        times.push_back(h.TimeStamp);
        while times.front().is_some_and(|&t| h.TimeStamp - t > 3 * freq) {
            times.pop_front();
        }
        // Processes that stopped presenting a while ago are forgotten.
        if map.len() > 64 {
            map.retain(|_, t| t.back().is_some_and(|&last| h.TimeStamp - last < 10 * freq));
        }
    }

    /// `pid`'s frame rate over the last two seconds of its frames that arrived (they arrive in
    /// batches up to two seconds late). None when nothing arrived for four seconds.
    pub fn fps(pid: u32) -> Option<u32> {
        let (now, freq) = ticks();
        let map = PRESENTS.lock().ok()?;
        let times = map.as_ref()?.get(&pid)?;
        let last = *times.back()?;
        if now - last > 4 * freq {
            return None;
        }
        Some(rate(times.iter().copied().filter(|&t| last - t <= 2 * freq), freq))
    }

    /// Frames per second from present times (in order, `freq` ticks a second).
    pub(super) fn rate(times: impl Iterator<Item = i64>, freq: i64) -> u32 {
        let (mut first, mut last, mut n) = (None, 0i64, 0u32);
        for t in times {
            first.get_or_insert(t);
            last = t;
            n += 1;
        }
        match first {
            Some(first) if n > 1 && last > first => ((n - 1) as f64 * freq as f64 / (last - first) as f64).round() as u32,
            _ => 0,
        }
    }

    /// Asks Windows (its administrator prompt) to add this account to Performance Log Users, as
    /// Game Bar does for its frame rate; Windows counts it from the next sign-in. False when
    /// the prompt could not be shown or was declined.
    pub fn allow() -> bool {
        use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
        use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
        use windows::core::w;
        // This account's SID: the prompt may run as another (an administrator's) account.
        // SAFETY: our own token, read into a buffer of the size Windows asks for (u64s, so the
        // TOKEN_USER in it is aligned); the SID string Windows allocates is freed after copying.
        let sid = unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut len = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            let read = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len).is_ok();
            let _ = CloseHandle(token);
            if !read {
                return false;
            }
            let user = &*buf.as_ptr().cast::<TOKEN_USER>();
            let mut s = PWSTR::null();
            if ConvertSidToStringSidW(user.User.Sid, &mut s).is_err() {
                return false;
            }
            let sid = s.to_string().unwrap_or_default();
            let _ = LocalFree(Some(HLOCAL(s.0.cast())));
            sid
        };
        if !sid.starts_with("S-1-") || !sid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return false;
        }
        // S-1-5-32-559: Performance Log Users, whatever Windows' language calls it.
        let args = HSTRING::from(format!("-NoProfile -WindowStyle Hidden -Command Add-LocalGroupMember -SID S-1-5-32-559 -Member {sid}"));
        // SAFETY: Windows' own prompt; the strings live for the call.
        let r = unsafe { ShellExecuteW(None, w!("runas"), w!("powershell.exe"), &args, None, SW_HIDE) };
        r.0 as isize > 32
    }

    /// Each process's presents: how many, and the first's and last's age in seconds (the
    /// opt-in test).
    #[cfg(test)]
    pub fn dump() -> Vec<(u32, usize, f64, f64)> {
        let (now, freq) = ticks();
        let Ok(map) = PRESENTS.lock() else { return Vec::new() };
        map.as_ref().map_or_else(Vec::new, |m| m.iter().map(|(p, t)| (*p, t.len(), (now - t.front().copied().unwrap_or(now)) as f64 / freq as f64, (now - t.back().copied().unwrap_or(now)) as f64 / freq as f64)).collect())
    }

    /// The process presenting the most frames, and its frame rate (the opt-in test).
    #[cfg(test)]
    pub fn busiest() -> Option<(u32, u32)> {
        let pids: Vec<u32> = PRESENTS.lock().ok()?.as_ref()?.keys().copied().collect();
        pids.into_iter().filter_map(|p| Some((p, fps(p)?))).max_by_key(|(_, f)| *f)
    }

    /// The frame rate of the program in front (not Sofa's own).
    pub fn in_front() -> Option<u32> {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        if state() != State::On {
            return None;
        }
        let mut pid = 0u32;
        // SAFETY: reads the foreground window's process id into a local.
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_invalid() {
                return None;
            }
            GetWindowThreadProcessId(fg, Some(&mut pid));
        }
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        fps(pid)
    }
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
fn cpu_times() -> Option<(u64, u64)> {
    stat_times(&std::fs::read_to_string("/proc/stat").ok()?)
}

/// Busy and total time from `/proc/stat`'s first line (user, nice, system, idle, iowait, irq,
/// softirq, steal; idle and iowait are not busy).
#[cfg(any(target_os = "linux", test))]
fn stat_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().next()?.strip_prefix("cpu ")?;
    let v: Vec<u64> = line.split_whitespace().take(8).filter_map(|n| n.parse().ok()).collect();
    if v.len() < 5 {
        return None;
    }
    let all: u64 = v.iter().sum();
    Some((all.saturating_sub(v[3] + v[4]), all))
}

/// NVIDIA's cards through NVML (started once), AMD's through amdgpu's files.
#[cfg(target_os = "linux")]
struct Source;

#[cfg(target_os = "linux")]
impl Source {
    fn open() -> Source {
        nvml::start();
        Source
    }

    fn read(&self) -> Vec<Gpu> {
        cards()
    }
}

#[cfg(target_os = "linux")]
fn cards() -> Vec<Gpu> {
    let mut out = nvml::gpus();
    // amdgpu reports its video memory and use in sysfs.
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
            let busy = read("gpu_busy_percent").map(|b| b.min(100) as u32);
            out.push(Gpu { name: tidy(&name), used, total, busy });
        }
    }
    out
}

/// NVIDIA's management library, loaded when there is one (the proprietary driver ships it).
#[cfg(target_os = "linux")]
mod nvml {
    use super::{Gpu, tidy};
    use std::ffi::{c_char, c_int, c_uint, c_void};
    use std::sync::OnceLock;

    #[repr(C)]
    struct Memory {
        total: u64,
        free: u64,
        used: u64,
    }

    #[repr(C)]
    struct Utilization {
        gpu: c_uint,
        memory: c_uint,
    }

    type Count = unsafe extern "C" fn(*mut c_uint) -> c_int;
    type Handle = unsafe extern "C" fn(c_uint, *mut *mut c_void) -> c_int;
    type MemInfo = unsafe extern "C" fn(*mut c_void, *mut Memory) -> c_int;
    type Name = unsafe extern "C" fn(*mut c_void, *mut c_char, c_uint) -> c_int;
    type Use = unsafe extern "C" fn(*mut c_void, *mut Utilization) -> c_int;

    struct Lib {
        count: Count,
        handle: Handle,
        memory: MemInfo,
        name: Name,
        busy: Option<Use>,
    }

    // SAFETY: plain function pointers into a library that stays loaded.
    unsafe impl Send for Lib {}
    // SAFETY: as above; NVML's calls are thread-safe.
    unsafe impl Sync for Lib {}

    /// Loads and starts NVML (once; it takes about a tenth of a second).
    pub fn start() {
        lib();
    }

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
                    busy: sym(c"nvmlDeviceGetUtilizationRates").map(|p| std::mem::transmute::<*mut c_void, Use>(p)),
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
                    tidy(&std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy())
                } else {
                    "NVIDIA".into()
                };
                let mut u = Utilization { gpu: 0, memory: 0 };
                let busy = l.busy.and_then(|f| (f(dev, &mut u) == 0).then(|| u.gpu.min(100)));
                out.push(Gpu { name, used: Some(m.used), total: m.total, busy });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(gb(6_549_825_536), "6.1 GB");
        assert_eq!(of(3 << 30, 16 << 30), "3.0 of 16.0 GB");
        assert_eq!(percent(1, 4), 25);
        assert_eq!(percent(1, 0), 0);
    }

    #[test]
    fn card_names_are_tidied() {
        assert_eq!(tidy(" AMD  Radeon RX 7800 XT "), "AMD Radeon RX 7800 XT");
    }

    #[test]
    fn adapter_luids_are_read_from_counter_names() {
        assert_eq!(parse_luid("luid_0x00000000_0x0000F3D8_phys_0"), Some((0, 0xF3D8)));
        assert_eq!(parse_luid("something else"), None);
        assert_eq!(engine_of("pid_1234_luid_0x00000000_0x0000F3D8_phys_0_eng_0_engtype_3D"), Some(((0, 0xF3D8), "3D")));
    }

    #[test]
    fn a_card_is_as_busy_as_its_busiest_engine_type() {
        let a = "luid_0x00000000_0x0000F3D8_phys_0";
        let names = [format!("pid_1_{a}_eng_0_engtype_3D"), format!("pid_2_{a}_eng_0_engtype_3D"), format!("pid_2_{a}_eng_3_engtype_Copy"), "pid_9_luid_0x00000000_0x00000001_phys_0_eng_0_engtype_3D".to_owned()];
        let values = [40.0, 35.0, 90.0, 120.0];
        let busy = busy_by_adapter(names.iter().map(|n| n.as_str()).zip(values));
        assert_eq!(busy, vec![((0, 0xF3D8), 90), ((0, 1), 100)]);
    }

    #[test]
    fn the_main_card_has_the_most_video_memory() {
        let card = |name: &str, total| Gpu { name: name.into(), used: None, total, busy: None };
        let cards = [card("Intel UHD", 128 << 20), card("Radeon", 16 << 30)];
        assert_eq!(main_gpu(&cards).map(|g| g.name.as_str()), Some("Radeon"));
    }

    #[test]
    fn cpu_time_is_read_from_proc_stat() {
        let stat = "cpu  100 5 50 800 20 3 2 0 0 0\ncpu0 1 2 3 4\n";
        assert_eq!(stat_times(stat), Some((160, 980)));
        assert_eq!(stat_times("intr 1 2 3"), None);
    }

    #[test]
    fn this_computer_has_memory() {
        let u = read(false);
        assert!(u.ram_total > 0 && u.ram_used <= u.ram_total);
        assert_eq!(u.gpus, None, "video memory only when asked");
        // CPU use needs two reads a clock tick apart (other tests read too).
        let start = std::time::Instant::now();
        let mut cpu = None;
        while cpu.is_none() && start.elapsed().as_secs() < 2 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            cpu = read(false).cpu;
        }
        assert!(cpu.is_some_and(|c| c <= 100), "a later read has CPU use: {cpu:?}");
    }

    #[cfg(windows)]
    #[test]
    fn frame_rate_is_frames_over_their_time() {
        // 61 frames 1/60 s apart over a second, in ticks of a 10 MHz counter.
        let freq = 10_000_000;
        assert_eq!(frames::rate((0..61).map(|i| i * freq / 60), freq), 60);
        assert_eq!(frames::rate([5].into_iter(), freq), 0, "one frame is no rate");
        assert_eq!(frames::rate(std::iter::empty(), freq), 0);
    }

    /// Opt-in (`cargo test --bin sofa frames_are_counted -- --ignored`): starts the ETW session
    /// and needs a program presenting frames meanwhile (a browser, a game, Wallpaper Engine).
    #[cfg(windows)]
    #[test]
    #[ignore = "starts an ETW session; needs Performance Log Users and a program presenting frames"]
    fn frames_are_counted() {
        frames::watch(true);
        assert_eq!(frames::state(), frames::State::On, "the session starts");
        std::thread::sleep(std::time::Duration::from_secs(3));
        eprintln!("presents (pid, count, first s ago, last s ago): {:?}", frames::dump());
        let busiest = frames::busiest();
        frames::watch(false);
        assert_eq!(frames::state(), frames::State::Off);
        eprintln!("busiest: {busiest:?}");
        assert!(busiest.is_some_and(|(_, fps)| fps > 0 && fps < 1000), "a program presented frames: {busiest:?}");
    }

    #[test]
    fn video_memory_opens_off_the_ui_thread() {
        let start = std::time::Instant::now();
        let first = read(true);
        assert!(start.elapsed().as_millis() < 50, "the first read waits for nothing: {:?}", start.elapsed());
        if first.gpus.is_some() {
            return;
        }
        while read(true).gpus.is_none() {
            assert!(start.elapsed().as_secs() < 10, "video memory never opened");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
