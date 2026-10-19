//! Audio and video conversions and tools, implemented with ffmpeg.

use crate::env::Env;
use crate::probe::{probe, progress_fraction, Probe};
use crate::process::{run, Cancel};
use crate::{Error, Result};
use satsuma_core::settings::Strength;
use satsuma_core::Format::{self, *};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Keeps H.264/VP9/MPEG-4 happy with odd-sized sources.
const EVEN: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

pub(crate) struct Ff<'a> {
    env: &'a Env,
    args: Vec<OsString>,
    duration: Option<f64>,
}

impl<'a> Ff<'a> {
    pub(crate) fn new(env: &'a Env) -> Self {
        Ff { env, args: Vec::new(), duration: None }
    }

    pub(crate) fn arg(mut self, a: impl Into<OsString>) -> Self {
        self.args.push(a.into());
        self
    }

    pub(crate) fn args<I, S>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(items.into_iter().map(Into::into));
        self
    }

    pub(crate) fn input(self, p: &Path) -> Self {
        self.arg("-i").arg(p)
    }

    pub(crate) fn duration(mut self, d: Option<f64>) -> Self {
        self.duration = d;
        self
    }

    pub(crate) fn run(self, out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
        let mut cmd = self
            .env
            .ffmpeg_command()
            .ok_or_else(|| Error::msg("ffmpeg is not installed. Install it with `brew install ffmpeg`."))?;
        cmd.args(["-loglevel", "error", "-progress", "pipe:1", "-nostats"]);
        cmd.args(&self.args);
        cmd.arg(out);
        let duration = self.duration;
        run(&mut cmd, cancel, &mut |line| {
            if let Some(f) = progress_fraction(line, duration) {
                progress(f)
            }
        })?;
        if !out.exists() {
            return Err(Error::msg("ffmpeg finished without writing an output file."));
        }
        progress(1.0);
        Ok(())
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Audio encoder arguments for an output format.
pub fn audio_codec_args(env: &Env, target: Format, kbps: u32) -> Result<Vec<String>> {
    let br = format!("{kbps}k");
    let missing = |what: &str| Error::msg(format!("This ffmpeg build cannot encode {what}."));
    Ok(match target {
        Mp3 => {
            if !env.has_encoder("libmp3lame") {
                return Err(missing("MP3"));
            }
            vec!["-c:a".into(), "libmp3lame".into(), "-b:a".into(), br]
        }
        M4a | Mp4 | Mov | Mkv => {
            let enc = env.pick_encoder(&["aac", "aac_at"]).ok_or_else(|| missing("AAC"))?;
            vec!["-c:a".into(), enc.into(), "-b:a".into(), br]
        }
        Wav => s(&["-c:a", "pcm_s16le"]),
        Aiff => s(&["-c:a", "pcm_s16be"]),
        Flac => s(&["-c:a", "flac", "-compression_level", "8"]),
        Ogg => match env.pick_encoder(&["libvorbis", "vorbis"]) {
            Some("libvorbis") => s(&["-c:a", "libvorbis", "-q:a", "6"]),
            Some(_) => s(&["-c:a", "vorbis", "-strict", "experimental", "-ac", "2"]),
            None => return Err(missing("Ogg Vorbis")),
        },
        Opus | Webm => match env.pick_encoder(&["libopus", "opus"]) {
            Some("libopus") => vec!["-c:a".into(), "libopus".into(), "-b:a".into(), br],
            Some(_) => vec![
                "-c:a".into(),
                "opus".into(),
                "-strict".into(),
                "experimental".into(),
                "-ar".into(),
                "48000".into(),
                "-b:a".into(),
                br,
            ],
            None => return Err(missing("Opus")),
        },
        Wma | Wmv => {
            if !env.has_encoder("wmav2") {
                return Err(missing("WMA"));
            }
            vec!["-c:a".into(), "wmav2".into(), "-b:a".into(), br]
        }
        Avi => {
            if env.has_encoder("libmp3lame") {
                vec!["-c:a".into(), "libmp3lame".into(), "-b:a".into(), br]
            } else {
                s(&["-c:a", "pcm_s16le"])
            }
        }
        other => return Err(Error::msg(format!("{} is not an audio format.", other.label()))),
    })
}

fn h264_args(env: &Env, crf: u32) -> Result<Vec<String>> {
    if env.has_encoder("libx264") {
        Ok(vec![
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
            "-crf".into(),
            crf.to_string(),
            "-pix_fmt".into(),
            "yuv420p".into(),
        ])
    } else if env.has_encoder("h264_videotoolbox") {
        Ok(s(&["-c:v", "h264_videotoolbox", "-q:v", "65", "-pix_fmt", "yuv420p"]))
    } else {
        Err(Error::msg("This ffmpeg build cannot encode H.264 video."))
    }
}

/// Video (and audio) encoder arguments for a video container.
pub fn video_codec_args(env: &Env, target: Format) -> Result<Vec<String>> {
    let mut a = Vec::new();
    match target {
        Mp4 | Mov | Mkv => {
            a.extend(h264_args(env, 20)?);
            a.extend(audio_codec_args(env, target, 192)?);
            if target != Mkv {
                a.extend(s(&["-movflags", "+faststart"]));
            }
        }
        Webm => {
            if !env.has_encoder("libvpx-vp9") {
                return Err(Error::msg("This ffmpeg build cannot encode VP9 video."));
            }
            a.extend(s(&[
                "-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-row-mt", "1", "-deadline", "good",
                "-cpu-used", "4", "-pix_fmt", "yuv420p",
            ]));
            a.extend(audio_codec_args(env, Webm, 128)?);
        }
        Avi => {
            a.extend(s(&["-c:v", "mpeg4", "-q:v", "3", "-vtag", "xvid"]));
            a.extend(audio_codec_args(env, Avi, 192)?);
        }
        Wmv => {
            if !env.has_encoder("wmv2") {
                return Err(Error::msg("This ffmpeg build cannot encode WMV video."));
            }
            a.extend(s(&["-c:v", "wmv2", "-b:v", "5M"]));
            a.extend(audio_codec_args(env, Wmv, 192)?);
        }
        other => return Err(Error::msg(format!("{} is not a video format.", other.label()))),
    }
    Ok(a)
}

fn probe_opt(env: &Env, input: &Path) -> Probe {
    probe(env, input).unwrap_or_default()
}

/// audio → audio
pub fn convert_audio(env: &Env, input: &Path, out: &Path, target: Format, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe_opt(env, input);
    Ff::new(env)
        .input(input)
        .args(["-map", "0:a:0", "-vn", "-map_metadata", "0"])
        .args(audio_codec_args(env, target, 192)?)
        .duration(p.duration)
        .run(out, cancel, progress)
}

/// video/gif → video, gif or mp3
pub fn convert_video(env: &Env, input: &Path, out: &Path, target: Format, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe_opt(env, input);
    let ff = Ff::new(env).input(input).duration(p.duration);
    let ff = match target {
        Gif => ff
            .args(["-map", "0:v:0", "-an", "-vf"])
            .arg("fps=12,scale='min(640,iw)':-2:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4")
            .args(["-loop", "0"]),
        Mp3 => ff
            .args(["-map", "0:a:0", "-vn"])
            .args(audio_codec_args(env, Mp3, 192)?),
        _ => ff
            .args(["-map", "0:v:0", "-map", "0:a:0?", "-vf", EVEN])
            .args(video_codec_args(env, target)?),
    };
    if target == Mp3 && p.audio.is_none() && p.duration.is_some() {
        return Err(Error::msg("This video has no audio track."));
    }
    ff.run(out, cancel, progress)
}

fn compress_kbps(strength: Strength) -> u32 {
    match strength {
        Strength::Balanced => 128,
        Strength::Strong => 96,
    }
}

/// Compress audio (mp3, m4a: lower bitrate; wav: lossless FLAC; flac: max compression).
pub fn compress_audio(env: &Env, input: &Path, in_fmt: Format, out: &Path, strength: Strength, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe_opt(env, input);
    let codec = match in_fmt {
        Wav => s(&["-c:a", "flac", "-compression_level", "8"]),
        Flac => s(&["-c:a", "flac", "-compression_level", "12"]),
        other => audio_codec_args(env, other, compress_kbps(strength))?,
    };
    Ff::new(env)
        .input(input)
        .args(["-map", "0:a:0", "-vn", "-map_metadata", "0"])
        .args(codec)
        .duration(p.duration)
        .run(out, cancel, progress)
}

/// Compress video with HEVC (VideoToolbox when available).
pub fn compress_video(env: &Env, input: &Path, in_fmt: Format, out: &Path, strength: Strength, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe_opt(env, input);
    let strong = strength == Strength::Strong;
    let mut v: Vec<String> = if env.has_encoder("hevc_videotoolbox") {
        vec!["-c:v".into(), "hevc_videotoolbox".into(), "-q:v".into(), (if strong { "38" } else { "50" }).into()]
    } else if env.has_encoder("libx265") {
        vec!["-c:v".into(), "libx265".into(), "-preset".into(), "fast".into(), "-crf".into(), (if strong { "32" } else { "28" }).into()]
    } else {
        h264_args(env, if strong { 30 } else { 26 })?
    };
    if !v.iter().any(|a| a == "-pix_fmt") {
        v.extend(s(&["-pix_fmt", "yuv420p"]));
    }
    if in_fmt != Mkv && v.iter().any(|a| a.contains("hevc") || a.contains("265")) {
        v.extend(s(&["-tag:v", "hvc1"]));
    }
    let audio = audio_codec_args(env, Mp4, if strong { 96 } else { 128 })?;
    let mut ff = Ff::new(env)
        .input(input)
        .args(["-map", "0:v:0", "-map", "0:a:0?", "-vf", EVEN])
        .args(v)
        .args(audio)
        .duration(p.duration);
    if in_fmt != Mkv {
        ff = ff.args(["-movflags", "+faststart"]);
    }
    ff.run(out, cancel, progress)
}

/// Strip container and stream metadata without re-encoding.
pub fn remove_metadata(env: &Env, input: &Path, in_fmt: Format, out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let mut ff = Ff::new(env)
        .input(input)
        .args(["-map", "0", "-map_metadata", "-1", "-map_metadata:s", "-1", "-map_chapters", "-1"])
        .args(["-c", "copy", "-fflags", "+bitexact", "-flags:v", "+bitexact", "-flags:a", "+bitexact"]);
    if matches!(in_fmt, Mp4 | Mov | M4a) {
        ff = ff.args(["-movflags", "+faststart"]);
    }
    if in_fmt == Mp3 {
        ff = ff.args(["-write_xing", "0", "-id3v2_version", "0"]);
    }
    if in_fmt == Gif {
        return Err(Error::msg("GIF metadata is removed without ffmpeg."));
    }
    ff.run(out, cancel, progress)
}

fn audio_filter_tool(env: &Env, input: &Path, in_fmt: Format, out: &Path, extra: Vec<String>, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe_opt(env, input);
    Ff::new(env)
        .input(input)
        .args(["-map", "0:a:0", "-vn", "-map_metadata", "0"])
        .args(extra)
        .args(audio_codec_args(env, in_fmt, 192)?)
        .duration(p.duration)
        .run(out, cancel, progress)
}

/// EBU R128 loudness normalisation to −16 LUFS.
pub fn normalize(env: &Env, input: &Path, in_fmt: Format, out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let rate = probe_opt(env, input).audio.map(|a| a.sample_rate).unwrap_or(48_000);
    audio_filter_tool(
        env,
        input,
        in_fmt,
        out,
        vec!["-af".into(), "loudnorm=I=-16:LRA=11:TP=-1.5".into(), "-ar".into(), rate.to_string()],
        cancel,
        progress,
    )
}

/// Remove silence at the start and end only, keeping pauses inside.
pub fn trim_silence(env: &Env, input: &Path, in_fmt: Format, out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let edge = "silenceremove=start_periods=1:start_duration=0:start_threshold=-50dB:start_silence=0.05";
    let filter = format!("{edge},areverse,{edge},areverse");
    audio_filter_tool(env, input, in_fmt, out, vec!["-af".into(), filter], cancel, progress)
}

/// Mono ↔ stereo. Returns the channel count written.
pub fn change_channels(env: &Env, input: &Path, in_fmt: Format, out: &Path, channels: u32, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    audio_filter_tool(env, input, in_fmt, out, vec!["-ac".into(), channels.to_string()], cancel, progress)
}

/// The channel count Convert Audio Channels will produce: mono, or stereo for mono input.
pub fn target_channels(env: &Env, input: &Path) -> u32 {
    match probe_opt(env, input).audio {
        Some(a) if a.channels == 1 => 2,
        _ => 1,
    }
}

pub fn mute(env: &Env, input: &Path, in_fmt: Format, out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe_opt(env, input);
    let mut ff = Ff::new(env)
        .input(input)
        .args(["-map", "0:v:0", "-c:v", "copy", "-an", "-map_metadata", "0"])
        .duration(p.duration);
    if in_fmt != Mkv {
        ff = ff.args(["-movflags", "+faststart"]);
    }
    ff.run(out, cancel, progress)
}

pub fn change_speed(env: &Env, input: &Path, in_fmt: Format, out: &Path, factor: f64, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe_opt(env, input);
    let mut ff = Ff::new(env)
        .input(input)
        .arg("-filter:v")
        .arg(format!("setpts=PTS/{factor},{EVEN}"))
        .args(["-map", "0:v:0"]);
    if p.audio.is_some() {
        ff = ff.args(["-map", "0:a:0", "-filter:a"]).arg(atempo_chain(factor));
    }
    ff.args(video_codec_args(env, in_fmt)?)
        .duration(p.duration.map(|d| d / factor))
        .run(out, cancel, progress)
}

/// `atempo` only accepts 0.5–2.0 per stage, so chain stages for other factors.
pub fn atempo_chain(mut factor: f64) -> String {
    let mut stages = Vec::new();
    while factor > 2.0 {
        stages.push("atempo=2.0".to_string());
        factor /= 2.0;
    }
    while factor < 0.5 {
        stages.push("atempo=0.5".to_string());
        factor /= 0.5;
    }
    stages.push(format!("atempo={factor}"));
    stages.join(",")
}

/// Full-resolution PNG of the middle frame.
pub fn snapshot(env: &Env, input: &Path, out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe(env, input)?;
    let at = p.duration.unwrap_or(0.0) / 2.0;
    Ff::new(env)
        .arg("-ss")
        .arg(format!("{at:.3}"))
        .input(input)
        .args(["-map", "0:v:0", "-frames:v", "1", "-update", "1"])
        .run(out, cancel, progress)
}

/// Split into `parts` equal clips inside `out_dir`.
pub fn split_equal(env: &Env, input: &Path, in_fmt: Format, out_dir: &Path, stem: &str, parts: u32, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<Vec<PathBuf>> {
    let p = probe(env, input)?;
    let duration = p.duration.ok_or_else(|| Error::msg("Could not read the video duration."))?;
    let parts = parts.max(2);
    let len = duration / parts as f64;
    if len < 0.05 {
        return Err(Error::msg("The video is too short to split."));
    }
    std::fs::create_dir_all(out_dir)?;
    let mut outputs = Vec::new();
    for i in 0..parts {
        let out = out_dir.join(format!("{stem} {:02}.{}", i + 1, in_fmt.extension()));
        let start = len * i as f64;
        let base = i as f64 / parts as f64;
        let mut ff = Ff::new(env)
            .arg("-ss")
            .arg(format!("{start:.3}"))
            .input(input)
            .arg("-t")
            .arg(format!("{len:.3}"))
            .args(["-map", "0:v:0", "-map", "0:a:0?", "-vf", EVEN])
            .args(video_codec_args(env, in_fmt)?)
            .duration(Some(len));
        if in_fmt != Mkv {
            ff = ff.args(["-avoid_negative_ts", "make_zero"]);
        }
        ff.run(&out, cancel, &mut |f| progress(base + f / parts as f64))?;
        outputs.push(out);
    }
    Ok(outputs)
}

/// Join clips into one MP4; the first clip sets the canvas, others are letterboxed.
pub fn join(env: &Env, inputs: &[PathBuf], out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    if inputs.len() < 2 {
        return Err(Error::msg("Select at least two videos to join."));
    }
    let probes: Vec<Probe> = inputs.iter().map(|i| probe(env, i)).collect::<Result<_>>()?;
    if probes.iter().any(|p| p.video.is_none()) {
        return Err(Error::msg("Every selected file must contain a video track."));
    }
    let (w, h) = probes[0].video.unwrap();
    let (w, h) = (w / 2 * 2, h / 2 * 2);
    let total: f64 = probes.iter().filter_map(|p| p.duration).sum();

    let mut ff = Ff::new(env);
    for i in inputs {
        ff = ff.input(i);
    }
    // Silent audio for clips without an audio track.
    let mut silent_index = inputs.len();
    let mut graph = String::new();
    let mut concat_inputs = String::new();
    for (k, p) in probes.iter().enumerate() {
        graph.push_str(&format!(
            "[{k}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30,format=yuv420p[v{k}];"
        ));
        if p.audio.is_some() {
            graph.push_str(&format!(
                "[{k}:a:0]aformat=sample_rates=48000:channel_layouts=stereo[a{k}];"
            ));
        } else {
            let d = p.duration.unwrap_or(1.0);
            ff = ff
                .args(["-f", "lavfi", "-t"])
                .arg(format!("{d:.3}"))
                .args(["-i", "anullsrc=r=48000:cl=stereo"]);
            graph.push_str(&format!("[{silent_index}:a:0]anull[a{k}];"));
            silent_index += 1;
        }
        concat_inputs.push_str(&format!("[v{k}][a{k}]"));
    }
    graph.push_str(&format!(
        "{concat_inputs}concat=n={}:v=1:a=1[v][a]",
        inputs.len()
    ));
    ff.arg("-filter_complex")
        .arg(graph)
        .args(["-map", "[v]", "-map", "[a]"])
        .args(h264_args(env, 20)?)
        .args(audio_codec_args(env, Mp4, 192)?)
        .args(["-movflags", "+faststart"])
        .duration(Some(total))
        .run(out, cancel, progress)
}

/// MP4 with an orange waveform on black.
pub fn visualize(env: &Env, input: &Path, out: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let p = probe(env, input)?;
    Ff::new(env)
        .input(input)
        .arg("-filter_complex")
        .arg("[0:a:0]showwaves=s=1280x720:mode=cline:rate=30:colors=0xFF7A1A:scale=sqrt,format=yuv420p[v]")
        .args(["-map", "[v]", "-map", "0:a:0"])
        .args(h264_args(env, 23)?)
        .args(audio_codec_args(env, Mp4, 192)?)
        .args(["-shortest", "-movflags", "+faststart"])
        .duration(p.duration)
        .run(out, cancel, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atempo_stays_in_range() {
        assert_eq!(atempo_chain(2.0), "atempo=2");
        assert_eq!(atempo_chain(4.0), "atempo=2.0,atempo=2");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.5");
    }
}
