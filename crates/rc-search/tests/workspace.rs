//! Index, Find in Files and Replace in Files on real folder trees.

use rc_core::watch::FsEvent;
use rc_core::{Inbox, Waker};
use rc_search::*;
use std::fs;
use std::path::Path;
use std::time::Duration;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// A small solution with build output, tool folders and a `.gitignore`.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".gitignore", "*.log\ngenerated/\n");
    write(r, "Shop.slnx", "<Solution />");
    write(r, "src/Shop.Core/Shop.Core.csproj", "<Project />");
    write(r, "src/Shop.Core/OrderService.cs", "class OrderService\n{\n    // TODO: order\n    void PlaceOrder() { }\n}\n");
    write(r, "src/Shop.Core/Form1.Designer.cs", "// TODO: designer\n");
    write(r, "src/Shop.Web/OrderController.cs", "class OrderController { }\n");
    write(r, "src/Shop.Web/notes.txt", "todo later\n");
    write(r, "src/Shop.Core/bin/Debug/Shop.Core.cs", "// TODO: build output\n");
    write(r, "src/Shop.Core/obj/project.assets.json", "TODO");
    write(r, "node_modules/x/index.js", "TODO");
    write(r, ".vs/state.cs", "TODO");
    write(r, "build.log", "TODO");
    write(r, "generated/Gen.cs", "// TODO: generated");
    write(r, "sub/.ignore", "secret.cs\n");
    write(r, "sub/secret.cs", "TODO");
    write(r, "sub/public.cs", "// TODO: public");
    dir
}

fn rels(index: &FileIndex) -> Vec<String> {
    index.snapshot().files().iter().map(|e| e.rel().to_string()).collect()
}

#[test]
fn index_respects_ignore_files_and_hidden_folders() {
    let dir = tree();
    let index = FileIndex::scan_blocking(dir.path());
    assert_eq!(
        rels(&index),
        [
            ".gitignore",
            "Shop.slnx",
            "src/Shop.Core/Form1.Designer.cs",
            "src/Shop.Core/OrderService.cs",
            "src/Shop.Core/Shop.Core.csproj",
            "src/Shop.Web/OrderController.cs",
            "src/Shop.Web/notes.txt",
            "sub/.ignore",
            "sub/public.cs",
        ]
    );
    let snap = index.snapshot();
    assert_eq!(snap.get("Shop.slnx").unwrap().kind(), FileKind::Solution);
    assert_eq!(snap.get("src/Shop.Core/Shop.Core.csproj").unwrap().kind(), FileKind::Project);
}

