//! Executing jobs: write to a hidden staging path beside the destination, then publish
//! atomically with a rename. Failures and cancellation leave nothing behind.

use crate::env::Env;
use crate::plan::{Job, Operation, Plan};
use crate::process::Cancel;
use crate::{archive, doc, docx, image, imaging, media, metadata, ocr, pdf, photo, sheet, subtitle, Error, Result};
use satsuma_core::naming::{first_free, stem_of};
use satsuma_core::settings::{Settings, Strength};
use satsuma_core::{Category, Format, Tool};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Context<'a> {
    pub env: &'a Env,
    pub settings: &'a Settings,
    pub cancel: &'a Cancel,
}

static STAGING_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn output_format(job: &Job) -> Option<Format> {
    match &job.op {
        Operation::Convert(t) => Some(*t),
        op => op.tool().and_then(|t| t.output_format(job.format)),
    }
}

fn output_extension(job: &Job) -> Option<&'static str> {
    output_format(job).map(|f| f.extension())
}

fn staging_path(job: &Job) -> PathBuf {
    let dir = job.output.parent().unwrap_or(Path::new("."));
    let n = STAGING_COUNTER.fetch_add(1, Ordering::SeqCst);
    let base = format!(".satsuma-partial-{}-{n}", std::process::id());
    match output_extension(job) {
        Some(ext) => dir.join(format!("{base}.{ext}")),
        None => dir.join(base),
    }
}

