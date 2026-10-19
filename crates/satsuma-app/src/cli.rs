//! Command-line interface on the same binary.

use satsuma_core::matrix::{conversion_targets, total_conversion_paths};
use satsuma_core::settings::Settings;
use satsuma_core::wheel::wheel_items;
use satsuma_core::{Format, Tool, WheelItem, WheelMode};
use satsuma_engine::{capability, detect_format, plan, run_plan, Cancel, Context, Env, Operation};
use std::io::Write;
use std::path::PathBuf;

const USAGE: &str = "Satsuma — drag-gesture file converter for macOS

USAGE:
  satsuma                                   Start the menu-bar app
  satsuma formats                           List every conversion path and whether it works here
  satsuma targets <files…>                  Show the Convert, Tools and Actions wheels for files
  satsuma convert --to <format> <files…>    Convert files (results go next to the originals)
  satsuma tool <tool> <files…>              Run a tool, e.g. compress, removeMetadata, mergePDF
        cropImage  --aspect 1:1 [--size 1080x1080]
        frameImage [--background sunset|white|<image>] [--padding px] [--corners px] [--shadow px] [--ratio 1:1]
        editImage  --set exposure=20 --set contrast=15 …   (keys: exposure, brightness, contrast,
                   highlights, shadows, whites, blacks, gamma, temperature, tint, saturation, vibrance,
                   clarity, texture, sharpness, dehaze, noiseReduction, fade, vignette, grain)
  satsuma tools                             List tool identifiers
  satsuma render-wheel [options] --out <png>
        --mode conversions|tools|actions    Which wheel (default conversions)
        --format <format>                   Pretend the dragged files have this format
        --count <n>                         How many files (default 1)
        --selected <index>                  Highlight a petal
        --dark                              Dark appearance
  satsuma render-icon --size <px> --out <png>   App icon artwork (used by scripts/bundle.sh)
  satsuma --version";

pub fn run(args: &[String]) -> i32 {
    let cmd = args[0].as_str();
    let rest = &args[1..];
    match cmd {
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            0
        }
        "-V" | "--version" => {
            println!("satsuma {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "formats" => formats(),
        "tools" => {
            for t in Tool::ALL {
                let info = t.info();
                let note = if t.has_editor() {
                    "  (editor window in the app; options on the command line)"
                } else if t.is_interactive() {
                    "  (editor; not in this version)"
                } else {
                    ""
                };
                println!("{:<18} {}{}", info.id, info.name, note);
            }
            0
        }
        "targets" => targets(rest),
        "convert" => convert(rest),
        "tool" => tool(rest),
        "render-wheel" => render_wheel(rest),
        "render-icon" => render_icon(rest),
        "render-editor" => render_editor(rest),
        #[cfg(feature = "demo")]
        "render-demo" => crate::demo::run(rest),
        other => {
            eprintln!("Unknown command `{other}`.\n\n{USAGE}");
            2
        }
    }
}

fn formats() -> i32 {
    let env = Env::discover();
    for f in Format::ALL {
        let targets = conversion_targets(f);
        if targets.is_empty() {
            continue;
        }
        let list: Vec<String> = targets
            .iter()
            .map(|t| {
                if capability::conversion_available(&env, f, *t) {
                    t.label()
                } else {
                    format!("({})", t.label())
                }
            })
            .collect();
        println!("{:<6} → {}", f.label(), list.join(" "));
    }
    println!(
        "\n{} conversion paths in the matrix; {} work on this Mac (unavailable ones are in parentheses).",
        total_conversion_paths(),
        capability::available_paths(&env)
    );
    if !env.has_ffmpeg() {
        println!("Install ffmpeg (`brew install ffmpeg`) for audio and video.");
    }
    0
}

fn paths(args: &[String]) -> Vec<PathBuf> {
    args.iter().map(|a| std::fs::canonicalize(a).unwrap_or_else(|_| PathBuf::from(a))).collect()
}

fn targets(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Usage: satsuma targets <files…>");
        return 2;
    }
    let env = Env::discover();
    let files = paths(args);
    let formats: Vec<_> = files.iter().map(|p| detect_format(p)).collect();
    for mode in WheelMode::ALL {
        let items = wheel_items(mode, &formats, &|i| capability::item_available(&env, i, &formats));
        let names: Vec<String> = items.iter().map(describe).collect();
        let shown = if names.is_empty() { satsuma_core::wheel::empty_message(mode).to_string() } else { names.join(", ") };
        println!("{:<8} {}", mode.name(), shown);
    }
    0
}