#[test]
fn background_scan_wakes_the_ui() {
    let dir = tree();
    let woke = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = woke.clone();
    let index = FileIndex::new(dir.path());
    assert!(!index.is_ready());
    index.start_scan(Waker::new(move || flag.store(true, std::sync::atomic::Ordering::SeqCst))).wait();
    assert!(index.is_ready());
    assert!(woke.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(index.snapshot().len(), 9);
}

#[cfg(unix)]
#[test]
fn symlink_loops_are_skipped() {
    let dir = tree();
    std::os::unix::fs::symlink(dir.path().join("src"), dir.path().join("src/Shop.Web/loop")).unwrap();
    let index = FileIndex::scan_blocking(dir.path());
    // The loop is detected; files reached through it once are fine, but the walk ends.
    assert!(index.snapshot().len() >= 9);
}

#[test]
fn incremental_updates() {
    let dir = tree();
    let r = dir.path();
    let index = FileIndex::scan_blocking(r);

    write(r, "src/Shop.Core/Customer.cs", "");
    write(r, "src/Shop.Core/debug.log", "");
    write(r, "src/Shop.Core/bin/Release/x.cs", "");
    write(r, "src/Shop.Api/Program.cs", "");
    write(r, "src/Shop.Api/Controllers/HomeController.cs", "");
    write(r, "generated/More.cs", "");
    write(r, "sub/secret2.cs", "");
    index.apply(&[
        FsEvent::Created(r.join("src/Shop.Core/Customer.cs")),
        FsEvent::Created(r.join("src/Shop.Core/debug.log")),
        FsEvent::Created(r.join("src/Shop.Core/bin/Release/x.cs")),
        FsEvent::Created(r.join("src/Shop.Api")),
        FsEvent::Created(r.join("generated/More.cs")),
        FsEvent::Created(r.join("sub/secret2.cs")),
    ]);
    let snap = index.snapshot();
    assert!(snap.get("src/Shop.Core/Customer.cs").is_some());
    assert!(snap.get("src/Shop.Api/Program.cs").is_some());
    assert!(snap.get("src/Shop.Api/Controllers/HomeController.cs").is_some());
    assert!(snap.get("sub/secret2.cs").is_some());
    assert!(snap.get("src/Shop.Core/debug.log").is_none());
    assert!(snap.get("src/Shop.Core/bin/Release/x.cs").is_none());
    assert!(snap.get("generated/More.cs").is_none());

    // Rename a file, then remove a whole folder.
    fs::rename(r.join("src/Shop.Core/Customer.cs"), r.join("src/Shop.Core/Client.cs")).unwrap();
    index.rename(&r.join("src/Shop.Core/Customer.cs"), &r.join("src/Shop.Core/Client.cs"));
    fs::remove_dir_all(r.join("src/Shop.Api")).unwrap();
    index.apply(&[FsEvent::Removed(r.join("src/Shop.Api"))]);
    let snap = index.snapshot();
    assert!(snap.get("src/Shop.Core/Customer.cs").is_none());
    assert!(snap.get("src/Shop.Core/Client.cs").is_some());
    assert!(!snap.files().iter().any(|e| e.rel().starts_with("src/Shop.Api/")));

    // A canonical path from the watcher (macOS reports /private/var/...) maps to the same root.
    let canon = r.canonicalize().unwrap();
    write(r, "Canon.cs", "");
    index.apply(&[FsEvent::Created(canon.join("Canon.cs"))]);
    assert!(index.snapshot().get("Canon.cs").is_some());
    // An event for a path that no longer exists counts as a removal.
    index.apply(&[FsEvent::Changed(r.join("src/Shop.Web/notes.txt"))]);
    assert!(index.snapshot().get("src/Shop.Web/notes.txt").is_some());
    fs::remove_file(r.join("src/Shop.Web/notes.txt")).unwrap();
    index.apply(&[FsEvent::Changed(r.join("src/Shop.Web/notes.txt"))]);
    assert!(index.snapshot().get("src/Shop.Web/notes.txt").is_none());
}

#[test]
fn events_during_a_scan_are_replayed() {
    let dir = tree();
    let r = dir.path();
    let index = FileIndex::new(r);
    let scan = index.start_scan(Waker::noop());
    write(r, "Late.cs", "");
    index.apply(&[FsEvent::Created(r.join("Late.cs"))]);
    scan.wait();
    assert!(index.snapshot().get("Late.cs").is_some());
}

#[test]
fn go_to_file_over_a_scanned_tree() {
    let dir = tree();
    let index = FileIndex::scan_blocking(dir.path());
    let r = search_files(&index.snapshot(), &FileQuery::parse("OrdSer:12"), &|_| 0, 10);
    assert_eq!(r.matches[0].entry.rel(), "src/Shop.Core/OrderService.cs");
    assert_eq!(r.query.line, Some(12));

    let inbox = Inbox::new(Waker::noop());
    let worker = FileSearchWorker::new(index.clone(), inbox.sender());
    worker.search(FileSearchRequest { text: "zzz".into(), limit: 5, boost: None });
    let id = worker.search(FileSearchRequest { text: "web/ord".into(), limit: 5, boost: None });
    let mut last = None;
    while let Some(res) = inbox.recv_timeout(Duration::from_secs(5)) {
        if res.generation == id {
            last = Some(res);
            break;
        }
    }
    let res = last.expect("worker answered the latest query");
    assert_eq!(res.matches.len(), 1);
    assert_eq!(res.matches[0].entry.rel(), "src/Shop.Web/OrderController.cs");
}

fn run_find(root: &Path, options: FindOptions) -> (Vec<FileMatches>, FindSummary) {
    let inbox = Inbox::new(Waker::noop());
    let handle = find_in_files(root, options, inbox.sender()).unwrap();
    let id = handle.id();
    handle.wait();
    let mut files = Vec::new();
    let mut summary = None;
    for ev in inbox.drain() {
        assert_eq!(ev.search(), id);
        match ev {
            FindEvent::File { file, .. } => files.push(file),
            FindEvent::Done { summary: s, .. } => summary = Some(s),
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    (files, summary.expect("done event"))
}

fn opts(pattern: &str) -> FindOptions {
    FindOptions { query: QueryOptions::text(pattern), ..Default::default() }
}

#[test]
fn find_in_files_respects_ignores_mask_and_scope() {
    let dir = tree();
    let (files, summary) = run_find(dir.path(), opts("todo"));
    let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(rels, ["src/Shop.Core/Form1.Designer.cs", "src/Shop.Core/OrderService.cs", "src/Shop.Web/notes.txt", "sub/public.cs"]);
    assert_eq!(summary.matches, 4);
    assert!(!summary.truncated && !summary.cancelled);

    let order = &files[1];
    assert_eq!(order.path, dir.path().join("src/Shop.Core/OrderService.cs"));
    assert_eq!(order.lines.len(), 1);
    let line = &order.lines[0];
    assert_eq!((line.line, line.text.as_str()), (2, "    // TODO: order"));
    assert_eq!(line.ranges, vec![7..11]);

    let mask = FileMask::parse("*.cs, !*.Designer.cs").unwrap();
    let (files, _) = run_find(dir.path(), FindOptions { mask, ..opts("todo") });
    let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(rels, ["src/Shop.Core/OrderService.cs", "sub/public.cs"]);

    let (files, _) = run_find(dir.path(), FindOptions { scope: Some("src/Shop.Web".into()), ..opts("todo") });
    let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(rels, ["src/Shop.Web/notes.txt"]);
}

#[test]
fn find_options_whole_word_case_regex_and_context() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.cs", "int order = 1;\nOrder o;\nvar orders = 2;\nreorder();\n");
    let count = |q: QueryOptions| run_find(dir.path(), FindOptions { query: q, ..Default::default() }).1.matches;
    assert_eq!(count(QueryOptions::text("order")), 4);
    assert_eq!(count(QueryOptions { pattern: "order".into(), whole_word: true, ..Default::default() }), 2);
    assert_eq!(count(QueryOptions { pattern: "Order".into(), case_sensitive: true, ..Default::default() }), 1);
    assert_eq!(count(QueryOptions { pattern: r"\w+ = \d".into(), regex: true, ..Default::default() }), 2);

    let (files, _) = run_find(dir.path(), FindOptions { query: QueryOptions { pattern: "Order o".into(), case_sensitive: true, ..Default::default() }, context_lines: 1, ..Default::default() });
    let l = &files[0].lines[0];
    assert_eq!(l.before, ["int order = 1;"]);
    assert_eq!(l.after, ["var orders = 2;"]);

    // Several matches on one line, columns in chars.
    write(dir.path(), "b.txt", "ñx x\n");
    let (files, _) = run_find(dir.path(), opts("x"));
    let b = files.iter().find(|f| f.rel == "b.txt").unwrap();
    assert_eq!(b.lines[0].ranges, [1..2, 3..4]);
}

#[test]
fn multi_line_regex_matches() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.cs", "class A\r\n{\r\n}\r\nclass B\n{\n}\n");
    let q = QueryOptions { pattern: r"class \w+\s*\{".into(), regex: true, ..Default::default() };
    let (files, summary) = run_find(dir.path(), FindOptions { query: q, ..Default::default() });
    assert_eq!(summary.matches, 2);
    let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
    assert_eq!(lines, [0, 3]);
    assert_eq!(files[0].lines[0].text, "class A");
    assert_eq!(files[0].lines[0].ranges, vec![0..7]);
}

#[test]
fn binary_and_huge_files_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "text.cs", "needle\n");
    fs::write(dir.path().join("bin.dat"), b"needle\0\x01\x02 needle").unwrap();
    write(dir.path(), "big.cs", &"needle\n".repeat(2000));
    let (files, _) = run_find(dir.path(), FindOptions { max_file_size: 1000, ..opts("needle") });
    let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(rels, ["text.cs"]);
}

