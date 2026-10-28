#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod convert;
mod debugger;
mod insight;
mod item_templates;
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

/// Opened from Finder, the app gets a bare PATH (/usr/bin:/bin:…) without what Terminal adds
/// from /etc/paths and /etc/paths.d, such as /usr/local/share/dotnet: the tools it starts
/// (Roslyn's project loader, `dotnet watch`, tests) would not find `dotnet`.
#[cfg(target_os = "macos")]
fn system_path() {
    let Ok(out) = std::process::Command::new("/usr/libexec/path_helper").arg("-s").output() else { return };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(helper) = text.split("PATH=\"").nth(1).and_then(|r| r.split('"').next()) else { return };
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<std::path::PathBuf> = std::env::split_paths(&current).collect();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    for d in std::env::split_paths(helper) {
        // /etc/paths.d entries such as `~/.dotnet/tools` are written with a tilde.
        let d = match (d.strip_prefix("~"), &home) {
            (Ok(rest), Some(h)) => h.join(rest),
            _ => d,
        };
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    if let Ok(joined) = std::env::join_paths(dirs) {
        // SAFETY: first thing in main, before any other thread exists.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

fn main() -> eframe::Result {
    #[cfg(target_os = "macos")]
    system_path();
    let mut options = eframe::NativeOptions { viewport: rc_kit::window_chrome::viewport(), ..Default::default() };
    // Window size and position persist through eframe; everything else lives in our own files.
    options.persist_window = true;
    // `dotcraft <path>` opens a folder, solution, project or any text file; on macOS Finder's
    // "Open With" arrives through `open_files`.
    let open = std::env::args_os().nth(1).map(std::path::PathBuf::from).filter(|p| !p.to_string_lossy().starts_with("-psn_"));
    open_files::install();
    eframe::run_native("DotCraft", options, Box::new(move |cc| Ok(Box::new(app::DotCraftApp::new(cc, open)))))
}
