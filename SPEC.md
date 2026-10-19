# SPEC — Satsuma, a drag-gesture file converter for macOS

This spec describes the behavior of **Tangerine 2.0.2** (Leap Studio, `com.th.Tangerine`),
observed through REA and public material, and defines what **Satsuma**, our own Rust
implementation, does. It describes behavior only. No code or assets were copied.

---

## 0. Evidence

| ID | Source | What it shows |
|----|--------|---------------|
| E1 | `tangerineformac.com` landing page + `assets/home-BU_Sezi2.js` (sha256 `24a7b6d5…bcc8`, captured with `rea analyze-web-bundle`) | Interactive demo: format matrix, tool lists, wheel modes, key mapping, output naming, keyboard navigation, wheel geometry, announcement strings |
| E2 | `assets/tools-DHpyoIBf.js` (`/tools` page) | Tool groups, titles, descriptions, feature bullets |
| E3 | `videos/tangerine-landing-demo.mp4` (4 s, 3130×2160) | Real app: drag `sample.wav` on desktop → tap Shift → glass ring of 6 petals at cursor → hovered petal turns orange, centre shows label → drop → HUD "Converting to MP3 · 1 of 1 saved" with progress bar → new file appears beside original |
| E4 | `/Applications/Tangerine.app/Contents/Info.plist` | v2.0.2 (build 9), `LSUIElement = true` (menu-bar agent), macOS ≥ 13 |
| E5 | `codesign -d --entitlements` | App Sandbox, user-selected read-write, app-scope bookmarks. **No** Accessibility entitlement ⇒ drag detection works without accessibility permission |
| E6 | `otool -L` | Swift/SwiftUI + AppKit, AVFoundation, PDFKit, ImageIO, Vision, QuickLookThumbnailing, ServiceManagement, StoreKit, WebKit, JavaScriptCore |
| E7 | `Contents/Resources/Tools/{ffmpeg,cwebp}`, `ExifTool/`, `PagesWriter.js`, `OfficePreview.js`, `PowerPoint/`, `HighlightSound.wav` | A/V via bundled ffmpeg, WebP via cwebp, metadata via ExifTool, Office/Pages via JS helpers |
| E8 | `strings` of the Mach-O (38 293 strings) | Classes `DragGestureMonitor`, `WheelModifierSettings`, `DragModifierCombination`, `FloatingFilePanel`, `OperationProgressWindowController`, `MenuBarController`; selectors `dragPasteboard`, `changeCount`, `pressedMouseButtons`, `modifierFlags`, `mouseLocation`, `addGlobalMonitorForEventsMatchingMask:handler:`, `registerForDraggedTypes:`, `draggingEntered:`, `draggingUpdated:`, `draggingExited:`, `performDragOperation:`; settings keys `conversionWheelDragModifierMask`, `toolsWheelDragModifierMask`, `actionsWheelDragModifierMask`, `fileActionsTerminalApplication`, `defaultImageCompressionStrength`, `defaultVideoCompressionStrength`; ffmpeg args `-progress pipe:1 -nostats`, `-b:a 192k`, `-tag:v hvc1`, `-movflags +faststart`, `h264_videotoolbox`, `hevc_videotoolbox`; HUD strings "Converting …", "Finished with errors", "Tangerine is working", "Tangerine is ready" |
| E9 | App Store listing + press kit | 275 conversion paths, 26 tools, 6 actions, free 20 uses then $29.99 unlock, results saved beside originals |
| E10 | User-supplied screen recording of Tangerine 2.0 (21.5 s, 480×270) | `Page 001.pdf` → Shift → petals DOCX/JPG/PNG/TXT → drop on JPG → `Page 001.jpg` beside it. JPG → Shift → **PNG, WEBP, HEIC, TIFF, AVIF, BMP, PDF, DOCX**. Command → COMPRESS, METADATA, EDIT, ANNOTATE, ADD BG, CROP, REDACT; the centre shows the file size (**"1.2 MB"**) until a petal is highlighted. Drop on CROP → **Crop Image** window (handles, Free/1:1/16:9/9:16/4:3/3:4, W × H px, Reset, Apply) → `Page 001 Cropped.jpg`. Drop on ADD BG → **Add BG** window (gradient and solid swatches plus image picker; Padding, Corners and Shadow sliders in px; Ratio Auto/1:1/16:9/4:3; Reset; Save with Background) → `Page 001 Cropped With Background.png` |
| E11 | Binary strings (as E8) | `Save with Background`, `frameGradientStrip`, `frameSolidColorStrip`, `Choose image background`, `Choose a custom background color`. A built-in DOCX writer with inline images (`word/document.xml`, `wp:inline`, `a:blip r:embed`). "Recognize scanned text with OCR. Keep illustrations as inline images." Editors' save buttons: Save Edited/Annotated/Redacted/Bleeped Copy, Save Organized PDF, Export One Video, Create Visualization |