fn remove_any(p: &Path) {
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Moves `from` to `to`, picking the next free name if `to` appeared meanwhile.
fn publish(from: &Path, to: &Path) -> Result<PathBuf> {
    let dir = to.parent().unwrap_or(Path::new("."));
    let mut target = to.to_path_buf();
    if target.exists() {
        let name = to.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let is_dir = from.is_dir();
        let (base, ext) = if is_dir {
            (name.clone(), None)
        } else {
            let stem = stem_of(&name).to_string();
            let ext = name[stem.len()..].trim_start_matches('.').to_string();
            (stem, Some(ext))
        };
        let free = first_free(&base, ext.as_deref().filter(|e| !e.is_empty()), &|n| dir.join(n).exists());
        target = dir.join(free);
    }
    std::fs::rename(from, &target)?;
    Ok(target)
}

fn strength_is_strong(s: Strength) -> bool {
    s == Strength::Strong
}

/// Runs one job and returns the published result path.
pub fn run_job(ctx: &Context, job: &Job, progress: &mut dyn FnMut(f64)) -> Result<PathBuf> {
    ctx.cancel.check()?;
    for input in &job.inputs {
        if !input.exists() {
            return Err(Error::msg(format!("{} no longer exists.", input.display())));
        }
    }
    let tmp = tempfile::Builder::new().prefix("satsuma-").tempdir()?;
    let staging = staging_path(job);
    let result = execute(ctx, job, &staging, tmp.path(), progress);
    match result {
        Ok(produced) => {
            // A document rendered to several page images becomes a folder.
            let final_target = if produced.is_dir() && output_extension(job).is_some() {
                let name = job.output.file_name().unwrap_or_default().to_string_lossy().into_owned();
                job.output.with_file_name(stem_of(&name))
            } else {
                job.output.clone()
            };
            match publish(&produced, &final_target) {
                Ok(p) => Ok(p),
                Err(e) => {
                    remove_any(&produced);
                    Err(e)
                }
            }
        }
        Err(e) => {
            remove_any(&staging);
            remove_any(&staging.with_extension(""));
            Err(friendly(e, job))
        }
    }
}

/// Makes tool errors readable: no full paths, no `[codec @ 0x…]` tags.
fn friendly(e: Error, job: &Job) -> Error {
    let Error::Message(mut msg) = e else { return e };
    for input in &job.inputs {
        let name = input.file_name().unwrap_or_default().to_string_lossy().into_owned();
        msg = msg.replace(&format!("{}: ", input.display()), "").replace(&input.display().to_string(), &name);
    }
    msg = strip_codec_tags(&msg);
    if let Some(detail) = msg.strip_prefix("ffmpeg failed: ") {
        let name = job.inputs[0].file_name().unwrap_or_default().to_string_lossy();
        msg = format!("Couldn't read {name}. It may be damaged or not a real {} file. ({})", job.format.label(), detail.trim());
    }
    Error::Message(msg)
}

fn strip_codec_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        match rest[i..].find(']') {
            Some(j) if rest[i..i + j].contains(" @ 0x") => {
                out.push_str(&rest[..i]);
                rest = rest[i + j + 1..].trim_start();
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn ensure_smaller(input: &Path, output: &Path) -> Result<()> {
    let before = std::fs::metadata(input)?.len();
    let after = std::fs::metadata(output)?.len();
    if after >= before {
        return Err(Error::msg("Compressed output would exceed the original file size."));
    }
    Ok(())
}

fn execute(ctx: &Context, job: &Job, out: &Path, tmp: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf> {
    let env = ctx.env;
    let cancel = ctx.cancel;
    let input = &job.inputs[0];
    let fmt = job.format;
    let stem = stem_of(&input.file_name().unwrap_or_default().to_string_lossy()).to_string();
    match &job.op {
        Operation::Crop(spec) => {
            let img = imaging::load(input)?;
            let rect = spec.pixels(imaging::size(&img));
            let cropped = imaging::crop(&img, rect, spec.output)?;
            progress(0.6);
            save_image(env, &cropped, out, fmt, tmp, cancel)?;
        }
        Operation::Frame(spec) => {
            let img = imaging::load(input)?;
            let background = match &spec.background {
                satsuma_core::edit::Background::Image(p) => Some(imaging::load(p)?),
                _ => None,
            };
            let framed = imaging::frame(&img, spec, background.as_deref(), 1.0)?;
            progress(0.6);
            imaging::write(&framed, out, Format::Png)?;
        }
        Operation::Adjust(adjustments) => {
            let img = imaging::load(input)?;
            let edited = photo::apply(&img, adjustments)?;
            progress(0.7);
            save_image(env, &edited, out, fmt, tmp, cancel)?;
        }
        Operation::Convert(target) => {
            let target = *target;
            match fmt.category() {
                Category::Image if target == Format::Docx => image_to_docx(env, input, fmt, out, tmp, cancel)?,
                Category::Image => image::convert(env, input, out, target, tmp, cancel)?,
                Category::Audio => media::convert_audio(env, input, out, target, cancel, progress)?,
                Category::Video => media::convert_video(env, input, out, target, cancel, progress)?,
                Category::Document => {
                    return doc::convert(env, input, fmt, out, target, tmp, cancel, progress);
                }
                Category::Spreadsheet => {
                    let s = sheet::read(input, fmt)?;
                    sheet::write(&s, target, out, tmp)?;
                }
                Category::Subtitle => subtitle::convert(input, out, target)?,
                Category::Archive => archive::convert(env, input, fmt, out, target, tmp, cancel, progress)?,
            }
        }
        Operation::Tool(tool) => match *tool {
            Tool::Compress => {
                match fmt.category() {
                    Category::Image => image::compress(
                        env,
                        input,
                        fmt,
                        out,
                        ctx.settings.image_compression,
                        ctx.settings.image_max_edge,
                        tmp,
                        cancel,
                    )?,
                    Category::Audio => media::compress_audio(env, input, fmt, out, ctx.settings.audio_compression, cancel, progress)?,
                    Category::Video => media::compress_video(env, input, fmt, out, ctx.settings.video_compression, cancel, progress)?,
                    _ if fmt == Format::Pdf => {
                        pdf::compress(input, out, strength_is_strong(ctx.settings.image_compression))?
                    }
                    _ => return Err(Error::msg("This file can't be compressed.")),
                }
                ensure_smaller(input, out)?;
            }
            Tool::RemoveMetadata => match fmt {
                Format::Jpg | Format::Png | Format::Webp | Format::Gif => {
                    let data = std::fs::read(input)?;
                    std::fs::write(out, metadata::strip(fmt, &data)?)?;
                }
                Format::Pdf => pdf::strip_metadata(input, out)?,
                _ => media::remove_metadata(env, input, fmt, out, cancel, progress)?,
            },
            Tool::NormalizeAudio => media::normalize(env, input, fmt, out, cancel, progress)?,
            Tool::TrimAudio => media::trim_silence(env, input, fmt, out, cancel, progress)?,
            Tool::AudioChannels => {
                let ch = media::target_channels(env, input);
                media::change_channels(env, input, fmt, out, ch, cancel, progress)?
            }
            Tool::AudioToVideo => media::visualize(env, input, out, cancel, progress)?,
            Tool::MuteVideo => media::mute(env, input, fmt, out, cancel, progress)?,
            Tool::ChangeVideoSpeed => media::change_speed(env, input, fmt, out, 2.0, cancel, progress)?,
            Tool::VideoSnapshots => media::snapshot(env, input, out, cancel, progress)?,
            Tool::SplitVideo => {
                media::split_equal(env, input, fmt, out, &stem, ctx.settings.split_video_parts, cancel, progress)?;
            }
            Tool::SplitPdf => {
                pdf::split(input, out, &stem, progress)?;
            }
            Tool::MergePdf => pdf::merge(&job.inputs, out)?,
            Tool::CreatePdf => image::images_to_pdf(env, &job.inputs, out, tmp, cancel, progress)?,
            Tool::JoinVideos => media::join(env, &job.inputs, out, cancel, progress)?,
            Tool::CreateCollage => image::collage(env, &job.inputs, out, tmp, cancel, progress)?,
            Tool::ExtractArchive => archive::extract(env, input, fmt, out, cancel)?,
            other => {
                return Err(Error::msg(format!(
                    "{} needs an editor window, which this version doesn't have yet.",
                    other.info().name
                )))
            }
        },
    }
    if !out.exists() {
        return Err(Error::msg("The operation finished without writing a result."));
    }
    progress(1.0);
    Ok(out.to_path_buf())
}

/// Writes an edited image in `format`; WebP goes through PNG and ffmpeg.
fn save_image(env: &Env, img: &imaging::Image, out: &Path, format: Format, tmp: &Path, cancel: &Cancel) -> Result<()> {
    if format == Format::Webp {
        let png = image::unique_in(tmp, "edited.png");
        imaging::write(img, &png, Format::Png)?;
        image::webp_encode(env, &png, out, 90, None, cancel)
    } else {
        imaging::write(img, out, format)
    }
}

/// Image → Word: recognised text as editable paragraphs, or the picture when there is none.
fn image_to_docx(env: &Env, input: &Path, fmt: Format, out: &Path, tmp: &Path, cancel: &Cancel) -> Result<()> {
    let readable = if matches!(fmt, Format::Jpg | Format::Png | Format::Heic | Format::Tiff) {
        input.to_path_buf()
    } else {
        image::to_png(env, input, tmp, cancel)?
    };
    let paragraphs = ocr::paragraphs(ocr::recognize(&readable)?);
    if !paragraphs.is_empty() {
        return docx::write(out, &paragraphs, None);
    }
    let img = imaging::load(&readable)?;
    let (w, h) = imaging::size(&img);
    let png = imaging::png_bytes(&img, tmp)?;
    docx::write(out, &[], Some(docx::Picture { png: &png, width_px: w as u32, height_px: h as u32 }))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchProgress {
    pub done: usize,
    pub total: usize,
    /// Overall fraction 0…1.
    pub fraction: f64,
}

#[derive(Debug, Default)]
pub struct BatchReport {
    pub outputs: Vec<PathBuf>,
    pub errors: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

/// Runs every job in order, reporting overall progress.
pub fn run_plan(ctx: &Context, plan: &Plan, on_progress: &mut dyn FnMut(BatchProgress)) -> BatchReport {
    let mut report = BatchReport::default();
    let total = plan.jobs.len();
    for (i, job) in plan.jobs.iter().enumerate() {
        let mut last = -1.0;
        let result = run_job(ctx, job, &mut |f| {
            if f - last >= 0.01 || f >= 1.0 {
                last = f;
                on_progress(BatchProgress { done: i, total, fraction: (i as f64 + f) / total as f64 });
            }
        });
        match result {
            Ok(path) => report.outputs.push(path),
            Err(Error::Cancelled) => {
                report.cancelled = true;
                break;
            }
            Err(e) => report.errors.push((job.inputs[0].clone(), e.to_string())),
        }
        on_progress(BatchProgress { done: i + 1, total, fraction: (i + 1) as f64 / total as f64 });
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_tidied() {
        let job = Job {
            inputs: vec![PathBuf::from("/x/y/broken.wav")],
            format: Format::Wav,
            op: Operation::Convert(Format::Flac),
            output: PathBuf::from("/x/y/broken.flac"),
        };
        let e = friendly(
            Error::msg("ffmpeg failed: [mp3 @ 0x79f5070000] Failed to read frame size. /x/y/broken.wav: Invalid argument"),
            &job,
        );
        assert_eq!(
            e.to_string(),
            "Couldn't read broken.wav. It may be damaged or not a real WAV file. (Failed to read frame size. Invalid argument)"
        );
    }

    #[test]
    fn publish_picks_next_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join(".satsuma-partial-1-0.tar.gz");
        std::fs::write(&staged, b"x").unwrap();
        std::fs::write(dir.path().join("a.tar.gz"), b"existing").unwrap();
        let p = publish(&staged, &dir.path().join("a.tar.gz")).unwrap();
        assert_eq!(p.file_name().unwrap(), "a 2.tar.gz");
        assert!(!staged.exists());
    }
}
