//! Gutter line diff on a 5,000-line file: one typed character, a pasted block, a deleted
//! block, scattered edits, and the side-by-side model of the same file.

use criterion::{Criterion, criterion_group, criterion_main};
use rc_vcs::{LineChanges, SideBySide};
use std::hint::black_box;

fn source(lines: usize) -> String {
    let mut s = String::new();
    for i in 0..lines {
        match i % 7 {
            0 => s.push_str("    }\n"),
            1 => s.push('\n'),
            2 => s.push_str(&format!("    public int Method{i}(int value) => value * {i};\n")),
            3 => s.push_str("    {\n"),
            _ => s.push_str(&format!("        var local{i} = Compute(\"{i}\", {i});\n")),
        }
    }
    s
}

fn bench(c: &mut Criterion) {
    let base = source(5_000);
    let mut lines: Vec<String> = base.lines().map(String::from).collect();

    let mut typed = lines.clone();
    typed[2_500].push('x');
    let typed = typed.join("\n");

    let mut pasted = lines.clone();
    for k in 0..40 {
        pasted.insert(1_000, format!("        // pasted {k}"));
    }
    let pasted = pasted.join("\n");

    let mut scattered = lines.clone();
    for k in (0..5_000).step_by(97) {
        scattered[k] = format!("changed {k}");
    }
    let scattered = scattered.join("\n");

    lines.drain(3_000..3_200);
    let deleted = lines.join("\n");

    c.bench_function("gutter 5k lines, one char typed", |b| b.iter(|| LineChanges::compute(black_box(&base), black_box(&typed))));
    c.bench_function("gutter 5k lines, 40 lines pasted", |b| b.iter(|| LineChanges::compute(black_box(&base), black_box(&pasted))));
    c.bench_function("gutter 5k lines, 200 lines deleted", |b| b.iter(|| LineChanges::compute(black_box(&base), black_box(&deleted))));
    c.bench_function("gutter 5k lines, 52 scattered edits", |b| b.iter(|| LineChanges::compute(black_box(&base), black_box(&scattered))));
    c.bench_function("side-by-side 5k lines, 52 scattered edits", |b| b.iter(|| SideBySide::compute(black_box(&base), black_box(&scattered))));
}

criterion_group!(benches, bench);
criterion_main!(benches);
