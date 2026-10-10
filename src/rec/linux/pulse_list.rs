//! PulseAudio (and PipeWire's pulse server) sources, from `pactl`: the
//! sinks' monitors (system sound) and the microphones. ffmpeg records
//! them by name (`-f pulse -i <name>`).
//!
//! `pactl -f json list sources` (pactl 16+), else `pactl list short
//! sources` (tab-separated: index, name, driver, sample spec, state).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // unit-tested everywhere, used on Linux

use crate::update::{items, members, str_field};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The default sink's monitor (resolved by the pulse server).
pub const DEFAULT_MONITOR: &str = "@DEFAULT_MONITOR@";
/// The default source.
pub const DEFAULT_SOURCE: &str = "@DEFAULT_SOURCE@";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub description: String,
    /// A sink's monitor (what plays), not a microphone.
    pub monitor: bool,
}

fn is_monitor_name(name: &str) -> bool {
    name.ends_with(".monitor")
}

/// Sources in `pactl -f json list sources` output; `None` when it is not
/// a JSON array.
pub fn parse_json(s: &str) -> Option<Vec<Source>> {
    let mut out = Vec::new();
    for it in items(s.trim_start())? {
        let Some(m) = members(it) else { continue };
        let Some(name) = str_field(&m, "name").filter(|n| !n.is_empty()) else { continue };
        let description = str_field(&m, "description").filter(|d| !d.is_empty()).unwrap_or_else(|| name.clone());
        let of_sink = str_field(&m, "monitor_of_sink").is_some_and(|s| !s.is_empty() && s != "n/a");
        let class = m
            .iter()
            .find(|(k, _)| k == "properties")
            .and_then(|(_, v)| members(v))
            .and_then(|p| str_field(&p, "device.class"));
        let monitor = of_sink || class.as_deref() == Some("monitor") || is_monitor_name(&name);
        out.push(Source { name, description, monitor });
    }
    Some(out)
}

/// Sources in `pactl list short sources` output (the name is also the
/// description).
pub fn parse_short(s: &str) -> Vec<Source> {
    s.lines()
        .filter_map(|l| {
            let name = l.split('\t').nth(1)?.trim();
            (!name.is_empty()).then(|| Source { name: name.into(), description: name.into(), monitor: is_monitor_name(name) })
        })
        .collect()
}

fn pactl() -> Option<PathBuf> {
    super::ffmpeg::find_with("pactl", None, std::env::var_os("PATH").as_deref(), |p| p.is_file())
}

