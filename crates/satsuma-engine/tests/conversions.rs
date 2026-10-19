//! End-to-end conversions on generated fixtures, using the real system tools.
//!
//! Media tests need ffmpeg; they are skipped (with a note) when it is not installed.

use satsuma_core::settings::Settings;
use satsuma_core::{Format, Tool};
use satsuma_engine::probe::probe;
use satsuma_engine::{pdf, plan, run_job, Cancel, Context, Env, Operation};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(Env::discover)
}

fn ffmpeg_or_skip() -> bool {
    if env().has_ffmpeg() {
        true
    } else {
        eprintln!("ffmpeg not installed; skipping media test");
        false
    }
}

/// Plans and runs `op` on `inputs`, returning every published output.
fn apply(inputs: &[PathBuf], op: Operation) -> Vec<PathBuf> {
    let settings = Settings::default();
    let cancel = Cancel::new();
    let ctx = Context { env: env(), settings: &settings, cancel: &cancel };
    let p = plan(env(), inputs, op.clone()).expect("plan");
    assert!(!p.jobs.is_empty(), "nothing to do for {op:?}: {:?}", p.skipped);
    p.jobs
        .iter()
        .map(|job| run_job(&ctx, job, &mut |_| {}).unwrap_or_else(|e| panic!("{op:?} on {:?}: {e}", job.inputs)))
        .collect()
}

fn convert(input: &Path, to: Format) -> PathBuf {
    apply(&[input.to_path_buf()], Operation::Convert(to)).remove(0)
}

fn tool(inputs: &[PathBuf], t: Tool) -> PathBuf {
    apply(inputs, Operation::Tool(t)).remove(0)
}

fn ffmpeg(args: &[&str]) {
    let status = Command::new(env().ffmpeg.as_ref().unwrap())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).unwrap().len()
}

fn no_partials_left(dir: &Path) {
    for e in std::fs::read_dir(dir).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with(".satsuma-partial"), "left behind {name}");
    }
}

