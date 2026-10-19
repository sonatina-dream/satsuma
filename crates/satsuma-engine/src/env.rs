//! Discovery of the external tools Satsuma relies on.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Env {
    /// ffmpeg, if installed (audio, video, WebP, collages, PNG compression).
    pub ffmpeg: Option<PathBuf>,
    encoders: HashSet<String>,
    pub sips: PathBuf,
    pub textutil: PathBuf,
    pub tar: PathBuf,
}

impl Env {
    /// Finds tools in `$SATSUMA_FFMPEG`, next to the app bundle, Homebrew and `PATH`.
    pub fn discover() -> Env {
        let ffmpeg = find_ffmpeg();
        let encoders = ffmpeg.as_deref().map(list_encoders).unwrap_or_default();
        Env {
            ffmpeg,
            encoders,
            sips: PathBuf::from("/usr/bin/sips"),
            textutil: PathBuf::from("/usr/bin/textutil"),
            tar: PathBuf::from("/usr/bin/tar"),
        }
    }

    /// An environment without ffmpeg (for tests and diagnostics).
    pub fn without_ffmpeg() -> Env {
        Env { ffmpeg: None, encoders: HashSet::new(), ..Env::discover() }
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.ffmpeg.is_some()
    }

    pub fn has_encoder(&self, name: &str) -> bool {
        self.ffmpeg.is_some() && self.encoders.contains(name)
    }

    /// The first available encoder of `names`.
    pub fn pick_encoder<'a>(&self, names: &[&'a str]) -> Option<&'a str> {
        names.iter().copied().find(|n| self.has_encoder(n))
    }

    pub fn ffmpeg_command(&self) -> Option<Command> {
        let mut cmd = Command::new(self.ffmpeg.as_ref()?);
        cmd.args(["-hide_banner", "-nostdin", "-y"]);
        Some(cmd)
    }
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn find_ffmpeg() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("SATSUMA_FFMPEG") {
        candidates.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("../Resources/bin/ffmpeg"));
            candidates.push(dir.join("ffmpeg"));
        }
    }
    candidates.push("/opt/homebrew/bin/ffmpeg".into());
    candidates.push("/usr/local/bin/ffmpeg".into());
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("ffmpeg")));
    }
    candidates.into_iter().find(|p| is_executable(p))
}

fn list_encoders(ffmpeg: &Path) -> HashSet<String> {
    let Ok(out) = Command::new(ffmpeg).args(["-hide_banner", "-encoders"]).output() else {
        return HashSet::new();
    };
    parse_encoders(&String::from_utf8_lossy(&out.stdout))
}

/// Parses `ffmpeg -encoders` output (" A....D libmp3lame  libmp3lame MP3 …").
pub(crate) fn parse_encoders(text: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    let mut past_header = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("------") {
            past_header = true;
            continue;
        }
        if !past_header {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let (Some(flags), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if flags.len() == 6 {
            set.insert(name.to_string());
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_encoder_list() {
        let text = "Encoders:\n V..... = Video\n ------\n V....D libx264  libx264 H.264\n A....D libmp3lame  MP3\n";
        let set = parse_encoders(text);
        assert!(set.contains("libx264"));
        assert!(set.contains("libmp3lame"));
        assert!(!set.contains("V....."));
    }
}
