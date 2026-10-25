#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod convert;
mod debugger;
mod insight;
mod runner;
mod search;
mod vcs;
#[cfg(test)]
mod vcs_tests;
#[cfg(test)]
mod runner_tests;
#[cfg(test)]
mod debugger_tests;
#[cfg(test)]
mod insight_tests;
#[cfg(test)]
mod screens;
mod docs;
mod native_menu;
mod open_files;
mod toolwindow;
mod ui;

fn main() -> eframe::Result {
    let mut options = eframe::NativeOptions { viewport: rc_kit::window_chrome::viewport(), ..Default::default() };
    // Window size and position persist through eframe; everything else lives in our own files.
    options.persist_window = true;
    // `dotcraft <path>` opens a folder, solution, project or any text file; on macOS Finder's
    // "Open With" arrives through `open_files`.
    let open = std::env::args_os().nth(1).map(std::path::PathBuf::from).filter(|p| !p.to_string_lossy().starts_with("-psn_"));
    open_files::install();
    eframe::run_native("DotCraft", options, Box::new(move |cc| Ok(Box::new(app::DotCraftApp::new(cc, open)))))
}
