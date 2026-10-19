//! Thin wrappers over AppKit: input sampling, pasteboards, feedback, file actions, alerts.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject};
use objc2::ClassType;
use objc2_app_kit::{
    NSAlert, NSApplication, NSEvent, NSEventModifierFlags, NSHapticFeedbackManager,
    NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime, NSHapticFeedbackPerformer,
    NSOpenPanel, NSPasteboard, NSPasteboardNameDrag, NSPasteboardTypeFileURL,
    NSPasteboardTypeString, NSPasteboardURLReadingFileURLsOnlyKey, NSPasteboardWriting, NSScreen,
    NSSharingService, NSSharingServiceNameSendViaAirDrop, NSSound,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSDictionary, NSNumber, NSPoint, NSRect, NSString, NSURL};
use satsuma_core::actions::{paths_as_text, terminal_directory};
use satsuma_core::naming::first_free;
use satsuma_core::Mods;
use std::path::{Path, PathBuf};

pub fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

// ---------------------------------------------------------------- input sampling

pub fn left_mouse_down() -> bool {
    NSEvent::pressedMouseButtons() & 1 != 0
}

pub fn mouse_location() -> NSPoint {
    NSEvent::mouseLocation()
}

pub fn mods_from_flags(flags: NSEventModifierFlags) -> Mods {
    let mut m = Mods::NONE;
    if flags.contains(NSEventModifierFlags::Shift) {
        m = m.union(Mods::SHIFT);
    }
    if flags.contains(NSEventModifierFlags::Control) {
        m = m.union(Mods::CONTROL);
    }
    if flags.contains(NSEventModifierFlags::Option) {
        m = m.union(Mods::OPTION);
    }
    if flags.contains(NSEventModifierFlags::Command) {
        m = m.union(Mods::COMMAND);
    }
    m
}

pub fn current_mods() -> Mods {
    mods_from_flags(NSEvent::modifierFlags_class())
}

pub fn drag_pasteboard() -> Retained<NSPasteboard> {
    NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag })
}

pub fn has_file_urls(pb: &NSPasteboard) -> bool {
    let file_type = unsafe { NSPasteboardTypeFileURL };
    pb.types()
        .map(|types| types.iter().any(|t| t.isEqualToString(file_type)))
        .unwrap_or(false)
}

/// File URLs on a pasteboard, as paths.
pub fn file_paths(pb: &NSPasteboard) -> Vec<PathBuf> {
    let class: &AnyClass = NSURL::class();
    let classes = NSArray::from_slice(&[class]);
    let yes = NSNumber::new_bool(true);
    let key = unsafe { NSPasteboardURLReadingFileURLsOnlyKey };
    let options = NSDictionary::from_slices(&[key], &[&*yes as &AnyObject]);
    let Some(objects) = (unsafe { pb.readObjectsForClasses_options(&classes, Some(&options)) }) else {
        return Vec::new();
    };
    objects
        .iter()
        .filter_map(|o| o.downcast::<NSURL>().ok())
        .filter_map(|u| u.to_file_path())
        .collect()
}

/// Visible frame of the screen containing `p` (falls back to the main screen).
pub fn screen_frame_for(p: NSPoint, mtm: MainThreadMarker) -> NSRect {
    for screen in NSScreen::screens(mtm).iter() {
        let f = screen.frame();
        if p.x >= f.origin.x && p.x <= f.origin.x + f.size.width && p.y >= f.origin.y && p.y <= f.origin.y + f.size.height {
            return screen.visibleFrame();
        }
    }
    NSScreen::mainScreen(mtm).map(|s| s.visibleFrame()).unwrap_or(NSRect::ZERO)
}

// ---------------------------------------------------------------- feedback

pub fn highlight_feedback() {
    if let Some(sound) = NSSound::soundNamed(&ns("Tink")) {
        sound.stop();
        sound.setVolume(0.25);
        sound.play();
    }
    let performer = NSHapticFeedbackManager::defaultPerformer();
    performer.performFeedbackPattern_performanceTime(
        NSHapticFeedbackPattern::Alignment,
        NSHapticFeedbackPerformanceTime::Now,
    );
}