REA runtime UI observation (`observe-native-ui`) was blocked because Screen Recording is
not granted. Runtime visuals come from E3.

---

## 1. Concept

A file converter that lives in the **menu bar** and shows a **radial wheel at the pointer
while you drag files**:

1. The user starts dragging one or more files (from Finder, the desktop or any app).
2. While dragging, they **tap a modifier key**:
   - **Shift**: *Convert* wheel (output formats)
   - **Command**: *Tools* wheel (compress, metadata, split …)
   - **Control**: *Actions* wheel (AirDrop, Float, Copy, Move, Copy Path, Terminal)
3. The wheel appears **centred on the pointer** and **stays open after the key is released** (E1, E9).
4. Tapping another modifier **switches** the wheel in place. The anchor stays put.
5. Moving over a petal highlights it orange and shows its name in the centre.
   A short click sound and haptic tick play on each highlight change (E1 `highlight.wav`, E7, E8 "Sound and haptic feedback").
6. **Dropping on a petal** runs the operation. Dropping anywhere else
   (centre hole, outside the ring) cancels it, and the drag behaves as usual.
7. Results are **saved beside the originals** as new files. Originals are never modified (E9).

## 2. Wheel modes and key mapping

- Default mapping: Shift → conversions, Command → tools, Control → actions (E1 `K()`, E8).
- Each mapping is a configurable **modifier combination** (E8 `DragModifierCombination`,
  `didMigrateToolsWheelToCommand`, so tools used to be Option+Shift).
- A mode is triggered when the **exact** set of held modifiers equals a configured combination
  (E1: `size !== 1 → null`). Holding Shift+Command together matches nothing unless configured.
- Triggering happens on a **change** in held modifiers. If a combination is already held when
  the file drag is first detected, the wheel opens straight away (E1: `V(K(x))` on pointer down).
- Releasing modifiers never closes the wheel.
- The wheel closes when the drag ends: on drop, when the mouse button is released, on Escape, or when the window loses focus (E1 `blur`, `pointercancel`).

### 2.1 Empty wheels
- Conversions with no targets: message *"No conversions for this format. Tap Control for file actions."*
- Tools with no targets: *"No editing tools for this format. Tap Shift to convert or Control for file actions."*
- Actions are never empty. They apply to anything, folders included (E1, E9).

### 2.2 Keyboard/click mode (no drag)
Opened for a chosen file (E1: Enter on a file; in Satsuma: menu bar *Convert Files…*):
- Opens on the conversions wheel. If there are no conversions it opens on actions instead.
- Arrow Right/Down moves to the next petal, Left/Up to the previous one (both wrap). Home goes to the first petal and End to the last.
- Enter or Space applies the highlighted petal, Escape cancels and Tab closes.
- Shift, Command and Control switch wheels, as in drag mode.
- The first petal is pre-selected.
- Satsuma also lets you hover and click petals in this mode.

