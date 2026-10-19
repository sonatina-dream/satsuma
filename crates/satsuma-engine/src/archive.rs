//! Archives via macOS `bsdtar` (reads ZIP, TAR, GZIP and RAR; writes ZIP, TAR and tar.gz).

use crate::env::Env;
use crate::process::{run, Cancel};
use crate::{Error, Result};
use satsuma_core::naming::stem_of;
use satsuma_core::Format;
use std::path::Path;
use std::process::Command;

fn tar(env: &Env) -> Command {
    let mut cmd = Command::new(&env.tar);
    // No AppleDouble "._" files or extended attributes in archives we write.
    cmd.env("COPYFILE_DISABLE", "1");
    cmd
}

fn is_tarball_name(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    name.ends_with(".tar.gz") || name.ends_with(".tgz")
}

/// Extracts `input` into `out_dir` (created if needed).
pub fn extract(env: &Env, input: &Path, fmt: Format, out_dir: &Path, cancel: &Cancel) -> Result<()> {
    std::fs::create_dir_all(out_dir)?;
    let result = run(
        tar(env).arg("-xf").arg(input).arg("-C").arg(out_dir),
        cancel,
        &mut |_| {},
    );
    if let Err(e) = result {
        // A plain .gz of a single file is not a tarball: gunzip it.
        if fmt == Format::Gz && !is_tarball_name(input) {
            gunzip(input, out_dir)?;
        } else {
            return Err(match e {
                Error::Cancelled => Error::Cancelled,
                _ => Error::msg(format!(
                    "Could not extract {}. It may be damaged or encrypted.",
                    input.file_name().unwrap_or_default().to_string_lossy()
                )),
            });
        }
    }
    if std::fs::read_dir(out_dir)?.next().is_none() {
        return Err(Error::msg("The archive is empty."));
    }
    Ok(())
}

fn gunzip(input: &Path, out_dir: &Path) -> Result<()> {
    let name = input.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let inner = stem_of(&name);
    let mut decoder = flate2::read::GzDecoder::new(std::fs::File::open(input)?);
    let mut out = std::fs::File::create(out_dir.join(inner))?;
    std::io::copy(&mut decoder, &mut out).map_err(|_| Error::msg("This GZIP file looks damaged."))?;
    Ok(())
}

/// Packs the contents of `dir` into `out`; the format follows `out`'s extension.
pub fn pack(env: &Env, dir: &Path, out: &Path, target: Format, cancel: &Cancel) -> Result<()> {
    let mut entries: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    if entries.is_empty() {
        return Err(Error::msg("The archive is empty."));
    }
    let mut cmd = tar(env);
    match target {
        Format::Zip => cmd.args(["--format", "zip", "-cf"]),
        Format::Tar => cmd.args(["--format", "pax", "-cf"]),
        Format::Gz => cmd.args(["--format", "pax", "-czf"]),
        other => return Err(Error::msg(format!("Cannot create {} archives.", other.label()))),
    };
    cmd.arg(out).arg("-C").arg(dir).arg("--").args(&entries);
    run(&mut cmd, cancel, &mut |_| {})
}

/// Re-packs an archive in another format.
pub fn convert(env: &Env, input: &Path, fmt: Format, out: &Path, target: Format, tmp: &Path, cancel: &Cancel, progress: &mut dyn FnMut(f64)) -> Result<()> {
    let staging = tmp.join("archive-contents");
    extract(env, input, fmt, &staging, cancel)?;
    progress(0.5);
    pack(env, &staging, out, target, cancel)?;
    progress(1.0);
    Ok(())
}
