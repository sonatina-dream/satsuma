//! Running external tools with cancellation and line-by-line progress.

use crate::{Error, Result};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// Shared cancellation flag.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Runs `cmd`, calling `on_line` for each stdout line. Fails with the tail of stderr on a
/// non-zero exit, and kills the process when `cancel` is set.
pub fn run(cmd: &mut Command, cancel: &Cancel, on_line: &mut dyn FnMut(&str)) -> Result<()> {
    cancel.check()?;
    let program = cmd.get_program().to_string_lossy().into_owned();
    let tool = std::path::Path::new(&program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(program.clone());
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::msg(format!("Could not start {tool}: {e}")))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });

    let mut cancelled = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => on_line(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.is_cancelled() && !cancelled {
                    cancelled = true;
                    let _ = child.kill();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    let _ = reader.join();
    let stderr_text = err_reader.join().unwrap_or_default();
    if cancelled || cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if !status.success() {
        let tail: Vec<&str> = stderr_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let start = tail.len().saturating_sub(3);
        let detail = tail[start..].join(" ");
        return Err(Error::msg(if detail.is_empty() {
            format!("{tool} failed ({status})")
        } else {
            format!("{tool} failed: {detail}")
        }));
    }
    Ok(())
}

/// Runs a command to completion and returns its stdout and stderr, regardless of exit status.
pub fn capture(cmd: &mut Command) -> Result<(bool, String, String)> {
    let out = cmd.stdin(Stdio::null()).output()?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_lines_and_reports_failures() {
        let mut lines = Vec::new();
        run(
            Command::new("/bin/sh").args(["-c", "echo one; echo two"]),
            &Cancel::new(),
            &mut |l| lines.push(l.to_string()),
        )
        .unwrap();
        assert_eq!(lines, ["one", "two"]);

        let err = run(
            Command::new("/bin/sh").args(["-c", "echo boom >&2; exit 3"]),
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
    }

    #[test]
    fn cancellation_kills_the_process() {
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            c2.cancel();
        });
        let started = std::time::Instant::now();
        let err = run(Command::new("/bin/sleep").arg("10"), &cancel, &mut |_| {}).unwrap_err();
        assert!(matches!(err, Error::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