#[test]
fn result_cap_truncates() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        write(dir.path(), &format!("f{i}.txt"), "hit hit hit\n");
    }
    let (files, summary) = run_find(dir.path(), FindOptions { max_results: 10, ..opts("hit") });
    assert!(summary.truncated);
    assert_eq!(summary.matches, 10);
    assert!(files.iter().map(FileMatches::match_count).sum::<usize>() <= 10);
}

#[test]
fn a_new_search_cancels_the_running_one() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..4000 {
        write(dir.path(), &format!("d{}/f{i}.txt", i % 40), "some text with a needle in it\n");
    }
    let inbox = Inbox::new(Waker::noop());
    let mut finder = Finder::new(dir.path(), inbox.sender());
    let first = finder.start(opts("needle")).unwrap();
    let second = finder.start(opts("needle")).unwrap();
    assert_ne!(first, second);
    assert_eq!(finder.current(), Some(second));
    let mut done = std::collections::HashMap::new();
    while done.len() < 2 {
        match inbox.recv_timeout(Duration::from_secs(30)).expect("search events") {
            FindEvent::Done { search, summary } => {
                done.insert(search, summary);
            }
            FindEvent::File { .. } => {}
        }
    }
    let (a, b) = (&done[&first], &done[&second]);
    assert!(a.cancelled);
    assert!(a.files_searched < 4000);
    assert!(!b.cancelled);
    assert_eq!(b.files_matched, 4000);

    // Dropping a handle cancels too.
    let h = find_in_files(dir.path(), opts("needle"), inbox.sender()).unwrap();
    drop(h);
}

