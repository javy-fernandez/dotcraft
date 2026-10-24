//! Real processes on a pseudo-terminal: `/bin/sh` with a known environment.
#![cfg(unix)]

use alacritty_terminal::grid::Dimensions;
use rc_core::Waker;
use rc_term::{SelectKind, SessionEvent, SpawnSpec, TermSession};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);

fn sh(args: &[&str]) -> SpawnSpec {
    SpawnSpec::command("/bin/sh", args.iter().copied(), Some(std::env::temp_dir())).env("PS1", "$ ").env("ENV", "").size(60, 12)
}

fn interactive(waker: Waker) -> TermSession {
    let mut spec = sh(&["-i"]);
    spec.announce_exit = false;
    let s = TermSession::spawn(&spec, waker).expect("spawn /bin/sh");
    assert!(s.wait_for_text("$", TIMEOUT), "no prompt: {:?}", s.screen_lines());
    s
}

#[test]
fn echo_shows_in_the_grid_and_wakes_the_ui() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    let s = interactive(Waker::new(move || {
        w.fetch_add(1, Ordering::Relaxed);
    }));
    s.write(b"echo hello\n".to_vec());
    let ok = s.wait_until(TIMEOUT, |st| (0..12).any(|r| st.row_text(r) == "hello"));
    assert!(ok, "screen: {:#?}", s.screen_lines());
    assert!(s.screen_lines().iter().any(|l| l.ends_with("echo hello")));
    assert!(wakes.load(Ordering::Relaxed) > 0);
    assert!(s.is_running());
}

#[test]
fn environment_is_set() {
    let s = TermSession::spawn(&sh(&["-c", "echo \"$TERM/$COLORTERM/$TERM_PROGRAM/$FOO\""]).env("FOO", "bar"), Waker::noop()).unwrap();
    assert!(s.wait_for_text("xterm-256color/truecolor/DotCraft/bar", TIMEOUT), "{:#?}", s.screen_lines());
}

#[test]
fn resize_reaches_the_process() {
    let s = interactive(Waker::noop());
    s.resize(50, 10, 8, 16);
    assert_eq!(s.size(), (50, 10));
    assert_eq!(s.lock().term.grid().columns(), 50);
    s.write(b"stty size\n".to_vec());
    assert!(s.wait_until(TIMEOUT, |st| (0..10).any(|r| st.row_text(r) == "10 50")), "{:#?}", s.screen_lines());
}

#[test]
fn exit_status_is_reported() {
    let s = TermSession::spawn(&sh(&["-c", "printf 'bye\\n'; exit 3"]), Waker::noop()).unwrap();
    assert!(s.wait_until(TIMEOUT, |st| st.exit().is_some()));
    let exit = s.exit_status().unwrap();
    assert_eq!(exit.code, 3);
    assert!(!exit.success());
    assert!(!s.is_running());
    let events = s.poll_events();
    assert!(events.contains(&SessionEvent::Exited(exit)), "{events:?}");
    let lines = s.screen_lines();
    assert!(lines.iter().any(|l| l == "bye"), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("exit code 3")), "{lines:#?}");
}

#[test]
fn kill_ends_the_process() {
    let s = interactive(Waker::noop());
    s.kill();
    assert!(s.wait_until(TIMEOUT, |st| st.exit().is_some()));
    let exit = s.exit_status().expect("killed process exits");
    assert!(!exit.success());
}

#[test]
fn title_and_colours_from_the_process() {
    let s = TermSession::spawn(&sh(&["-c", "printf '\\033]0;my title\\007\\033[31mred\\033[0m\\n'; sleep 5"]), Waker::noop()).unwrap();
    assert!(s.wait_for_text("red", TIMEOUT));
    s.poll_events();
    assert_eq!(s.title().as_deref(), Some("my title"));
    let st = s.lock();
    let cell = &st.term.grid()[alacritty_line(0)][alacritty_terminal::index::Column(0)];
    assert_eq!(cell.c, 'r');
    assert_eq!(cell.fg, alacritty_terminal::vte::ansi::Color::Named(alacritty_terminal::vte::ansi::NamedColor::Red));
}

fn alacritty_line(l: i32) -> alacritty_terminal::index::Line {
    alacritty_terminal::index::Line(l)
}

#[test]
fn selection_text_extraction() {
    let s = TermSession::output_only(20, 4, Waker::noop());
    s.feed(b"foo bar baz\r\nsecond line\r\n");
    s.select(SelectKind::Simple, (0, 4), (0, 6));
    assert_eq!(s.selection_text().as_deref(), Some("bar"));
    s.select(SelectKind::Word, (0, 9), (0, 9));
    assert_eq!(s.selection_text().as_deref(), Some("baz"));
    s.select(SelectKind::Lines, (1, 3), (1, 3));
    assert_eq!(s.selection_text().as_deref(), Some("second line\n"));
    s.select(SelectKind::Simple, (0, 8), (1, 5));
    assert_eq!(s.selection_text().as_deref(), Some("baz\nsecond"));
    s.select_all();
    assert_eq!(s.selection_text().unwrap().trim_end(), "foo bar baz\nsecond line");
    s.clear_selection();
    assert_eq!(s.selection_text(), None);
}

#[test]
fn wide_chars_take_two_cells() {
    let s = TermSession::output_only(20, 2, Waker::noop());
    s.feed("a中b".as_bytes());
    let st = s.lock();
    let row = &st.term.grid()[alacritty_line(0)];
    assert_eq!(row[alacritty_terminal::index::Column(1)].c, '中');
    assert_eq!(row[alacritty_terminal::index::Column(3)].c, 'b');
    assert_eq!(st.row_text(0), "a中b");
}

#[test]
fn bracketed_paste_follows_the_mode() {
    let s = interactive(Waker::noop());
    // `cat -v` shows the escape sequences it receives.
    s.write(b"printf '\\033[?2004h'; cat -v\n".to_vec());
    assert!(s.wait_until(TIMEOUT, |st| st.mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)));
    s.paste("pasted");
    s.write(b"\n".to_vec());
    assert!(s.wait_for_text("^[[200~pasted^[[201~", TIMEOUT), "{:#?}", s.screen_lines());
}
