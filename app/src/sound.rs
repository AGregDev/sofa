//! Interface sounds: short, quiet tones synthesized once into in-memory WAV files and played
//! asynchronously by Windows (no files, no audio thread of Sofa's own). On Linux a `sound`
//! thread plays them through PulseAudio's simple API (PipeWire answers it too), loaded at run
//! time, keeping its stream open only while sounds play.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
#[cfg(windows)]
use windows::core::PCWSTR;

pub static ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy)]
pub enum Sound {
    Move,
    Select,
    Back,
    Open,
    Launch,
    Edge,
    Notice,
}

const RATE: u32 = 44_100;

/// Tones: (start Hz, end Hz, start ms, length ms, volume).
fn synth(tones: &[(f32, f32, f32, f32, f32)]) -> Vec<u8> {
    let total_ms = tones.iter().map(|t| t.2 + t.3).fold(0.0, f32::max) + 10.0;
    let n = (RATE as f32 * total_ms / 1000.0) as usize;
    let mut samples = vec![0f32; n];
    for &(f0, f1, at, len, vol) in tones {
        let start = (RATE as f32 * at / 1000.0) as usize;
        let count = (RATE as f32 * len / 1000.0) as usize;
        let mut phase = 0f32;
        for i in 0..count {
            let p = i as f32 / count as f32;
            let f = f0 + (f1 - f0) * p;
            phase += f / RATE as f32 * std::f32::consts::TAU;
            // Soft attack, exponential release: no clicks.
            let env = (i as f32 / (RATE as f32 * 0.004)).min(1.0) * (-5.0 * p).exp();
            if let Some(s) = samples.get_mut(start + i) {
                *s += phase.sin() * env * vol;
            }
        }
    }
    let data: Vec<u8> = samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

static SOUNDS: LazyLock<[Vec<u8>; 7]> = LazyLock::new(|| {
    [
        synth(&[(1250.0, 1150.0, 0.0, 22.0, 0.07)]),
        synth(&[(880.0, 880.0, 0.0, 40.0, 0.09), (1320.0, 1320.0, 32.0, 50.0, 0.08)]),
        synth(&[(660.0, 440.0, 0.0, 70.0, 0.08)]),
        synth(&[(520.0, 780.0, 0.0, 80.0, 0.07)]),
        synth(&[(523.0, 523.0, 0.0, 90.0, 0.08), (659.0, 659.0, 70.0, 90.0, 0.08), (784.0, 784.0, 140.0, 160.0, 0.08)]),
        synth(&[(220.0, 180.0, 0.0, 35.0, 0.06)]),
        synth(&[(988.0, 988.0, 0.0, 80.0, 0.06), (1319.0, 1319.0, 70.0, 140.0, 0.06)]),
    ]
});

#[cfg(target_os = "linux")]
pub fn play(s: Sound) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    crate::linux::pulse::play(&SOUNDS[s as usize][44..], RATE);
}

#[cfg(windows)]
pub fn play(s: Sound) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let wav = &SOUNDS[s as usize];
    // SAFETY: the WAV bytes are static, so they outlive the asynchronous playback; with
    // SND_MEMORY the "name" pointer is the in-memory file.
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}