fn run(exe: &PathBuf, args: &[&str]) -> Option<String> {
    let o = Command::new(exe).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Every source, or `None` when `pactl` is missing or cannot reach a
/// server.
pub fn list() -> Option<Vec<Source>> {
    let exe = pactl()?;
    if let Some(v) = run(&exe, &["-f", "json", "list", "sources"]).and_then(|s| parse_json(&s)) {
        return Some(v);
    }
    run(&exe, &["list", "short", "sources"]).map(|s| parse_short(&s))
}

/// The sinks' monitors.
pub fn monitors() -> Vec<Source> {
    list().unwrap_or_default().into_iter().filter(|s| s.monitor).collect()
}

/// The microphones (sources that are not monitors).
pub fn sources() -> Vec<Source> {
    list().unwrap_or_default().into_iter().filter(|s| !s.monitor).collect()
}

/// The source for the system sound, out of `all` (`None`: pactl failed):
/// the default monitor, when there is any monitor.
pub fn system_source(all: Option<&[Source]>) -> Result<String, String> {
    match all {
        None => Err("no PulseAudio/PipeWire sound server (or pactl is not installed)".into()),
        Some(v) if v.iter().any(|s| s.monitor) => Ok(DEFAULT_MONITOR.into()),
        Some(_) => Err("no output device to record".into()),
    }
}

/// The source for microphone `id` ("" = the default), out of `all`.
pub fn mic_source(all: Option<&[Source]>, id: &str) -> Result<String, String> {
    let Some(v) = all else { return Err("no PulseAudio/PipeWire sound server (or pactl is not installed)".into()) };
    let mics: Vec<&Source> = v.iter().filter(|s| !s.monitor).collect();
    if mics.is_empty() {
        return Err("no microphone found".into());
    }
    if id.is_empty() {
        return Ok(DEFAULT_SOURCE.into());
    }
    mics.iter().find(|s| s.name == id).map(|s| s.name.clone()).ok_or_else(|| format!("microphone {id} is not connected"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"[{"index":0,"state":"SUSPENDED","name":"alsa_output.pci-0000_00_1f.3.analog-stereo.monitor","description":"Monitor of Built-in Audio Analog Stereo","driver":"PipeWire","sample_specification":"s32le 2ch 48000Hz","channel_map":"front-left,front-right","owner_module":4294967295,"mute":false,"volume":{"front-left":{"value":65536,"value_percent":"100%","db":"0.00 dB"}},"balance":0,"base_volume":{"value":65536,"value_percent":"100%","db":"0.00 dB"},"monitor_of_sink":"alsa_output.pci-0000_00_1f.3.analog-stereo","latency":{"actual":0,"configured":0},"flags":["HARDWARE","DECIBEL_VOLUME","LATENCY"],"properties":{"device.class":"monitor","media.class":"Audio/Source"},"ports":[],"active_port":null,"formats":["pcm"]},
 {"index":1,"state":"RUNNING","name":"alsa_input.usb-Blue_Yeti-00.analog-stereo","description":"Yeti Stereo Microphone \"Pro\"","driver":"PipeWire","monitor_of_sink":"n/a","properties":{"device.class":"sound"},"ports":[{"name":"analog-input-mic","description":"Microphone"}],"active_port":"analog-input-mic"},
 {"index":2,"name":"null-sink.monitor","description":"","properties":{}}]"#;

    #[test]
    fn pactl_json_monitors_and_mics() {
        let v = parse_json(JSON).unwrap();
        assert_eq!(
            v,
            [
                Source {
                    name: "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor".into(),
                    description: "Monitor of Built-in Audio Analog Stereo".into(),
                    monitor: true
                },
                Source {
                    name: "alsa_input.usb-Blue_Yeti-00.analog-stereo".into(),
                    description: "Yeti Stereo Microphone \"Pro\"".into(),
                    monitor: false
                },
                Source { name: "null-sink.monitor".into(), description: "null-sink.monitor".into(), monitor: true },
            ]
        );
        assert_eq!(parse_json("[]"), Some(vec![]));
        assert_eq!(parse_json("Unknown option -f"), None, "an old pactl: use the short format");
    }

    #[test]
    fn pactl_short_format() {
        let s = "0\talsa_output.pci-0000_00_1f.3.analog-stereo.monitor\tmodule-alsa-card.c\ts16le 2ch 44100Hz\tSUSPENDED\n\
                 1\talsa_input.pci-0000_00_1f.3.analog-stereo\tmodule-alsa-card.c\ts16le 2ch 44100Hz\tRUNNING\n\
                 \n";
        let v = parse_short(s);
        assert_eq!(v.len(), 2);
        assert!(v[0].monitor && !v[1].monitor);
        assert_eq!(v[1].name, "alsa_input.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(v[1].description, v[1].name);
    }

    #[test]
    fn choosing_sources() {
        let v = parse_json(JSON).unwrap();
        assert_eq!(system_source(Some(&v)), Ok(DEFAULT_MONITOR.into()));
        assert!(system_source(None).is_err());
        let mics: Vec<Source> = v.iter().filter(|s| !s.monitor).cloned().collect();
        assert!(system_source(Some(&mics)).is_err(), "no monitor");
        assert_eq!(mic_source(Some(&v), ""), Ok(DEFAULT_SOURCE.into()));
        assert_eq!(mic_source(Some(&v), "alsa_input.usb-Blue_Yeti-00.analog-stereo"), Ok("alsa_input.usb-Blue_Yeti-00.analog-stereo".into()));
        assert!(mic_source(Some(&v), "gone").is_err());
        assert!(mic_source(Some(&v), "null-sink.monitor").is_err(), "a monitor is not a microphone");
        let mons: Vec<Source> = v.iter().filter(|s| s.monitor).cloned().collect();
        assert!(mic_source(Some(&mons), "").is_err());
        assert!(mic_source(None, "").is_err());
    }
}