## 3. Wheel geometry (behavioral)

- *n* petals in a ring between an inner radius `r_in` and an outer radius `r_out`, with a constant gap
  between petals. Petal *i* is centred at angle
  `θ_i = θ_0 − 2π·i/n` (clockwise), where `θ_0 = π/2` (top), except `θ_0 = π` when `n = 3` (E1 `F()`).
- Hit test for pointer offset `(dx, dy)` from the anchor (y up):
  - `d = hypot(dx,dy)`. If `d < r_in` or `d > r_out`, there is no selection.
  - The nearest petal index comes from the angle. Points inside the gap
    (`|Δθ| > π/n − asin(gap / 2d)`) select nothing.
- Petals have rounded outer corners. Labels sit at radius `r_label + (r_out − r_label)·0.56` (E1 `Me()`).
- *Dense* styling (smaller labels) applies when n > 6.
- The anchor is clamped so the whole wheel stays on screen (E1 `he()`).
- Visual style (E3, E10): frosted glass disc, light petals, uppercase labels; the highlighted petal is tangerine
  orange (`#FF5200` family, E8 `colors=0xff5200`) with white text, and the centre label repeats the selection.
  Tools and actions petals show an SF Symbol above the label (E1 symbol names).
- When no petal is highlighted, the centre shows the dragged file's size, e.g. "1.2 MB" (E10). Satsuma
  shows "N ITEMS" above the total size for several files, and "FOLDER" for a folder.

## 4. Format matrix (conversions)

Let `IMG_IN = jpg png webp heic tiff svg avif bmp`, `IMG_OUT = jpg png webp heic tiff avif bmp pdf`,
`AUD = mp3 m4a wav flac ogg opus aiff wma`, `VID = mp4 mov mkv webm avi wmv`,
`SUB = srt vtt`, `DOC_IN = docx pages md rtf rtfd textClipping`, `SHEET = xlsx xls ods csv tsv`,
`DOC_OUT = docx pages pdf txt md html epub png jpg`.
**Input-only** formats (never offered as targets): `rar ppt xls rtf rtfd textClipping` (svg is in no output set).

| Input | Targets (excluding the input itself and input-only formats) |
|-------|---------|
| any of `IMG_IN` | `IMG_OUT` + `docx` (E10: recognised text in a Word document) |
| any of `AUD` | `AUD` |
| any of `VID` | `VID` + `gif`, `mp3` |
| `gif` | `VID` |
| `pdf` | docx pages pptx xlsx jpg png epub txt md |
| `ppt`, `pptx` | pdf jpg png |
| any of `SHEET` | `SHEET` + pdf |
| any of `DOC_IN` | `DOC_OUT` |
| `epub` | pdf docx pages txt md html |
| `txt` | docx pages pdf md html epub jpg png |
| `srt`, `vtt` | srt vtt txt |
| `zip tar gz rar` (`.tar.gz`/`.tgz` count as gz) | zip tar gz |
| anything else (folders too) | none |

Order matters: petals appear in the order listed (E1 `z()`).
Labels are upper case, except `gz` → **GZIP** (E1 `E()`).

The website matrix (E1) has exactly the advertised 275 paths. The 2.0 app adds image → DOCX for the 8
image inputs (E10), making 283.

- **Image → DOCX**: on-device text recognition (Vision) puts the recognised lines into editable
  paragraphs. An image with no text becomes a document holding the picture.
- **Scanned PDFs**: when a PDF has no selectable text, its pages are recognised with OCR for TXT, MD,
  DOCX and EPUB output (E11, E2 "Scanned pages use local OCR").

### 4.1 Multi-file selections (Satsuma rule; Tangerine shows "Batch drops of any size", E8)
Target *T* is offered when, for every file *f*, either `T ∈ targets(f)` or `T == format(f)`, and at
least one file actually needs converting. Files already in *T* are skipped. The petal order follows
the first file that needs converting.