fn sample_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let p = dir.join(name);
    if env().has_ffmpeg() {
        ffmpeg(&["-f", "lavfi", "-i", &format!("testsrc2=s={w}x{h}"), "-frames:v", "1", p.to_str().unwrap()]);
    } else {
        // A tiny valid PNG via sips from a generated TIFF is overkill; use a known 1x1 PNG.
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\0\x05\0\x01\xff\x89\x99=\x1d\0\0\0\0IEND\xaeB`\x82";
        std::fs::write(&p, png).unwrap();
    }
    p
}

#[test]
fn images_convert_to_every_output_format() {
    let dir = tempfile::tempdir().unwrap();
    let png = sample_png(dir.path(), "photo.png", 320, 200);
    for target in [Format::Jpg, Format::Heic, Format::Tiff, Format::Avif, Format::Bmp, Format::Pdf, Format::Webp] {
        if target == Format::Webp && !env().has_encoder("libwebp") {
            continue;
        }
        let out = convert(&png, target);
        assert_eq!(out.file_name().unwrap().to_string_lossy(), format!("photo.{}", target.extension()));
        assert!(size(&out) > 0, "{target:?}");
    }
    // HEIC back to JPG collides with the earlier photo.jpg.
    let jpg2 = convert(&dir.path().join("photo.heic"), Format::Jpg);
    assert_eq!(jpg2.file_name().unwrap(), "photo 2.jpg");
    no_partials_left(dir.path());
}

#[test]
fn svg_input_renders() {
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("logo.svg");
    std::fs::write(
        &svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="#ff7a1a"/><circle cx="60" cy="40" r="30" fill="#fff"/></svg>"##,
    )
    .unwrap();
    let out = convert(&svg, Format::Png);
    assert!(size(&out) > 100);
}

#[test]
fn image_tools() {
    if !ffmpeg_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let a = sample_png(dir.path(), "a.png", 640, 480);
    let b = sample_png(dir.path(), "b.png", 300, 500);
    let c = sample_png(dir.path(), "c.png", 500, 300);

    // Collage of three → 2x2 grid.
    let collage = tool(&[a.clone(), b.clone(), c.clone()], Tool::CreateCollage);
    assert_eq!(collage.file_name().unwrap(), "Collage.png");
    let p = probe(env(), &collage).unwrap();
    assert_eq!(p.video, Some((2 * 800 + 16 + 32, 2 * 800 + 16 + 32)));

    // PDF with one page per image.
    let pdf_out = tool(&[a.clone(), b.clone()], Tool::CreatePdf);
    assert_eq!(pdf_out.file_name().unwrap(), "a Images.pdf");
    assert_eq!(pdf::page_count(&pdf_out).unwrap(), 2);

    // Compression shrinks a JPEG and keeps the original.
    let jpg = convert(&a, Format::Jpg);
    let before = size(&jpg);
    let small = tool(&[jpg.clone()], Tool::Compress);
    assert_eq!(small.file_name().unwrap(), "a Compressed.jpg");
    assert!(size(&small) < before);
    assert_eq!(size(&jpg), before);

    // PNG quantisation shrinks a photo-like PNG.
    let png_small = tool(&[a.clone()], Tool::Compress);
    assert!(size(&png_small) < size(&a), "{} vs {}", size(&png_small), size(&a));
}

#[test]
fn jpeg_metadata_is_removed_losslessly() {
    let dir = tempfile::tempdir().unwrap();
    let png = sample_png(dir.path(), "shot.png", 64, 64);
    let jpg = convert(&png, Format::Jpg);
    // Inject a comment segment with private text right after SOI.
    let mut data = std::fs::read(&jpg).unwrap();
    let mut com = vec![0xFF, 0xFE, 0x00, 0x10];
    com.extend_from_slice(b"GPS 52.1,4.3 !");
    data.splice(2..2, com);
    std::fs::write(&jpg, &data).unwrap();
    let clean = tool(&[jpg.clone()], Tool::RemoveMetadata);
    assert_eq!(clean.file_name().unwrap(), "shot No Metadata.jpg");
    let out = std::fs::read(&clean).unwrap();
    assert!(!out.windows(3).any(|w| w == b"GPS"));
    // Still a decodable JPEG.
    let png2 = dir.path().join("check.png");
    let st = Command::new("/usr/bin/sips").args(["-s", "format", "png"]).arg(&clean).arg("--out").arg(&png2).output().unwrap();
    assert!(st.status.success() && png2.exists());
}

fn sine_wav(dir: &Path, name: &str, seconds: f32, layout: &str) -> PathBuf {
    let p = dir.join(name);
    ffmpeg(&[
        "-f", "lavfi", "-i", &format!("sine=frequency=440:duration={seconds}:sample_rate=44100"),
        "-ac", if layout == "mono" { "1" } else { "2" },
        "-metadata", "title=Secret Title",
        p.to_str().unwrap(),
    ]);
    p
}

#[test]
fn audio_conversions_and_tools() {
    if !ffmpeg_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = sine_wav(dir.path(), "sample.wav", 2.0, "stereo");
    for target in satsuma_core::matrix::conversion_targets(Format::Wav) {
        if !satsuma_engine::capability::conversion_available(env(), Format::Wav, target) {
            continue;
        }
        let out = convert(&wav, target);
        let p = probe(env(), &out).unwrap();
        assert!(p.audio.is_some(), "{target:?}");
        let d = p.duration.unwrap();
        assert!((d - 2.0).abs() < 0.2, "{target:?} duration {d}");
    }
    let mp3 = dir.path().join("sample.mp3");

    // Compress: WAV → FLAC, smaller.
    let flac = tool(&[wav.clone()], Tool::Compress);
    assert_eq!(flac.file_name().unwrap(), "sample Compressed.flac");
    assert!(size(&flac) < size(&wav));

    // Channels: stereo → mono, mono → stereo.
    let mono = tool(&[wav.clone()], Tool::AudioChannels);
    assert_eq!(mono.file_name().unwrap(), "sample Mono.wav");
    assert_eq!(probe(env(), &mono).unwrap().audio.unwrap().channels, 1);
    let stereo = tool(&[mono.clone()], Tool::AudioChannels);
    assert!(stereo.file_name().unwrap().to_string_lossy().ends_with("Stereo.wav"));
    assert_eq!(probe(env(), &stereo).unwrap().audio.unwrap().channels, 2);

    // Normalize keeps the sample rate.
    let norm = tool(&[mp3.clone()], Tool::NormalizeAudio);
    assert_eq!(probe(env(), &norm).unwrap().audio.unwrap().sample_rate, 44100);

    // Trim silence: 1 s silence + 1 s tone + 1 s silence → about 1 s.
    let padded = dir.path().join("padded.wav");
    ffmpeg(&[
        "-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono:d=1",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=1:sample_rate=44100",
        "-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono:d=1",
        "-filter_complex", "[0][1][2]concat=n=3:v=0:a=1",
        padded.to_str().unwrap(),
    ]);
    let trimmed = tool(&[padded], Tool::TrimAudio);
    let d = probe(env(), &trimmed).unwrap().duration.unwrap();
    assert!((d - 1.0).abs() < 0.15, "trimmed to {d}");

    // Metadata removal.
    let clean = tool(&[mp3.clone()], Tool::RemoveMetadata);
    let (_, _, stderr) = satsuma_engine::process::capture(
        Command::new(env().ffmpeg.as_ref().unwrap()).args(["-hide_banner", "-i"]).arg(&clean),
    )
    .unwrap();
    assert!(!stderr.contains("Secret Title"), "{stderr}");

    // Visualizer → MP4 with video and audio.
    let viz = tool(&[mp3], Tool::AudioToVideo);
    let p = probe(env(), &viz).unwrap();
    assert_eq!(p.video, Some((1280, 720)));
    assert!(p.audio.is_some());
    no_partials_left(dir.path());
}

fn test_video(dir: &Path, name: &str, size: &str, seconds: u32, audio: bool) -> PathBuf {
    let p = dir.join(name);
    let mut args = vec![
        "-f".to_string(), "lavfi".into(), "-i".into(), format!("testsrc2=s={size}:d={seconds}:r=30"),
    ];
    if audio {
        args.extend(["-f".into(), "lavfi".into(), "-i".into(), format!("sine=frequency=330:duration={seconds}")]);
    }
    // High quality so compression has something to save.
    args.extend(["-c:v".into(), "mpeg4".into(), "-q:v".into(), "2".into()]);
    if audio {
        args.extend(["-c:a".into(), "aac".into(), "-shortest".into()]);
    }
    args.push(p.to_str().unwrap().into());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    ffmpeg(&refs);
    p
}

#[test]
fn video_conversions_and_tools() {
    if !ffmpeg_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mov = test_video(dir.path(), "weekend.mov", "320x240", 2, true);

    for target in [Format::Mp4, Format::Mkv, Format::Webm, Format::Avi, Format::Wmv, Format::Gif, Format::Mp3] {
        if !satsuma_engine::capability::conversion_available(env(), Format::Mov, target) {
            continue;
        }
        let out = convert(&mov, target);
        let p = probe(env(), &out).unwrap();
        if target == Format::Mp3 {
            assert!(p.video.is_none() && p.audio.is_some());
        } else {
            assert_eq!(p.video, Some((320, 240)), "{target:?}");
        }
    }
    // GIF back to video.
    let gif = dir.path().join("weekend.gif");
    let from_gif = convert(&gif, Format::Mp4);
    assert_eq!(from_gif.file_name().unwrap(), "weekend 2.mp4");

    let mute = tool(&[mov.clone()], Tool::MuteVideo);
    assert!(probe(env(), &mute).unwrap().audio.is_none());

    let fast = tool(&[mov.clone()], Tool::ChangeVideoSpeed);
    assert_eq!(fast.file_name().unwrap(), "weekend Speed 2x.mov");
    let d = probe(env(), &fast).unwrap().duration.unwrap();
    assert!((d - 1.0).abs() < 0.2, "2x duration {d}");

    let snap = tool(&[mov.clone()], Tool::VideoSnapshots);
    assert_eq!(snap.file_name().unwrap(), "weekend Snapshot.png");
    assert_eq!(probe(env(), &snap).unwrap().video, Some((320, 240)));

    let clips = tool(&[mov.clone()], Tool::SplitVideo);
    assert!(clips.is_dir());
    let mut names: Vec<_> = std::fs::read_dir(&clips).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    assert_eq!(names, ["weekend 01.mov", "weekend 02.mov"]);

    let compressed = tool(&[mov.clone()], Tool::Compress);
    assert!(size(&compressed) < size(&mov));

    // Join a clip with audio and a differently sized clip without audio.
    let silent = test_video(dir.path(), "silent.mp4", "200x400", 1, false);
    let joined = tool(&[mov.clone(), silent], Tool::JoinVideos);
    assert_eq!(joined.file_name().unwrap(), "Joined.mp4");
    let p = probe(env(), &joined).unwrap();
    assert_eq!(p.video, Some((320, 240)));
    assert!((p.duration.unwrap() - 3.0).abs() < 0.3, "{:?}", p.duration);
    no_partials_left(dir.path());
}

#[test]
fn cancelling_leaves_no_partial_file() {
    if !ffmpeg_or_skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mov = test_video(dir.path(), "long.mov", "640x480", 30, true);
    let settings = Settings::default();
    let cancel = Cancel::new();
    let ctx = Context { env: env(), settings: &settings, cancel: &cancel };
    let p = plan(env(), &[mov], Operation::Convert(Format::Webm)).unwrap();
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        c2.cancel();
    });
    let err = run_job(&ctx, &p.jobs[0], &mut |_| {}).unwrap_err();
    assert!(matches!(err, satsuma_engine::Error::Cancelled));
    assert!(!p.jobs[0].output.exists());
    no_partials_left(dir.path());
}

const MARKDOWN: &str = "# Field Notes\n\nThe **tangerine** grove was *quiet*.\n\n- first item\n- second item\n\n| Tree | Fruit |\n|---|---|\n| A | 12 |\n";

#[test]
fn markdown_to_every_document_format() {
    let dir = tempfile::tempdir().unwrap();
    let md = dir.path().join("notes.md");
    std::fs::write(&md, MARKDOWN).unwrap();

    let html = convert(&md, Format::Html);
    assert!(std::fs::read_to_string(&html).unwrap().contains("<h1>Field Notes</h1>"));

    let txt = convert(&md, Format::Txt);
    let t = std::fs::read_to_string(&txt).unwrap();
    assert!(t.contains("Field Notes") && t.contains("tangerine"), "{t}");

    let docx = convert(&md, Format::Docx);
    assert!(size(&docx) > 1000);

    let pdf_out = convert(&md, Format::Pdf);
    let text = pdf::text(&pdf_out).unwrap();
    assert!(text.contains("Field Notes") && text.contains("tangerine"), "{text}");

    let epub = convert(&md, Format::Epub);
    let (title, body) = satsuma_engine::epub::read(&epub).unwrap();
    assert_eq!(title, "notes");
    assert!(body.contains("<h1>Field Notes</h1>"), "{body}");

    let png = convert(&md, Format::Png);
    assert!(png.is_file() && size(&png) > 1000);

    // DOCX back to Markdown keeps the words and emphasis.
    let back = convert(&docx, Format::Md);
    let m = std::fs::read_to_string(&back).unwrap();
    assert!(m.contains("Field Notes") && m.contains("**tangerine**"), "{m}");

    // EPUB → PDF and TXT.
    assert!(pdf::text(&convert(&epub, Format::Pdf)).unwrap().contains("quiet"));
    assert!(std::fs::read_to_string(convert(&epub, Format::Txt)).unwrap().contains("second item"));
    no_partials_left(dir.path());
}

#[test]
fn long_rich_text_paginates_with_colours() {
    let dir = tempfile::tempdir().unwrap();
    let rtf = dir.path().join("story.rtf");
    let mut body = String::from("{\\rtf1\\ansi{\\fonttbl\\f0 Helvetica;}{\\colortbl;\\red255\\green80\\blue0;}\\f0\\fs24 ");
    for i in 0..200 {
        body.push_str(&format!("\\cf1 Paragraph {i}\\cf0  of a long story about citrus trees.\\par "));
    }
    body.push('}');
    std::fs::write(&rtf, body).unwrap();
    let out = convert(&rtf, Format::Pdf);
    let pages = pdf::page_count(&out).unwrap();
    assert!(pages >= 4, "only {pages} pages");
    let text = pdf::text(&out).unwrap();
    assert!(text.contains("Paragraph 0") && text.contains("Paragraph 199"));
}

#[test]
fn pdf_conversions_and_tools() {
    let dir = tempfile::tempdir().unwrap();
    let md = dir.path().join("report.md");
    let mut long = String::from("# Report\n\n");
    for i in 0..120 {
        long.push_str(&format!("Line {i} of the quarterly orange report.\n\n"));
    }
    std::fs::write(&md, long).unwrap();
    let pdf_in = convert(&md, Format::Pdf);
    let pages = pdf::page_count(&pdf_in).unwrap();
    assert!(pages >= 2);

    assert!(std::fs::read_to_string(convert(&pdf_in, Format::Txt)).unwrap().contains("Line 119"));
    assert!(std::fs::read_to_string(convert(&pdf_in, Format::Md)).unwrap().contains("quarterly orange"));
    assert!(size(&convert(&pdf_in, Format::Docx)) > 1000);
    assert!(size(&convert(&pdf_in, Format::Epub)) > 500);

    // Multi-page PDF → folder of page images at 300 DPI.
    let images = convert(&pdf_in, Format::Jpg);
    assert!(images.is_dir(), "{images:?}");
    let count = std::fs::read_dir(&images).unwrap().count();
    assert_eq!(count, pages);

    let split = tool(&[pdf_in.clone()], Tool::SplitPdf);
    assert_eq!(split.file_name().unwrap(), "report Pages");
    assert_eq!(std::fs::read_dir(&split).unwrap().count(), pages);

    let first_page = split.join("report 01.pdf");
    let merged = tool(&[first_page.clone(), pdf_in.clone()], Tool::MergePdf);
    assert_eq!(pdf::page_count(&merged).unwrap(), pages + 1);

    let clean = tool(&[pdf_in.clone()], Tool::RemoveMetadata);
    assert_eq!(pdf::page_count(&clean).unwrap(), pages);
    no_partials_left(dir.path());
}

#[test]
fn spreadsheets() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("budget.csv");
    std::fs::write(&csv, "Item,Cost\nRent,1200\nCoffee,12.5\n").unwrap();
    let xlsx = convert(&csv, Format::Xlsx);
    let ods = convert(&csv, Format::Ods);
    let tsv = convert(&xlsx, Format::Tsv);
    assert_eq!(std::fs::read_to_string(&tsv).unwrap(), "Item\tCost\nRent\t1200\nCoffee\t12.5\n");
    let csv2 = convert(&ods, Format::Csv);
    assert_eq!(csv2.file_name().unwrap(), "budget 2.csv");
    assert_eq!(std::fs::read_to_string(&csv2).unwrap(), "Item,Cost\nRent,1200\nCoffee,12.5\n");
    let pdf_out = convert(&xlsx, Format::Pdf);
    assert!(pdf::text(&pdf_out).unwrap().contains("Coffee"));
}

#[test]
fn subtitles_and_text_clippings() {
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("captions.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld\n").unwrap();
    let vtt = convert(&srt, Format::Vtt);
    assert!(std::fs::read_to_string(&vtt).unwrap().starts_with("WEBVTT"));
    assert_eq!(std::fs::read_to_string(convert(&srt, Format::Txt)).unwrap(), "Hello\nWorld\n");

    let clip = dir.path().join("Snippet.textClipping");
    let mut utis = plist::Dictionary::new();
    utis.insert("public.utf8-plain-text".into(), plist::Value::String("Clipped words".into()));
    let mut root = plist::Dictionary::new();
    root.insert("UTI-Data".into(), plist::Value::Dictionary(utis));
    plist::Value::Dictionary(root).to_file_binary(&clip).unwrap();
    let txt = convert(&clip, Format::Txt);
    assert!(std::fs::read_to_string(&txt).unwrap().contains("Clipped words"));
}

#[test]
fn archives_convert_and_extract() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("little-project");
    std::fs::create_dir_all(src.join("docs")).unwrap();
    std::fs::write(src.join("readme.txt"), "hi").unwrap();
    std::fs::write(src.join("docs/a.md"), "# a").unwrap();
    let zip = dir.path().join("little-project.zip");
    let st = Command::new("/usr/bin/ditto").args(["-c", "-k", "--keepParent"]).arg(&src).arg(&zip).status().unwrap();
    assert!(st.success());

    let tgz = convert(&zip, Format::Gz);
    assert_eq!(tgz.file_name().unwrap(), "little-project.tar.gz");
    let tar = convert(&tgz, Format::Tar);
    assert_eq!(tar.file_name().unwrap(), "little-project.tar");
    let zip2 = convert(&tar, Format::Zip);
    assert_eq!(zip2.file_name().unwrap(), "little-project 2.zip");
    let again = tool(&[zip2], Tool::ExtractArchive);
    assert_eq!(again.file_name().unwrap(), "little-project 2 Extracted");

    let out = tool(&[zip.clone()], Tool::ExtractArchive);
    assert_eq!(out.file_name().unwrap(), "little-project Extracted");
    assert_eq!(std::fs::read_to_string(out.join("little-project/docs/a.md")).unwrap(), "# a");
    // No AppleDouble files sneaked in.
    let listing = Command::new("/usr/bin/tar").arg("-tf").arg(&tgz).output().unwrap();
    assert!(!String::from_utf8_lossy(&listing.stdout).contains("._"));
    no_partials_left(dir.path());
}

#[test]
fn crop_and_add_background_editors() {
    use satsuma_core::edit::{Background, CropSpec, FrameSpec, Rect};
    let dir = tempfile::tempdir().unwrap();
    let jpg = dir.path().join("Page 001.jpg");
    let png = sample_png(dir.path(), "page.png", 256, 330);
    convert(&png, Format::Jpg);
    std::fs::rename(dir.path().join("page.jpg"), &jpg).unwrap();
    let size = satsuma_engine::imaging::size(&satsuma_engine::imaging::load(&jpg).unwrap());
    assert_eq!(size, (256.0, 330.0));

    // E10: free crop of the bottom, then 1:1 → "Page 001 Cropped.jpg".
    let spec = CropSpec::from_pixels(Rect::new(0.0, 0.0, 256.0, 256.0), size, None);
    let cropped = apply(&[jpg.clone()], Operation::Crop(spec)).remove(0);
    assert_eq!(cropped.file_name().unwrap(), "Page 001 Cropped.jpg");
    assert_eq!(satsuma_engine::imaging::size(&satsuma_engine::imaging::load(&cropped).unwrap()), (256.0, 256.0));

    // Exact output size.
    let sized = CropSpec::from_pixels(Rect::new(0.0, 0.0, 256.0, 256.0), size, Some((100, 100)));
    let small = apply(&[jpg.clone()], Operation::Crop(sized)).remove(0);
    assert_eq!(small.file_name().unwrap(), "Page 001 Cropped 2.jpg");
    assert_eq!(satsuma_engine::imaging::size(&satsuma_engine::imaging::load(&small).unwrap()), (100.0, 100.0));

    // Add Background → "… With Background.png", square canvas.
    let mut frame = FrameSpec::defaults_for((256.0, 256.0));
    frame.padding = 40.0;
    frame.ratio = Some((1, 1));
    let framed = apply(&[cropped.clone()], Operation::Frame(frame.clone())).remove(0);
    assert_eq!(framed.file_name().unwrap(), "Page 001 Cropped With Background.png");
    assert_eq!(satsuma_engine::imaging::size(&satsuma_engine::imaging::load(&framed).unwrap()), (336.0, 336.0));

    // Image backgrounds and solid colours render too.
    frame.background = Background::Image(png.clone());
    assert!(apply(&[cropped.clone()], Operation::Frame(frame.clone())).remove(0).exists());
    frame.background = Background::Solid { r: 20, g: 150, b: 140 };
    assert!(apply(&[cropped], Operation::Frame(frame)).remove(0).exists());
    no_partials_left(dir.path());
}

/// A PNG of rendered text, via our own Markdown → PNG path.
fn text_image(dir: &Path) -> PathBuf {
    let md = dir.join("scan.md");
    std::fs::write(&md, "# Harvest Report\n\nThe satsuma orchard produced forty crates this week.\n").unwrap();
    convert(&md, Format::Png)
}

#[test]
fn images_become_word_documents_with_recognised_text() {
    let dir = tempfile::tempdir().unwrap();
    let png = text_image(dir.path());
    let docx = convert(&png, Format::Docx);
    assert_eq!(docx.file_name().unwrap(), "scan.docx");
    let txt = dir.path().join("check.txt");
    let st = Command::new("/usr/bin/textutil").args(["-convert", "txt", "-output"]).arg(&txt).arg(&docx).status().unwrap();
    assert!(st.success());
    let text = std::fs::read_to_string(&txt).unwrap();
    assert!(text.contains("Harvest Report") && text.contains("forty crates"), "{text}");

    // A picture without text is embedded instead.
    let blank = dir.path().join("blank.png");
    let st = Command::new("/usr/bin/sips").args(["-s", "format", "png", "-z", "64", "64"]).arg(sample_png(dir.path(), "x.png", 64, 64)).arg("--out").arg(&blank).output().unwrap();
    assert!(st.status.success());
    // Flatten to a single colour so there is nothing to read.
    let data = std::fs::read(&blank).unwrap();
    let img = satsuma_engine::imaging::crop(&satsuma_engine::imaging::load(&blank).unwrap(), satsuma_core::edit::Rect::new(0.0, 0.0, 2.0, 2.0), Some((64, 64))).unwrap();
    satsuma_engine::imaging::write(&img, &blank, Format::Png).unwrap();
    assert_ne!(std::fs::read(&blank).unwrap(), data);
    let pic = convert(&blank, Format::Docx);
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&pic).unwrap()).unwrap();
    assert!(zip.by_name("word/media/image1.png").is_ok());
}

#[test]
fn scanned_pdfs_are_read_with_ocr() {
    let dir = tempfile::tempdir().unwrap();
    let png = text_image(dir.path());
    // sips writes an image-only PDF (no text layer).
    let scanned = dir.path().join("scanned.pdf");
    let st = Command::new("/usr/bin/sips").args(["-s", "format", "pdf"]).arg(&png).arg("--out").arg(&scanned).output().unwrap();
    assert!(st.status.success());
    assert!(pdf::text(&scanned).unwrap().trim().is_empty());
    let txt = convert(&scanned, Format::Txt);
    let text = std::fs::read_to_string(&txt).unwrap();
    assert!(text.contains("Harvest Report") && text.contains("satsuma orchard"), "{text}");
}

#[test]
fn edit_photo_saves_an_edited_copy() {
    use satsuma_core::edit::Adjustments;
    let dir = tempfile::tempdir().unwrap();
    let png = sample_png(dir.path(), "photo.png", 320, 200);
    let jpg = convert(&png, Format::Jpg);
    let mut adj = Adjustments::default();
    adj.set(Adjustments::index("exposure").unwrap(), 40.0);
    adj.set(Adjustments::index("vignette").unwrap(), 60.0);
    adj.set(Adjustments::index("grain").unwrap(), 30.0);
    let out = apply(&[jpg.clone()], Operation::Adjust(adj)).remove(0);
    assert_eq!(out.file_name().unwrap(), "photo Edited.jpg");
    let img = satsuma_engine::imaging::load(&out).unwrap();
    assert_eq!(satsuma_engine::imaging::size(&img), (320.0, 200.0));
    assert_ne!(std::fs::read(&out).unwrap(), std::fs::read(&jpg).unwrap());
    // HEIC keeps its format.
    let heic = convert(&png, Format::Heic);
    let edited = apply(&[heic], Operation::Adjust(adj)).remove(0);
    assert_eq!(edited.file_name().unwrap(), "photo Edited.heic");
    no_partials_left(dir.path());
}
