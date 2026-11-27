//! Sound on Linux, for the quick menu and the DualSense's mute button: the outputs and which
//! one is the default, the output volume, and the microphone's mute. Through `pactl`, which
//! speaks to PulseAudio and to PipeWire's pulse server alike (Ubuntu, Fedora, Arch with
//! pipewire-pulse); `wpctl` covers volume and mute where only WirePlumber is there. The same API
//! as the Windows `audio.rs`.
//!
//! Every call runs on the UI thread and takes a few milliseconds. In a QA run `ui.rs` never
//! calls the functions that change anything.

use std::process::Command;

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
    /// The sink's name (`alsa_output.pci-0000_01_00.1.hdmi-stereo`).
    pub id: String,
    /// "LG TV", "Headphones".
    pub name: String,
    /// The card ("GA102 High Definition Audio Controller"), to tell two outputs apart.
    pub detail: String,
    pub kind: Kind,
}

fn run(prog: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(prog).env("LC_ALL", "C").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn pactl(args: &[&str]) -> Option<String> {
    run("pactl", args)
}

/// One sink as `pactl list sinks` describes it.
#[derive(Default, Debug)]
struct Sink {
    name: String,
    description: String,
    active_port: String,
    port_names: Vec<(String, String)>,
    props: Vec<(String, String)>,
}

impl Sink {
    fn prop(&self, k: &str) -> &str {
        self.props.iter().find(|(key, _)| key == k).map_or("", |(_, v)| v.as_str())
    }
}

fn parse_sinks(text: &str) -> Vec<Sink> {
    let mut out: Vec<Sink> = Vec::new();
    let mut section = "";
    for line in text.lines() {
        if line.starts_with("Sink #") {
            out.push(Sink::default());
            section = "";
            continue;
        }
        let Some(s) = out.last_mut() else { continue };
        let depth = line.len() - line.trim_start_matches('\t').len();
        let t = line.trim();
        if depth == 1 {
            section = "";
            if let Some(v) = t.strip_prefix("Name: ") {
                s.name = v.to_owned();
            } else if let Some(v) = t.strip_prefix("Description: ") {
                s.description = v.to_owned();
            } else if let Some(v) = t.strip_prefix("Active Port: ") {
                s.active_port = v.to_owned();
            } else if t == "Properties:" {
                section = "props";
            } else if t == "Ports:" {
                section = "ports";
            }
        } else if depth >= 2 {
            match section {
                "props" => {
                    if let Some((k, v)) = t.split_once(" = ") {
                        s.props.push((k.trim().to_owned(), v.trim().trim_matches('"').to_owned()));
                    }
                }
                "ports" if depth == 2 => {
                    if let Some((k, v)) = t.split_once(": ") {
                        let desc = v.split(" (type:").next().unwrap_or(v).split(" (priority").next().unwrap_or(v);
                        s.port_names.push((k.to_owned(), desc.trim().to_owned()));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn kind_of(s: &Sink) -> Kind {
    let all = format!("{} {} {} {}", s.name, s.description, s.active_port, s.prop("device.product.name")).to_lowercase();
    if all.contains("dualsense") || all.contains("wireless controller") {
        return Kind::Controller;
    }
    let ff = s.prop("device.form_factor");
    if matches!(ff, "headphone" | "headset" | "handset" | "hands-free") || all.contains("headphone") || all.contains("headset") || s.name.starts_with("bluez_output") {
        return Kind::Headphones;
    }
    if all.contains("hdmi") || all.contains("displayport") || ff == "tv" {
        return Kind::Tv;
    }
    Kind::Speakers
}

/// The TV's own name, which the HDMI port carries from the screen ("HDMI / DisplayPort (LG TV)").
fn screen_name(s: &Sink) -> Option<String> {
    let desc = &s.port_names.iter().find(|(k, _)| *k == s.active_port)?.1;
    let inner = desc.rsplit_once(" (")?.1.strip_suffix(')')?;
    (!inner.is_empty()).then(|| inner.to_owned())
}

fn output_of(s: &Sink) -> Output {
    let kind = kind_of(s);
    let card = s.prop("alsa.card_name");
    let card = if card.is_empty() { s.prop("device.description") } else { card };
    let name = match kind {
        Kind::Tv => screen_name(s).unwrap_or_else(|| "TV".into()),
        _ => s.description.clone(),
    };
    Output { id: s.name.clone(), name, detail: card.to_owned(), kind }
}

/// The outputs, in the sound server's order, and the default one's id.
pub fn outputs() -> (Vec<Output>, Option<String>) {
    let mut list: Vec<Output> = pactl(&["list", "sinks"]).map(|t| parse_sinks(&t).iter().map(output_of).collect()).unwrap_or_default();
    // Two outputs with the same name keep their card's name.
    let names: Vec<String> = list.iter().map(|o| o.name.to_lowercase()).collect();
    for o in &mut list {
        if names.iter().filter(|n| **n == o.name.to_lowercase()).count() > 1 && !o.detail.is_empty() {
            o.name = format!("{} ({})", o.name, o.detail);
        }
    }
    let default = pactl(&["get-default-sink"]).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    (list, default)
}

/// Makes `id` the default output, and moves what is playing to it.
pub fn set_output(id: &str) -> bool {
    if pactl(&["set-default-sink", id]).is_none() {
        return false;
    }
    if let Some(inputs) = pactl(&["list", "short", "sink-inputs"]) {
        for n in inputs.lines().filter_map(|l| l.split_whitespace().next()) {
            let _ = pactl(&["move-sink-input", n, id]);
        }
    }
    true
}

/// The first percentage in `pactl`'s volume line, as 0 to 1.
fn percent(text: &str) -> Option<f32> {
    let at = text.find('%')?;
    let digits: String = text[..at].chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse::<f32>().ok().map(|p| p / 100.0)
}

/// The default output's volume (0 to 1) and whether it is muted.
pub fn volume() -> Option<(f32, bool)> {
    if let (Some(v), Some(m)) = (pactl(&["get-sink-volume", "@DEFAULT_SINK@"]), pactl(&["get-sink-mute", "@DEFAULT_SINK@"])) {
        return Some((percent(&v)?, m.contains("yes")));
    }
    // WirePlumber alone: "Volume: 0.40 [MUTED]".
    let w = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
    let level = w.split_whitespace().nth(1)?.parse::<f32>().ok()?;
    Some((level, w.contains("MUTED")))
}

pub fn set_volume(level: f32) -> bool {
    let pct = format!("{}%", (level.clamp(0.0, 1.0) * 100.0).round() as u32);
    if pactl(&["set-sink-mute", "@DEFAULT_SINK@", "0"]).is_some() && pactl(&["set-sink-volume", "@DEFAULT_SINK@", &pct]).is_some() {
        return true;
    }
    let v = format!("{:.2}", level.clamp(0.0, 1.0));
    run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]).is_some() && run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &v]).is_some()
}

pub fn set_muted(muted: bool) -> bool {
    let m = if muted { "1" } else { "0" };
    pactl(&["set-sink-mute", "@DEFAULT_SINK@", m]).is_some() || run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", m]).is_some()
}

/// Whether the default microphone is muted (None: there is no microphone).
pub fn mic_muted() -> Option<bool> {
    if let Some(src) = pactl(&["get-default-source"]) {
        let src = src.trim();
        // With no microphone the default source is an output's monitor.
        if src.is_empty() || src.ends_with(".monitor") {
            return None;
        }
        return pactl(&["get-source-mute", "@DEFAULT_SOURCE@"]).map(|m| m.contains("yes"));
    }
    run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).map(|w| w.contains("MUTED"))
}

pub fn set_mic_muted(muted: bool) -> bool {
    let m = if muted { "1" } else { "0" };
    pactl(&["set-source-mute", "@DEFAULT_SOURCE@", m]).is_some() || run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SOURCE@", m]).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINKS: &str = "Sink #49\n\tState: SUSPENDED\n\tName: alsa_output.pci-0000_01_00.1.hdmi-stereo\n\tDescription: GA102 High Definition Audio Controller Digital Stereo (HDMI)\n\tMute: no\n\tProperties:\n\t\talsa.card_name = \"HDA NVidia\"\n\t\tdevice.bus = \"pci\"\n\tPorts:\n\t\thdmi-output-0: HDMI / DisplayPort (LG TV SSCR2) (type: HDMI, priority: 5900, available)\n\tActive Port: hdmi-output-0\nSink #50\n\tName: bluez_output.AA_BB.1\n\tDescription: WH-1000XM4\n\tProperties:\n\t\tdevice.form_factor = \"headphone\"\n";

    #[test]
    fn sinks_become_outputs() {
        let s = parse_sinks(SINKS);
        assert_eq!(s.len(), 2);
        let tv = output_of(&s[0]);
        assert_eq!((tv.name.as_str(), tv.kind, tv.detail.as_str()), ("LG TV SSCR2", Kind::Tv, "HDA NVidia"));
        let hp = output_of(&s[1]);
        assert_eq!((hp.name.as_str(), hp.kind), ("WH-1000XM4", Kind::Headphones));
    }

    #[test]
    fn volume_lines_are_read() {
        assert_eq!(percent("Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50%"), Some(0.5));
        assert_eq!(percent("nothing"), None);
    }
}