## 5. Output naming

- Conversion: `<stem>.<ext>` beside the source, where `stem` drops a trailing `.tar.gz`, `.tgz` or the last
  extension, and `gz` writes `.tar.gz`. On collision: `<stem> 2.<ext>`, `<stem> 3.<ext>`, … (E1 `Ge()`).
- Tool: `<stem> <Label>.<ext>` and, on collision, `<stem> <Label> 2.<ext>` … (E1 `tt()`):

| Tool | Label | Output format |
|------|-------|---------------|
| compress | Compressed | same (wav → **flac**) |
| removeMetadata | No Metadata | same |
| normalizeAudio | Normalized | same |
| muteVideo | Muted | same |
| trimAudio | Trimmed | same |
| changeVideoSpeed | Speed 2x | same |
| videoSnapshots | Snapshot | png |
| splitVideo | Clips | folder |
| splitPDF | Pages | folder |
| extractArchive | Extracted | folder |
| audioToVideo | Visualizer | mp4 |
| audioChannels | Mono | same |
| organizePDF | Organized | pdf |
| frameImage (Add BG) | With Background | **png** (E10; the website demo said "Background") |
| editImage / annotateImage / cropImage / redactImage | Edited / Annotated (png) / Cropped / Redacted | same |
| trimVideo / cropVideo / redactVideo / redactAudio | Trimmed / Cropped / Redacted / Bleeped | same |

- Multi-file tools (Satsuma names): mergePDF → `Merged.pdf`, createPDF → `<first stem> Images.pdf`,
  joinVideos → `Joined.mp4`, createCollage → `Collage.png`. All go beside the first file, with the collision rule above.

## 6. Tools wheel

Per-format tool lists (E1 `et()`; `k` = number of files):

| Formats | k = 1 | k > 1 |
|---------|-------|-------|
| jpg png webp heic tiff | compress, removeMetadata, editImage, annotateImage, frameImage, cropImage, redactImage | compress, removeMetadata, cropImage, createPDF, createCollage |
| svg | — | createPDF, createCollage |
| mp3 m4a wav flac | compress, removeMetadata, normalizeAudio, audioToVideo, trimAudio, audioChannels, redactAudio | same list, filtered to batch-capable tools |
| mp4 mov mkv | compress, removeMetadata, muteVideo, trimVideo, cropVideo, changeVideoSpeed, videoSnapshots, splitVideo, redactVideo | compress, removeMetadata, muteVideo, trimVideo, cropVideo, changeVideoSpeed, joinVideos |
| gif | removeMetadata | removeMetadata |
| pdf | compress, removeMetadata, splitPDF, organizePDF | compress, removeMetadata, splitPDF, mergePDF |
| zip tar gz rar | extractArchive | extractArchive |

When k > 1, only **batch-capable** tools remain: compress, removeMetadata, cropImage, normalizeAudio,
muteVideo, splitPDF, extractArchive, createPDF, mergePDF, joinVideos, createCollage (E1 `Je`, `$e()`).
Mixed selections get the intersection of each format's list (Satsuma rule).

Titles and symbols (E1 `L`): COMPRESS `arrow.down.right.and.arrow.up.left`, METADATA `tag`,
EDIT `slider.horizontal.3`, ANNOTATE `pencil.and.outline`, ADD BG `photo.artframe`, CROP `crop`,
NORMALIZE `waveform`, MUTE `speaker.slash.fill`, TRIM `scissors`, SPLIT `rectangle.split.2x1`/`film`,
ORGANIZE `square.grid.2x2`, EXTRACT `archivebox`, CREATE PDF `doc.badge.plus`, MERGE PDF `doc.on.doc`,
SPEED `speedometer`, JOIN `film.stack`, SNAPSHOT `camera`, VISUALIZE `waveform`, CHANNELS `hifispeaker.2`,
COLLAGE `rectangle.3.group`, REDACT `eye.slash`, BLEEP `waveform.badge.minus`.