fn describe(i: &WheelItem) -> String {
    match i {
        WheelItem::Convert(f) => f.id().to_string(),
        WheelItem::Tool(t) => t.id().to_string(),
        WheelItem::Action(a) => a.id().to_string(),
    }
}

fn execute(op: Operation, files: Vec<PathBuf>) -> i32 {
    let env = Env::discover();
    let settings = Settings::load(&Settings::default_path());
    let title = op.title();
    let p = match plan(&env, &files, op) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    for (path, why) in &p.skipped {
        eprintln!("skipped {}: {why}", path.display());
    }
    if p.jobs.is_empty() {
        eprintln!("Nothing to do.");
        return 1;
    }
    let cancel = Cancel::new();
    let ctx = Context { env: &env, settings: &settings, cancel: &cancel };
    let report = run_plan(&ctx, &p, &mut |bp| {
        eprint!("\r{title}… {:>3.0}%  ({} of {} saved)", bp.fraction * 100.0, bp.done, bp.total);
        let _ = std::io::stderr().flush();
    });
    eprintln!();
    for out in &report.outputs {
        println!("{}", out.display());
    }
    for (path, err) in &report.errors {
        eprintln!("error {}: {err}", path.display());
    }
    if report.errors.is_empty() { 0 } else { 1 }
}

fn convert(args: &[String]) -> i32 {
    let mut target = None;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--to" || a == "-t" {
            target = it.next().and_then(|f| Format::from_id(f));
        } else {
            files.push(a.clone());
        }
    }
    let Some(target) = target else {
        eprintln!("Usage: satsuma convert --to <format> <files…>");
        return 2;
    };
    if files.is_empty() {
        eprintln!("No files given.");
        return 2;
    }
    execute(Operation::Convert(target), paths(&files))
}

fn parse_ratio(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once([':', 'x'])?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn tool(args: &[String]) -> i32 {
    let Some(t) = args.first().and_then(|id| Tool::from_id(id)) else {
        eprintln!("Usage: satsuma tool <tool> [options] <files…>   (see `satsuma tools`)");
        return 2;
    };
    // Editor tools take their settings as options.
    let mut files = Vec::new();
    let mut sets: Vec<(String, f64)> = Vec::new();
    let mut aspect = None;
    let mut size = None;
    let mut ratio = None;
    let mut background = None;
    let mut numbers: Vec<(String, f64)> = Vec::new();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--set" => {
                if let Some((k, v)) = it.next().and_then(|kv| kv.split_once('=')) {
                    sets.push((k.to_string(), v.parse().unwrap_or(0.0)));
                }
            }
            "--aspect" => aspect = it.next().and_then(|v| parse_ratio(v)),
            "--size" => size = it.next().and_then(|v| parse_ratio(v)),
            "--ratio" => ratio = it.next().map(|v| parse_ratio(v)),
            "--background" => background = it.next().cloned(),
            "--padding" | "--corners" | "--shadow" => {
                if let Some(v) = it.next().and_then(|v| v.parse().ok()) {
                    numbers.push((a.trim_start_matches('-').to_string(), v));
                }
            }
            _ => files.push(a.clone()),
        }
    }
    if files.is_empty() {
        eprintln!("No files given.");
        return 2;
    }
    let files = paths(&files);
    match t {
        Tool::EditImage => {
            let mut adj = satsuma_core::edit::Adjustments::default();
            for (k, v) in &sets {
                match satsuma_core::edit::Adjustments::index(k) {
                    Some(i) => adj.set(i, *v),
                    None => {
                        eprintln!("Unknown adjustment `{k}`.");
                        return 2;
                    }
                }
            }
            execute(Operation::Adjust(adj), files)
        }
        Tool::CropImage | Tool::FrameImage => {
            // Crop and background settings depend on each image's size.
            let mut code = 0;
            for f in files {
                let dims = match satsuma_engine::imaging::load(&f) {
                    Ok(img) => satsuma_engine::imaging::size(&img),
                    Err(e) => {
                        eprintln!("{}: {e}", f.display());
                        code = 1;
                        continue;
                    }
                };
                let op = if t == Tool::CropImage {
                    let mut spec = satsuma_core::edit::CropSpec::centered(aspect, dims);
                    spec.output = size;
                    Operation::Crop(spec)
                } else {
                    let mut spec = satsuma_core::edit::FrameSpec::defaults_for(dims);
                    if let Some(r) = ratio {
                        spec.ratio = r;
                    }
                    for (k, v) in &numbers {
                        match k.as_str() {
                            "padding" => spec.padding = *v,
                            "corners" => spec.corners = *v,
                            _ => spec.shadow = *v,
                        }
                    }
                    if let Some(b) = &background {
                        let lower = b.to_ascii_lowercase();
                        spec.background = if let Some(i) = satsuma_core::edit::GRADIENTS.iter().position(|g| g.name.to_ascii_lowercase() == lower) {
                            satsuma_core::edit::Background::Gradient(i)
                        } else if let Some((_, (r, g, bl))) = satsuma_core::edit::SOLIDS.iter().find(|s| s.0.to_ascii_lowercase() == lower) {
                            satsuma_core::edit::Background::Solid { r: *r, g: *g, b: *bl }
                        } else {
                            satsuma_core::edit::Background::Image(PathBuf::from(b))
                        };
                    }
                    Operation::Frame(spec)
                };
                code = code.max(execute(op, vec![f]));
            }
            code
        }
        _ => execute(Operation::Tool(t), files),
    }
}

