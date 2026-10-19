//! Turning a dropped selection plus a petal into jobs with collision-free output paths.

use crate::capability::{conversion_available, tool_available};
use crate::env::Env;
use crate::{media, Error, Result};
use satsuma_core::matrix::conversion_targets;
use satsuma_core::naming::{conversion_output_name, first_free, tool_output_name};
use satsuma_core::edit::{Adjustments, CropSpec, FrameSpec};
use satsuma_core::{Format, Tool};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    Convert(Format),
    Tool(Tool),
    /// Crop Image with the editor's settings.
    Crop(CropSpec),
    /// Add Background with the editor's settings.
    Frame(FrameSpec),
    /// Edit Photo with the editor's adjustments.
    Adjust(Adjustments),
}

impl Operation {
    /// Progress title, e.g. "Converting to MP3" or "Compressing".
    pub fn title(&self) -> String {
        match self {
            Operation::Convert(f) => format!("Converting to {}", f.label()),
            Operation::Tool(t) => t.info().verb.to_string(),
            Operation::Crop(_) => Tool::CropImage.info().verb.to_string(),
            Operation::Frame(_) => Tool::FrameImage.info().verb.to_string(),
            Operation::Adjust(_) => Tool::EditImage.info().verb.to_string(),
        }
    }

    /// The tool an edit operation belongs to.
    pub fn tool(&self) -> Option<Tool> {
        match self {
            Operation::Tool(t) => Some(*t),
            Operation::Crop(_) => Some(Tool::CropImage),
            Operation::Frame(_) => Some(Tool::FrameImage),
            Operation::Adjust(_) => Some(Tool::EditImage),
            Operation::Convert(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub inputs: Vec<PathBuf>,
    /// Format of the (first) input.
    pub format: Format,
    pub op: Operation,
    /// Where the result goes (a file, or a folder for folder-producing tools).
    pub output: PathBuf,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub jobs: Vec<Job>,
    /// Items left out, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// Format of a path, treating folders as formatless unless they are document packages.
pub fn detect_format(path: &Path) -> Option<Format> {
    let f = Format::from_path(path)?;
    if path.is_dir() && !f.is_package() {
        return None;
    }
    Some(f)
}

/// Remembers names handed out within one plan so a batch never collides with itself.
#[derive(Default)]
struct Reserved(HashSet<PathBuf>);

impl Reserved {
    fn name_in(&mut self, dir: &Path, pick: impl FnOnce(&dyn Fn(&str) -> bool) -> String) -> PathBuf {
        let taken = |name: &str| {
            let p = dir.join(name);
            p.exists() || self.0.contains(&lower(&p))
        };
        let name = pick(&taken);
        let path = dir.join(name);
        self.0.insert(lower(&path));
        path
    }
}

fn lower(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

fn parent(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn file_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

pub fn plan(env: &Env, paths: &[PathBuf], op: Operation) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut reserved = Reserved::default();
    match op.clone() {
        Operation::Crop(_) | Operation::Frame(_) | Operation::Adjust(_) => {
            let tool = op.tool().unwrap();
            for path in paths {
                match detect_format(path) {
                    Some(fmt) if tool_available(env, tool, fmt) => {
                        let name = file_name(path);
                        let label = tool.info().label;
                        let output = reserved.name_in(&parent(path), |taken| {
                            tool_output_name(&name, label, tool.output_format(fmt), taken)
                        });
                        plan.jobs.push(Job { inputs: vec![path.clone()], format: fmt, op: op.clone(), output });
                    }
                    _ => plan.skipped.push((path.clone(), format!("{} works on photos.", tool.info().name))),
                }
            }
        }
        Operation::Convert(target) => {
            for path in paths {
                let Some(fmt) = detect_format(path) else {
                    plan.skipped.push((path.clone(), "Not a convertible file.".into()));
                    continue;
                };
                if fmt == target {
                    plan.skipped.push((path.clone(), format!("Already {}.", target.label())));
                    continue;
                }
                if !conversion_targets(fmt).contains(&target) || !conversion_available(env, fmt, target) {
                    plan.skipped.push((
                        path.clone(),
                        format!("{} can't be converted to {}.", fmt.label(), target.label()),
                    ));
                    continue;
                }
                let name = file_name(path);
                let output = reserved.name_in(&parent(path), |taken| conversion_output_name(&name, target, taken));
                plan.jobs.push(Job { inputs: vec![path.clone()], format: fmt, op: op.clone(), output });
            }
        }
        Operation::Tool(tool) => {
            let mut usable = Vec::new();
            for path in paths {
                match detect_format(path) {
                    Some(fmt) if tool_available(env, tool, fmt) => usable.push((path.clone(), fmt)),
                    _ => plan.skipped.push((path.clone(), format!("{} doesn't apply to this file.", tool.info().name))),
                }
            }
            if usable.is_empty() {
                return Ok(plan);
            }
            if tool.combines_inputs() {
                let min = if tool == Tool::CreatePdf { 1 } else { 2 };
                if usable.len() < min {
                    return Err(Error::msg(match tool {
                        Tool::MergePdf => "Select at least two PDFs to merge.",
                        Tool::JoinVideos => "Select at least two videos to join.",
                        _ => "Select at least two images.",
                    }));
                }
                let (first, fmt) = usable[0].clone();
                let dir = parent(&first);
                let output = match tool {
                    Tool::MergePdf => reserved.name_in(&dir, |t| first_free("Merged", Some("pdf"), t)),
                    Tool::JoinVideos => reserved.name_in(&dir, |t| first_free("Joined", Some("mp4"), t)),
                    Tool::CreateCollage => reserved.name_in(&dir, |t| first_free("Collage", Some("png"), t)),
                    _ => {
                        let name = file_name(&first);
                        reserved.name_in(&dir, |t| tool_output_name(&name, "Images", Some(Format::Pdf), t))
                    }
                };
                plan.jobs.push(Job {
                    inputs: usable.into_iter().map(|(p, _)| p).collect(),
                    format: fmt,
                    op: op.clone(),
                    output,
                });
            } else {
                for (path, fmt) in usable {
                    let label = match tool {
                        Tool::AudioChannels => {
                            if media::target_channels(env, &path) == 1 { "Mono" } else { "Stereo" }
                        }
                        _ => tool.info().label,
                    };
                    let name = file_name(&path);
                    let output = reserved.name_in(&parent(&path), |taken| {
                        tool_output_name(&name, label, tool.output_format(fmt), taken)
                    });
                    plan.jobs.push(Job { inputs: vec![path], format: fmt, op: op.clone(), output });
                }
            }
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_names_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("photo.png");
        let b = dir.path().join("photo.heic");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        std::fs::write(dir.path().join("photo.jpg"), b"x").unwrap();
        let env = Env::without_ffmpeg();
        let p = plan(&env, &[a.clone(), b.clone()], Operation::Convert(Format::Jpg)).unwrap();
        let outs: Vec<String> = p.jobs.iter().map(|j| file_name(&j.output)).collect();
        assert_eq!(outs, ["photo 2.jpg", "photo 3.jpg"]);
    }

    #[test]
    fn same_format_and_folders_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        std::fs::write(&a, b"x").unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let env = Env::without_ffmpeg();
        let p = plan(&env, &[a, folder], Operation::Convert(Format::Jpg)).unwrap();
        assert!(p.jobs.is_empty());
        assert_eq!(p.skipped.len(), 2);
    }

    #[test]
    fn tool_names_and_combined_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("one.pdf");
        let b = dir.path().join("two.pdf");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        let env = Env::without_ffmpeg();
        let p = plan(&env, &[a.clone(), b.clone()], Operation::Tool(Tool::MergePdf)).unwrap();
        assert_eq!(p.jobs.len(), 1);
        assert_eq!(p.jobs[0].inputs.len(), 2);
        assert_eq!(file_name(&p.jobs[0].output), "Merged.pdf");

        let p = plan(&env, &[a.clone()], Operation::Tool(Tool::SplitPdf)).unwrap();
        assert_eq!(file_name(&p.jobs[0].output), "one Pages");

        assert!(plan(&env, &[a], Operation::Tool(Tool::MergePdf)).is_err());
    }

    #[test]
    fn titles() {
        assert_eq!(Operation::Convert(Format::Mp3).title(), "Converting to MP3");
        assert_eq!(Operation::Tool(Tool::ExtractArchive).title(), "Extracting archive");
    }
}