Tool behavior (E2, E8):
- **Compress**: presets *Balanced* ("Smaller files with less quality loss") and *Strong* ("Smaller files with more
  quality loss"), plus image size ("Keep original dimensions" or longest edge). Modes: preset, target file size,
  or percentage (1–99 %). The output must never be larger than the original
  ("Compressed output would exceed the original or target file size."). The original is untouched.
- **Metadata**: strip embedded tags (EXIF/GPS/XMP/IPTC for images, container tags for A/V, document info for PDF).
- **Normalize**: EBU R128 loudness; target −70…−5 LUFS, LRA 1…50 LU, true peak −9…0 dBTP.
- **Mute**: copy the video stream without audio.
- **Speed**: 0.25×–4× with pitch-preserving audio (wheel default 2×).
- **Snapshots**: export full-resolution PNG frames.
- **Split video**: equal parts or custom points (≥ 0.05 s apart), clips placed in a folder.
- **Join**: mixed MP4/MOV/MKV joined into one MP4. The first clip defines the canvas and others are letterboxed. Needs ≥ 2 videos.
- **Visualizer**: MP4 with an orange waveform on black, landscape/portrait/square.
- **Trim audio**: removes silence at both ends only, keeping inner pauses.
- **Channels**: mono/stereo.
- **Split PDF**: one PDF per page in a folder. **Merge PDFs**: in selection order. **Create PDF**: one page per image.
- **Collage**: grid/row/column/featured layout.
- **Extract**: ZIP/TAR/GZIP/RAR into a folder beside the archive.
- Interactive editors (edit/annotate/background/crop/redact/trim video/crop video/organize PDF/bleep/QR) open
  a window with a live preview before saving (E10, E11).

### 6.1 Crop Image window (E10)
- A titled window with a close button, the image preview and a crop rectangle. The outside is dimmed and the
  rectangle has eight orange handles (corners and edge midpoints).
- Dragging a handle resizes; dragging inside moves. The rectangle never leaves the image.
- **Aspect ratio** segmented control: Free, 1:1, 16:9, 9:16, 4:3, 3:4. Choosing a ratio fits the largest
  rectangle of that ratio, centred on the current crop. Handle drags then keep the ratio.
- **W × H px** fields show the crop size and accept exact sizes (example: 2550 × 3300 → 2550 × 1677 → 1:1 →
  2550 × 2550). In Satsuma, typing a size switches the preset back to Free.
- **Reset** restores the whole image. **Apply** saves `<name> Cropped.<ext>` in the same format.
- With several images, the same relative rectangle applies to each (website: "Crop one image or a selection").

### 6.2 Add Background window (E10, E11)
- A live preview of the photo on its new canvas, with rounded corners and a drop shadow.
- **Background**: a strip of gradients, an image picker ("Choose image background"), a strip of solid colours,
  and a custom colour.
- **Padding**, **Corners** and **Shadow** sliders, with values in output pixels (the recording shows
  527 px / 164 px / 24 px for a 2550 px page).
- **Ratio**: Auto (equal padding on every side), 1:1, 16:9, 4:3. With a fixed ratio, the canvas grows on the
  short side and the photo stays centred.
- **Reset** restores the defaults. **Save with Background** writes `<name> With Background.png`.
- Satsuma's defaults are padding 8 % of the long side, corners 3 % of the short side, and shadow 1.2 % of
  the long side. Its presets are its own: 9 gradients and 10 solids.

### 6.3 Edit Photo window (E2, E10, E11)
- Opened by the EDIT petal. "Adjust exposure, color, detail, effects, and more while previewing the result."
  The save button is **Save Edited Copy** (E11), which writes `<name> Edited.<ext>` in the same format.
- Adjustments are Tangerine's `ImageAdjustments` fields (E11): exposure, brightness, contrast, highlights,
  shadows, whites, blacks, temperature, tint, saturation, vibrance, gamma, clarity, texture,
  sharpness, dehaze, fade, vignette, grain, noise reduction. Tangerine renders them with CoreImage (E11:
  CIExposureAdjust, CIHighlightShadowAdjust, CITemperatureAndTint, CINoiseReduction,
  CISharpenLuminance, and CIRandomGenerator + CISoftLightBlendMode for grain).
- Satsuma groups them into Light / Color / Detail / Effects tabs ("light, color, and detail", E2). Bipolar
  sliders run −100…+100 and one-sided ones 0…100. A **Show Original** toggle compares with the original, and
  **Reset** zeroes every slider. The filter mappings and strengths are Satsuma's own.

## 7. Actions wheel (E1 `M`)

| Action | Behavior |
|--------|----------|
| AirDrop | Opens the macOS AirDrop recipient picker for the dropped items |
| Float | Keeps the items in a floating shelf that stays on top. Items can be dragged out into other apps and new items dropped in ("Drop to add"). Setting: add to the existing Float or create a new one |
| Copy | Puts file URLs on the clipboard, ready to paste in Finder |
| Move | Asks for a destination folder and moves the originals there |
| Copy Path | Copies full POSIX paths as text, one per line |
| Terminal | Opens the folder (or a file's parent folder) in Terminal or iTerm2 (setting). If the chosen terminal isn't installed, shows an error |

Actions work with folders and mixed selections.

## 8. Progress and results (E3, E8)

- A small HUD appears near where the drop happened, showing a title (`Converting to MP3`, or the tool's verb such as
  `Compressing`, `Extracting archive`, `Normalizing volume`), the subtitle `k of n saved`, a determinate progress
  bar and a cancel (×) button.
- When it finishes, the HUD fades out. If any file failed, it shows *Finished with errors* plus the first error message.
- The menu-bar item shows a working state while jobs run ("… is working" / "… is ready").
- ffmpeg progress comes from `-progress pipe:1 -nostats` (`out_time_us` against the input duration).

## 9. Menu bar and settings

- No Dock icon (`LSUIElement`). A status item with: state line, *Convert Files…*, settings, *Quit*.
- Settings (E8 keys): modifier combination per wheel; sound and haptic feedback on/off; image compression
  strength and size; video compression strength; terminal app; Float behavior; launch at login.
- First run: a welcome screen explaining "Step 1. Drag files. Step 2. While dragging, tap …".

## 10. Drag detection (mechanism, E5 + E8)

The approach needs no Accessibility permission:
1. Sample `NSEvent.pressedMouseButtons`, `NSEvent.modifierFlags`, `NSEvent.mouseLocation` and
   `NSPasteboard(name: .drag).changeCount` from a timer or from global mouse monitors.
2. A **file drag is active** when the left button is down, the drag pasteboard's change count differs from the
   value recorded while the button was up, and the pasteboard holds file URLs.
3. While a file drag is active, a modifier change that matches a configured combination opens (or switches)
   the wheel at `mouseLocation`.
4. The wheel is a borderless, non-activating, floating **NSPanel** registered for file-URL drags. Because it appears
   under the pointer mid-drag, the drag session delivers `draggingEntered:`/`draggingUpdated:` to it.
   `draggingUpdated:` hit-tests the petals and returns Copy over a petal, None otherwise.
   `performDragOperation:` reads the URLs and starts the job, and refuses drops off a petal so the drag is cancelled.
5. When the mouse button comes up, the wheel closes.

## 11. Edge cases

- Non-file drags (text, images from browsers without file URLs) never open a wheel.
- Same-format targets are never shown. A format with no targets shows the empty-wheel message.
- Name collisions never overwrite: they always take the next free ` N` suffix, including outputs created earlier in the same batch.
- `.tar.gz` and `.tgz` are treated as a single extension (gz).
- Failures leave no partial output: write to a temporary file in the destination folder, then rename.
- Cancel stops the running process and removes the partial file.
- Multi-file tools (merge/join/collage/create PDF) need at least 2 files, or at least 1 image for createPDF.
- Unreadable or inaccessible files: "Drop only accessible files" (E8).

---

## 12. Satsuma v1: scope of our implementation

Satsuma reproduces the gesture UX, the format matrix, the naming rules and the non-interactive tools.
Conversions run locally with macOS system tools (`sips`, `textutil`, `tar`, PDFKit/CoreGraphics/CoreText)
and a user-installed **ffmpeg** (`brew install ffmpeg`, found on `PATH`, `/opt/homebrew/bin`,
`/usr/local/bin` or `$SATSUMA_FFMPEG`). Targets whose backend is missing are hidden from the wheel.

**Implemented conversions**
- Images: every `IMG_IN` → `IMG_OUT` (sips; WebP output via ffmpeg/libwebp).
- Audio ↔ audio, video ↔ video, video → GIF/MP3, GIF → video (ffmpeg).
- Documents: docx/md/txt/rtf/rtfd/textClipping/html/epub → docx/txt/md/html/epub/pdf/png/jpg
  (textutil, a Markdown renderer, our own EPUB writer and reader, CoreText PDF pagination, and PDF page rendering).
- PDF → txt/md/docx/epub/jpg/png (PDFKit text extraction and CoreGraphics rendering at 300 DPI; several pages go into a folder).
- Spreadsheets: xlsx/xls/ods/csv/tsv → xlsx/ods/csv/tsv/pdf (first sheet).
- Subtitles: srt ↔ vtt, → txt.
- Archives: zip/tar/gz/rar → zip/tar/gz.

- Images → DOCX with Vision OCR; scanned PDFs → TXT/MD/DOCX/EPUB with OCR.

**Implemented tools**: compress (image/audio/video/PDF, Balanced/Strong presets, image longest-edge option),
removeMetadata (jpg/png/webp lossless chunk stripping, gif, audio/video, PDF), normalizeAudio,
muteVideo, trimAudio (silence at ends), audioChannels (mono), changeVideoSpeed (2×),
videoSnapshots (middle frame), splitVideo (equal parts, default 2), audioToVideo, splitPDF, mergePDF,
createPDF, joinVideos, createCollage (grid), extractArchive, plus the **Crop Image** (§6.1),
**Add Background** (§6.2) and **Edit Photo** (§6.3) editor windows. The command line takes their
settings as options.

**Implemented actions**: all six.

**Still missing (compared with Tangerine 2.0)**:
- Editor windows not built yet: Annotate Photo (arrows, shapes, text, callouts), Redact Photo (face/text detection, blur/solid), Trim Video, Crop
  Video, Redact Video, Organize PDF (reorder/rotate/duplicate/remove pages), Bleep Audio, Read QR Code.
- Options windows that Tangerine shows before running a tool. Satsuma runs these with defaults from its
  menu instead:
  - Compress: target size, percentage
  - Normalize: LUFS, LRA, true peak
  - Trim silence: threshold
  - Split video: custom points
  - Speed: 0.25–4×
  - Channels: left/right balance
  - Visualizer: style and orientation
  - Collage: layouts
  - Join: clip ordering
- Formats: Pages and PowerPoint input and output, PDF → XLSX/PPTX.
- Smaller items: metadata editing (Tangerine edits tags; Satsuma only removes them), metadata removal for
  HEIC/TIFF/AVIF, per-format conversion defaults ("Conversion Settings"), the wheel colour theme, the
  editor zoom-in animation, and the onboarding window.
- Licensing/paywall: not wanted.

**Satsuma-specific choices**
- Settings live in `~/Library/Application Support/Satsuma/settings.json` and are edited from the menu-bar menu.
- The app isn't sandboxed, so there's no folder-access onboarding.
- A command-line interface on the same binary: `satsuma targets|convert|tool|formats|render-wheel`.
