//! Go/no-go numbers of phase 0 for the syntax layer, on a 20k-line C# file.

use criterion::{Criterion, criterion_group, criterion_main};
use rc_syntax::{Lang, highlight_lines, input_edit, parse_now, sample::big_csharp};
use rc_text::{Buffer, Change, ChangeSet, EditKind, Selection};
use std::hint::black_box;

fn benches(c: &mut Criterion) {
    let text = big_csharp(20_000);
    let buf = Buffer::new(&text);
    let rope = buf.snapshot();
    let tree = parse_now(Lang::CSharp, &rope, None).unwrap();

    c.bench_function("full parse 20k lines", |b| b.iter(|| black_box(parse_now(Lang::CSharp, &rope, None))));

    // Typing one char in the middle of a method body, then reparsing incrementally.
    let mid = rope.line_to_char(10_000) + 8;
    c.bench_function("incremental reparse after a keystroke", |b| {
        b.iter_batched(
            || {
                let mut buf = buf.clone();
                let edits = buf.apply(&ChangeSet::single(Change::insert(mid, "x")), &Selection::point(mid), &Selection::point(mid + 1), EditKind::Insert);
                let mut old = tree.clone();
                for e in &edits {
                    old.edit(&input_edit(e));
                }
                (buf.snapshot(), old)
            },
            |(rope, old)| black_box(parse_now(Lang::CSharp, &rope, Some(&old))),
            criterion::BatchSize::SmallInput,
        )
    });

    c.bench_function("highlight 80 visible lines", |b| {
        b.iter(|| black_box(highlight_lines(Lang::CSharp, &tree, &rope, 10_000, 80)))
    });
    c.bench_function("folding ranges 20k lines", |b| b.iter(|| black_box(rc_syntax::folding_ranges(Lang::CSharp, &tree, &rope))));
}

criterion_group! {
    name = parse;
    config = Criterion::default().sample_size(30).measurement_time(std::time::Duration::from_secs(3));
    targets = benches
}
criterion_main!(parse);