fn render_wheel(args: &[String]) -> i32 {
    let mut mode = WheelMode::Conversions;
    let mut format = Format::Wav;
    let mut count = 1usize;
    let mut selected = None;
    let mut dark = false;
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mode" => {
                mode = match it.next().map(String::as_str) {
                    Some("tools") => WheelMode::Tools,
                    Some("actions") => WheelMode::Actions,
                    _ => WheelMode::Conversions,
                }
            }
            "--format" => format = it.next().and_then(|f| Format::from_id(f)).unwrap_or(format),
            "--count" => count = it.next().and_then(|n| n.parse().ok()).unwrap_or(1),
            "--selected" => selected = it.next().and_then(|n| n.parse().ok()),
            "--dark" => dark = true,
            "--out" | "-o" => out = it.next().cloned(),
            other => {
                eprintln!("Unknown option {other}");
                return 2;
            }
        }
    }
    let Some(out) = out else {
        eprintln!("Usage: satsuma render-wheel --out wheel.png [--mode …] [--format …]");
        return 2;
    };
    let env = Env::discover();
    let formats = vec![Some(format); count.max(1)];
    let items = wheel_items(mode, &formats, &|i| capability::item_available(&env, i, &formats));
    let mut model = crate::wheel::WheelModel::new(mode, items);
    model.selected = selected;
    let mtm = crate::system::main_thread();
    // AppKit drawing needs the shared application.
    let _app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    match crate::wheel::render_png(mtm, model, dark, std::path::Path::new(&out)) {
        Ok(()) => {
            println!("{out}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn render_icon(args: &[String]) -> i32 {
    let mut size = 1024usize;
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => size = it.next().and_then(|n| n.parse().ok()).unwrap_or(1024),
            "--out" | "-o" => out = it.next().cloned(),
            other => {
                eprintln!("Unknown option {other}");
                return 2;
            }
        }
    }
    let Some(out) = out else {
        eprintln!("Usage: satsuma render-icon --size 1024 --out icon.png");
        return 2;
    };
    let mtm = crate::system::main_thread();
    let _app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    match crate::icon::render_icon(size, std::path::Path::new(&out)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Hidden: `satsuma render-editor crop|frame|edit <image> --out <png> [--segment N] [--set key=value]`.
fn render_editor(args: &[String]) -> i32 {
    let (Some(kind), Some(image)) = (args.first(), args.get(1)) else {
        eprintln!("Usage: satsuma render-editor crop|frame <image> --out <png> [--segment N]");
        return 2;
    };
    let mut out = None;
    let mut segment = None;
    let mut sets: Vec<(String, f64)> = Vec::new();
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" | "-o" => out = it.next().cloned(),
            "--segment" => segment = it.next().and_then(|n| n.parse().ok()),
            "--set" => {
                if let Some((k, v)) = it.next().and_then(|kv| kv.split_once('=')) {
                    sets.push((k.to_string(), v.parse().unwrap_or(0.0)));
                }
            }
            other => {
                eprintln!("Unknown option {other}");
                return 2;
            }
        }
    }
    let Some(out) = out else {
        eprintln!("--out is required");
        return 2;
    };
    let mtm = crate::system::main_thread();
    let _app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let delegate = crate::app::AppDelegate::new(mtm);
    let d = objc2::runtime::ProtocolObject::from_ref(&*delegate);
    let files = paths(&[image.clone()]);
    let editor = match kind.as_str() {
        "crop" => crate::editor::EditorWindow::crop(mtm, files, d, false),
        "edit" => crate::editor::EditorWindow::adjust(mtm, files, d, false),
        _ => crate::editor::EditorWindow::frame(mtm, files, d, false),
    };
    let editor = editor.inspect(|e| {
        for (k, v) in &sets {
            e.set_adjustment(k, *v);
        }
    });
    match editor.and_then(|e| e.render_png(segment, std::path::Path::new(&out))) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
