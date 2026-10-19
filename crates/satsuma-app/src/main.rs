//! Satsuma: a drag-gesture file converter for the macOS menu bar.
//!
//! Without arguments it starts the menu-bar app; with arguments it is a command-line tool.

mod app;
mod cli;
#[cfg(feature = "demo")]
mod demo;
mod draw;
mod editor;
mod events;
mod float;
mod hud;
mod icon;
mod system;
mod wheel;

fn main() {
    // Finder may pass a process serial number argument on older systems.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn_")).collect();
    if args.is_empty() {
        app::run();
    } else {
        std::process::exit(cli::run(&args));
    }
}
