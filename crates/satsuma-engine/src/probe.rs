//! Media inspection by parsing `ffmpeg -i` output (no ffprobe needed).

use crate::env::Env;
use crate::process::capture;
use crate::{Error, Result};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    pub duration: Option<f64>,
    /// Width and height of the first real video stream (cover art excluded).
    pub video: Option<(u32, u32)>,
    pub audio: Option<AudioInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u32,
}

pub fn probe(env: &Env, path: &Path) -> Result<Probe> {
    let ffmpeg = env.ffmpeg.as_ref().ok_or_else(|| Error::msg("ffmpeg is not installed."))?;
    let (_, _, stderr) = capture(
        std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-nostdin", "-i"])
            .arg(path),
    )?;
    let p = parse_probe(&stderr);
    if p.duration.is_none() && p.video.is_none() && p.audio.is_none() {
        return Err(Error::msg(format!(
            "Could not read {}.",
            path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
        )));
    }
    Ok(p)
}

pub fn parse_probe(text: &str) -> Probe {
    let mut p = Probe::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Duration:") {
            let t = rest.split(',').next().unwrap_or("").trim();
            p.duration = parse_clock(t);
        } else if line.starts_with("Stream #") {
            if line.contains("Video:") && p.video.is_none() && !line.contains("attached pic") {
                p.video = parse_dimensions(line);
            } else if line.contains("Audio:") && p.audio.is_none() {
                p.audio = Some(parse_audio(line));
            }
        }
    }
    p
}

/// "00:01:02.50" → 62.5
pub fn parse_clock(t: &str) -> Option<f64> {
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let s: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

fn parse_dimensions(line: &str) -> Option<(u32, u32)> {
    let after = line.split("Video:").nth(1)?;
    for token in after.split([' ', ',', '[']) {
        let token = token.trim();
        if let Some((w, h)) = token.split_once('x') {
            if w.starts_with('0') {
                continue; // hex values like 0x31637661
            }
            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                if w > 0 && h > 0 {
                    return Some((w, h));
                }
            }
        }
    }
    None
}

fn parse_audio(line: &str) -> AudioInfo {
    let after = line.split("Audio:").nth(1).unwrap_or("");
    let fields: Vec<&str> = after.split(',').map(str::trim).collect();
    let sample_rate = fields
        .iter()
        .find_map(|f| f.strip_suffix(" Hz").and_then(|n| n.trim().parse().ok()))
        .unwrap_or(48_000);
    let channels = fields
        .iter()
        .find_map(|f| {
            let f = f.split('(').next().unwrap_or(f).trim();
            match f {
                "mono" => Some(1),
                "stereo" => Some(2),
                "2.1" => Some(3),
                "quad" | "4.0" => Some(4),
                "5.0" => Some(5),
                "5.1" | "5.1(side)" => Some(6),
                "6.1" => Some(7),
                "7.1" => Some(8),
                _ => f.strip_suffix(" channels").and_then(|n| n.trim().parse().ok()),
            }
        })
        .unwrap_or(2);
    AudioInfo { sample_rate, channels }
}

/// Fraction done from an ffmpeg `-progress` line, given the total duration.
pub fn progress_fraction(line: &str, duration: Option<f64>) -> Option<f64> {
    let d = duration.filter(|d| *d > 0.0)?;
    let value = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    let us: f64 = value.trim().parse().ok()?;
    Some((us / 1_000_000.0 / d).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'clip.mov':
  Metadata:
    major_brand     : qt
  Duration: 00:00:04.07, start: 0.000000, bitrate: 2359 kb/s
  Stream #0:0[0x1](und): Video: hevc (Main) (hvc1 / 0x31637668), yuv420p(tv), 3130x2160, 2343 kb/s, 60 fps
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, mono, fltp, 2 kb/s
"#;

    #[test]
    fn parses_duration_video_and_audio() {
        let p = parse_probe(SAMPLE);
        assert_eq!(p.duration, Some(4.07));
        assert_eq!(p.video, Some((3130, 2160)));
        assert_eq!(p.audio, Some(AudioInfo { sample_rate: 44100, channels: 1 }));
    }

    #[test]
    fn ignores_cover_art() {
        let text = "  Duration: 00:03:00.00, start: 0\n  Stream #0:0: Audio: mp3, 48000 Hz, stereo, fltp, 320 kb/s\n  Stream #0:1: Video: mjpeg (Baseline), yuvj420p, 600x600, 90k tbn (attached pic)\n";
        let p = parse_probe(text);
        assert_eq!(p.video, None);
        assert_eq!(p.audio.unwrap().channels, 2);
        assert_eq!(p.duration, Some(180.0));
    }

    #[test]
    fn progress_lines() {
        assert_eq!(progress_fraction("out_time_us=2000000", Some(4.0)), Some(0.5));
        assert_eq!(progress_fraction("out_time_ms=8000000", Some(4.0)), Some(1.0));
        assert_eq!(progress_fraction("frame=12", Some(4.0)), None);
        assert_eq!(progress_fraction("out_time_us=1", None), None);
    }

    #[test]
    fn clock() {
        assert_eq!(parse_clock("01:00:01.5"), Some(3601.5));
        assert_eq!(parse_clock("N/A"), None);
    }
}