#[test]
fn bad_patterns_fail_up_front() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(Waker::noop());
    let bad = FindOptions { query: QueryOptions { pattern: "(".into(), regex: true, ..Default::default() }, ..Default::default() };
    assert!(matches!(find_in_files(dir.path(), bad, inbox.sender()), Err(QueryError::InvalidRegex(_))));
}

#[test]
fn replace_on_disk_keeps_bom_and_crlf_and_buffers_get_edits() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::write(r.join("a.cs"), b"\xEF\xBB\xBFvar x = GetName();\r\nvar y = GetAge();\r\n").unwrap();
    write(r, "b.cs", "nothing here\n");
    write(r, "open.cs", "GetName();\n");
    let q = Query::new(QueryOptions { pattern: r"Get(\w+)\(\)".into(), regex: true, case_sensitive: true, ..Default::default() }).unwrap();
    let out = replace_in_files(
        [
            (r.join("a.cs"), ReplaceTarget::Disk),
            (r.join("b.cs"), ReplaceTarget::Disk),
            (r.join("open.cs"), ReplaceTarget::Buffer("x\nGetName();\n".into())),
            (r.join("missing.cs"), ReplaceTarget::Disk),
        ],
        &q,
        r"$1\n",
    );
    assert!(matches!(out[0].1, ReplaceOutcome::Written(2)));
    assert!(matches!(out[1].1, ReplaceOutcome::Written(0)));
    match &out[2].1 {
        ReplaceOutcome::Edits(e) => assert_eq!(e, &[TextEdit { range: 2..11, text: "Name\n".into() }]),
        o => panic!("{o:?}"),
    }
    assert!(matches!(out[3].1, ReplaceOutcome::Failed(ReplaceError::Io(_))));
    assert_eq!(fs::read(r.join("a.cs")).unwrap(), b"\xEF\xBB\xBFvar x = Name\r\n;\r\nvar y = Age\r\n;\r\n");
    // Open buffers are not written.
    assert_eq!(fs::read_to_string(r.join("open.cs")).unwrap(), "GetName();\n");
    // No temporary files left behind.
    assert_eq!(fs::read_dir(r).unwrap().count(), 3);
}

#[test]
fn replace_refuses_non_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("latin1.txt");
    fs::write(&p, b"caf\xE9 cafe").unwrap();
    let q = Query::new(QueryOptions::text("cafe")).unwrap();
    assert!(matches!(replace_in_file(&p, &q, "x"), Err(ReplaceError::NotUtf8)));
    assert_eq!(fs::read(&p).unwrap(), b"caf\xE9 cafe");
}