// ---------------------------------------------------------------- alerts

pub fn activate(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

pub fn alert(mtm: MainThreadMarker, title: &str, message: &str) {
    activate(mtm);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&ns(title));
    alert.setInformativeText(&ns(message));
    alert.runModal();
}

// ---------------------------------------------------------------- file actions

fn urls(paths: &[PathBuf]) -> Vec<Retained<NSURL>> {
    paths.iter().filter_map(|p| NSURL::from_file_path(p)).collect()
}

pub fn copy_files(paths: &[PathBuf]) {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let urls = urls(paths);
    let writers: Vec<&ProtocolObject<dyn NSPasteboardWriting>> =
        urls.iter().map(|u| ProtocolObject::from_ref(&**u)).collect();
    pb.writeObjects(&NSArray::from_slice(&writers));
}

pub fn copy_paths(paths: &[PathBuf]) {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    pb.setString_forType(&ns(&paths_as_text(paths)), unsafe { NSPasteboardTypeString });
}

pub fn airdrop(mtm: MainThreadMarker, paths: &[PathBuf]) -> Result<(), String> {
    let service = NSSharingService::sharingServiceNamed(unsafe { NSSharingServiceNameSendViaAirDrop })
        .ok_or("AirDrop isn't available on this Mac.")?;
    activate(mtm);
    let urls = urls(paths);
    let items: Vec<&AnyObject> = urls.iter().map(|u| &***u as &AnyObject).collect();
    let array = NSArray::from_slice(&items);
    unsafe { service.performWithItems(&array) };
    Ok(())
}

pub fn open_terminal(path: &Path, bundle_id: &str, app_name: &str) -> Result<(), String> {
    let dir = terminal_directory(path, path.is_dir());
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-b", bundle_id])
        .arg(&dir)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{app_name} isn't installed. Choose another terminal app in the Satsuma menu."))
    }
}

pub fn choose_folder(mtm: MainThreadMarker, prompt: &str, message: &str) -> Option<PathBuf> {
    activate(mtm);
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(false);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    panel.setPrompt(Some(&ns(prompt)));
    panel.setMessage(Some(&ns(message)));
    if panel.runModal() != 1 {
        return None;
    }
    panel.URLs().iter().next().and_then(|u| u.to_file_path())
}

pub fn choose_files(mtm: MainThreadMarker) -> Vec<PathBuf> {
    activate(mtm);
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(true);
    panel.setPrompt(Some(&ns("Choose")));
    panel.setMessage(Some(&ns("Choose files to convert, edit or act on.")));
    if panel.runModal() != 1 {
        return Vec::new();
    }
    panel.URLs().iter().filter_map(|u| u.to_file_path()).collect()
}

/// Moves items into `dest`, renaming on collisions. Returns error messages.
pub fn move_into(paths: &[PathBuf], dest: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for src in paths {
        let name = src.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if src.parent() == Some(dest) {
            continue;
        }
        let (base, ext) = if src.is_dir() {
            (name.clone(), None)
        } else {
            let stem = satsuma_core::naming::stem_of(&name).to_string();
            let ext = name[stem.len()..].trim_start_matches('.').to_string();
            (stem, if ext.is_empty() { None } else { Some(ext) })
        };
        let target = dest.join(first_free(&base, ext.as_deref(), &|n| dest.join(n).exists()));
        if std::fs::rename(src, &target).is_ok() {
            continue;
        }
        // Different volume: copy, then remove the original.
        let copied = std::process::Command::new("/usr/bin/ditto").arg(src).arg(&target).status();
        match copied {
            Ok(s) if s.success() => {
                let removed = if src.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) };
                if let Err(e) = removed {
                    errors.push(format!("Copied {name} but couldn't remove the original: {e}"));
                }
            }
            _ => errors.push(format!("Couldn't move {name}.")),
        }
    }
    errors
}

pub fn main_thread() -> MainThreadMarker {
    MainThreadMarker::new().expect("must run on the main thread")
}
