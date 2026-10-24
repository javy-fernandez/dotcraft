//! Go to File must answer within a frame budget on a mid-size solution.

mod common;

use rc_search::{FileQuery, IndexSnapshot, search_files};
use std::path::Path;
use std::time::{Duration, Instant};

#[test]
fn go_to_file_over_6000_files_is_fast() {
    let paths = common::synthetic_paths(6000);
    let snap = IndexSnapshot::from_rel_paths(Path::new("/ws"), paths.iter().map(String::as_str));
    assert_eq!(snap.len(), 6000);
    let queries = ["OrdSer", "core/order", "custinv", "o", "PaymentController12.cs:40", "tests/unit/val", "xyzzy"];
    // Warm up thread-local matchers.
    search_files(&snap, &FileQuery::parse("x"), &|_| 0, 50);
    let mut worst = Duration::ZERO;
    for q in queries {
        let query = FileQuery::parse(q);
        let t = Instant::now();
        let r = search_files(&snap, &query, &|_| 0, 50);
        worst = worst.max(t.elapsed());
        assert!(r.matches.len() <= 50);
    }
    // The goal is 50 ms in release; unoptimised test builds get slack.
    let budget = if cfg!(debug_assertions) { Duration::from_millis(500) } else { Duration::from_millis(50) };
    assert!(worst < budget, "slowest query took {worst:?}");
    eprintln!("slowest Go to File query over 6000 files: {worst:?}");
}
