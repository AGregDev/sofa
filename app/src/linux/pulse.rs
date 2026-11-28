//! Interface sounds on Linux, through PulseAudio's simple API (`libpulse-simple.so.0`; PipeWire's
//! pulse server answers it too), loaded at run time so Sofa needs no audio library to start.
//! One `sound` thread plays them: a new sound cuts the one playing short (as Windows' PlaySound
//! does), and the stream closes two seconds after the last one, so Sofa holds no audio stream
//! while nothing sounds.

use std::ffi::{c_char, c_int, c_void};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

const PA_SAMPLE_S16LE: c_int = 3;
const PA_STREAM_PLAYBACK: c_int = 1;

type NewFn = unsafe extern "C" fn(*const c_char, *const c_char, c_int, *const c_char, *const c_char, *const SampleSpec, *const c_void, *const BufferAttr, *mut c_int) -> *mut c_void;
type WriteFn = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int;
type FlushFn = unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int;
type FreeFn = unsafe extern "C" fn(*mut c_void);

struct Lib {
    new: NewFn,
    write: WriteFn,
    flush: FlushFn,
    drain: FlushFn,
    free: FreeFn,
}

impl Lib {
    fn load() -> Option<Lib> {
        // SAFETY: dlopen and dlsym with NUL-terminated names; each symbol is cast to the type
        // PulseAudio's simple.h declares for it. The library stays loaded for the process.
        unsafe {
            let h = libc::dlopen(c"libpulse-simple.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if h.is_null() {
                return None;
            }
            let sym = |name: &std::ffi::CStr| {
                let p = libc::dlsym(h, name.as_ptr());
                (!p.is_null()).then_some(p)
            };
            Some(Lib {
                new: std::mem::transmute::<*mut c_void, NewFn>(sym(c"pa_simple_new")?),
                write: std::mem::transmute::<*mut c_void, WriteFn>(sym(c"pa_simple_write")?),
                flush: std::mem::transmute::<*mut c_void, FlushFn>(sym(c"pa_simple_flush")?),
                drain: std::mem::transmute::<*mut c_void, FlushFn>(sym(c"pa_simple_drain")?),
                free: std::mem::transmute::<*mut c_void, FreeFn>(sym(c"pa_simple_free")?),
            })
        }
    }
}

/// A sound: 16-bit mono PCM and its rate.
type Req = (&'static [u8], u32);

static TX: OnceLock<Mutex<Sender<Req>>> = OnceLock::new();

/// Plays 16-bit mono PCM at `rate` (the sound thread starts with the first one).
pub fn play(pcm: &'static [u8], rate: u32) {
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel();
        let _ = std::thread::Builder::new().name("sound".into()).spawn(move || sound_thread(rx));
        Mutex::new(tx)
    });
    if let Ok(tx) = tx.lock() {
        let _ = tx.send((pcm, rate));
    }
}

fn sound_thread(rx: Receiver<Req>) {
    let Some(lib) = Lib::load() else {
        // No PulseAudio or PipeWire: sounds are dropped.
        for _ in rx {}
        return;
    };
    let mut stream: *mut c_void = std::ptr::null_mut();
    let mut rate = 0u32;
    let mut err: c_int = 0;
    loop {
        let next = if stream.is_null() {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(Duration::from_secs(2))
        };
        let (mut pcm, r) = match next {
            Ok(req) => req,
            Err(RecvTimeoutError::Timeout) => {
                // SAFETY: our own stream, drained and freed once.
                unsafe {
                    (lib.drain)(stream, &mut err);
                    (lib.free)(stream);
                }
                stream = std::ptr::null_mut();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if !stream.is_null() && r != rate {
            // SAFETY: our own stream, freed once.
            unsafe { (lib.free)(stream) };
            stream = std::ptr::null_mut();
        }
        if stream.is_null() {
            rate = r;
            let spec = SampleSpec { format: PA_SAMPLE_S16LE, rate, channels: 1 };
            // A short buffer (20 ms) keeps the sound close to the press.
            let attr = BufferAttr { maxlength: u32::MAX, tlength: rate * 2 / 50, prebuf: u32::MAX, minreq: u32::MAX, fragsize: u32::MAX };
            // SAFETY: every pointer is to a local or a static C string, valid for the call.
            stream = unsafe { (lib.new)(std::ptr::null(), c"Sofa".as_ptr(), PA_STREAM_PLAYBACK, std::ptr::null(), c"Interface sounds".as_ptr(), &spec, std::ptr::null(), &attr, &mut err) };
            if stream.is_null() {
                continue;
            }
        } else {
            // SAFETY: our own open stream.
            unsafe { (lib.flush)(stream, &mut err) };
        }
        // Written in 10 ms pieces, so a newer sound can cut in.
        let piece = (rate as usize / 100) * 2;
        while !pcm.is_empty() {
            let n = pcm.len().min(piece.max(2));
            // SAFETY: `pcm` holds at least `n` bytes; the stream is open.
            if unsafe { (lib.write)(stream, pcm.as_ptr().cast(), n, &mut err) } < 0 {
                break;
            }
            pcm = &pcm[n..];
            if let Ok((p, r)) = rx.try_recv() {
                if r != rate {
                    break;
                }
                // SAFETY: our own open stream.
                unsafe { (lib.flush)(stream, &mut err) };
                pcm = p;
            }
        }
    }
    if !stream.is_null() {
        // SAFETY: our own stream, freed once.
        unsafe { (lib.free)(stream) };
    }
}
