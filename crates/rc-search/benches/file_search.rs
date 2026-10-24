//! Go to File over a generated tree of 6 000 paths, and Find in Files over 1 000 real files.

#[path = "../tests/common/mod.rs"]
mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use rc_core::{Inbox, Waker};
use rc_search::{FileIndex, FileQuery, FindOptions, IndexSnapshot, QueryOptions, find_in_files, rank, search_files};
use std::hint::black_box;
use std::path::Path;

fn go_to_file(c: &mut Criterion) {
    let paths = common::synthetic_paths(6000);
    let snap = IndexSnapshot::from_rel_paths(Path::new("/ws"), paths.iter().map(String::as_str));
    let mut g = c.benchmark_group("go_to_file_6000");
    for q in ["OrdSer", "core/order", "custinv", "o", "PaymentController12.cs:40", "xyzzy"] {
        let query = FileQuery::parse(q);
        g.bench_function(q, |b| b.iter(|| black_box(search_files(&snap, &query, &|_| 0, 50))));
    }
    g.finish();

    let names: Vec<&str> = paths.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
    c.bench_function("rank_6000_names/OrdSer", |b| b.iter(|| black_box(rank(names.iter().copied(), "OrdSer", 50))));
}

fn index_and_find(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    for (i, p) in common::synthetic_paths(1000).iter().enumerate() {
        let path = dir.path().join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("namespace Shop;\n\npublic class C{i}\n{{\n    // TODO: item {i}\n}}\n").repeat(20)).unwrap();
    }
    c.bench_function("scan_1000_files", |b| b.iter(|| black_box(FileIndex::scan_blocking(dir.path()).snapshot().len())));
    c.bench_function("find_in_1000_files/TODO", |b| {
        b.iter(|| {
            let inbox = Inbox::new(Waker::noop());
            let opts = FindOptions { query: QueryOptions::text("todo"), ..Default::default() };
            find_in_files(dir.path(), opts, inbox.sender()).unwrap().wait();
            black_box(inbox.drain().len())
        })
    });
}

criterion_group!(benches, go_to_file, index_and_find);
criterion_main!(benches);
