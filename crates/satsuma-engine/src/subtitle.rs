//! SRT ⇄ VTT, and subtitle → plain text.

use crate::{Error, Result};
use satsuma_core::Format;

#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

pub fn decode(bytes: &[u8]) -> Result<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::msg("The subtitle file is not valid UTF-8."))
}

/// "01:02:03,456", "01:02:03.456" or "02:03.456" → milliseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    let (clock, frac) = s.rsplit_once([',', '.'])?;
    let ms: u64 = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
    let parts: Vec<u64> = clock.split(':').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let secs = match parts.as_slice() {
        [h, m, s] => h * 3600 + m * 60 + s,
        [m, s] => m * 60 + s,
        _ => return None,
    };
    Some(secs * 1000 + ms)
}

fn format_time(ms: u64, sep: char) -> String {
    let (h, rem) = (ms / 3_600_000, ms % 3_600_000);
    let (m, rem) = (rem / 60_000, rem % 60_000);
    let (s, ms) = (rem / 1000, rem % 1000);
    format!("{h:02}:{m:02}:{s:02}{sep}{ms:03}")
}

/// Parses SRT or VTT (both are accepted by either name).
pub fn parse(text: &str) -> Result<Vec<Cue>> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    for block in normalized.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(time_idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let (a, b) = lines[time_idx].split_once("-->").unwrap();
        let end_field = b.split_whitespace().next().unwrap_or("");
        let (Some(start_ms), Some(end_ms)) = (parse_time(a), parse_time(end_field)) else {
            continue;
        };
        let text = lines[time_idx + 1..].join("\n").trim().to_string();
        if text.is_empty() {
            continue;
        }
        cues.push(Cue { start_ms, end_ms, text });
    }
    if cues.is_empty() {
        return Err(Error::msg("No valid subtitle cues were found."));
    }
    Ok(cues)
}

/// Removes all markup tags (`<i>`, `<c.yellow>`, `<v Speaker>` …).
fn strip_tags(s: &str, keep_basic: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let tag = &rest[i..i + j + 1];
        let name = tag.trim_start_matches(['<', '/']).trim_end_matches('>');
        if keep_basic && matches!(name, "i" | "b" | "u") {
            out.push_str(tag);
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

pub fn write(cues: &[Cue], fmt: Format) -> Result<String> {
    let mut out = String::new();
    match fmt {
        Format::Srt => {
            for (i, c) in cues.iter().enumerate() {
                out.push_str(&format!(
                    "{}\n{} --> {}\n{}\n\n",
                    i + 1,
                    format_time(c.start_ms, ','),
                    format_time(c.end_ms, ','),
                    strip_tags(&c.text, true)
                ));
            }
        }
        Format::Vtt => {
            out.push_str("WEBVTT\n\n");
            for c in cues {
                out.push_str(&format!(
                    "{} --> {}\n{}\n\n",
                    format_time(c.start_ms, '.'),
                    format_time(c.end_ms, '.'),
                    c.text
                ));
            }
        }
        Format::Txt => {
            let mut last = String::new();
            for c in cues {
                for line in strip_tags(&c.text, false).lines() {
                    let line = line.trim();
                    if !line.is_empty() && line != last {
                        out.push_str(line);
                        out.push('\n');
                        last = line.to_string();
                    }
                }
            }
        }
        other => return Err(Error::msg(format!("Cannot write subtitles as {}.", other.label()))),
    }
    Ok(out)
}

pub fn convert(input: &std::path::Path, out: &std::path::Path, target: Format) -> Result<()> {
    let text = decode(&std::fs::read(input)?)?;
    let cues = parse(&text)?;
    std::fs::write(out, write(&cues, target)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,500\r\nHello <i>there</i>\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nSecond\r\nline\r\n";
    const VTT: &str = "WEBVTT - demo\n\nNOTE skip me\n\nintro\n00:01.000 --> 00:02.500 align:start\n<v Ann>Hello</v> <c.yellow>there</c>\n\n00:00:03.000 --> 00:00:04.000\nSecond\n";

    #[test]
    fn times() {
        assert_eq!(parse_time("00:00:01,000"), Some(1000));
        assert_eq!(parse_time("01:02:03.456"), Some(3_723_456));
        assert_eq!(parse_time("02:03.4"), Some(123_400));
        assert_eq!(parse_time("garbage"), None);
        assert_eq!(format_time(3_723_456, ','), "01:02:03,456");
    }

    #[test]
    fn srt_to_vtt() {
        let cues = parse(SRT).unwrap();
        assert_eq!(cues.len(), 2);
        let vtt = write(&cues, Format::Vtt).unwrap();
        assert!(vtt.starts_with("WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello <i>there</i>\n"));
    }

    #[test]
    fn vtt_to_srt_and_txt() {
        let cues = parse(VTT).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        let srt = write(&cues, Format::Srt).unwrap();
        assert!(srt.starts_with("1\n00:00:01,000 --> 00:00:02,500\nHello there\n\n2\n"), "{srt}");
        let txt = write(&cues, Format::Txt).unwrap();
        assert_eq!(txt, "Hello there\nSecond\n");
    }

    #[test]
    fn errors() {
        assert!(parse("just text").is_err());
        assert!(decode(&[0xFF, 0xFE, 0x00]).is_err());
        assert_eq!(decode(b"\xEF\xBB\xBFok").unwrap(), "ok");
    }
}
