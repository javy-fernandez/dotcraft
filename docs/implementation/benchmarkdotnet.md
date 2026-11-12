# BenchmarkDotNet runs from the C# gutter

## Behavior

- C# classes and methods containing `[Benchmark]` or `[BenchmarkAttribute]` receive run icons in the gutter. Clicking one runs that class or method from its nearest runnable console project.
- DotCraft builds in Release through the existing cancellable .NET runner, disables launch profiles, and forwards `--filter`, `--exporters json`, and a unique artifacts path to the benchmark program. Stop cancels the process tree; Rerun repeats the selected filter.
- The Benchmarks tool window shows method/job names, mean, confidence interval margin, standard deviation, and allocated bytes when present. Raw process output remains available below the table. Reports are read on a background task, and each run's artifacts remain under DotCraft's data directory in `benchmarks/`.

## Validation

- `cargo check -p rc-app --locked --offline` passed with a dedicated compact Cargo target directory. Four focused benchmark tests passed for C# marker discovery, Release command arguments, JSON statistics parsing, and exported report loading. The tool-window session identity test passed.
- All seven locale JSON files parsed and `git diff --check` passed.

## Provenance and limitations

The CLI flags and filter behavior follow [BenchmarkDotNet's console arguments](https://benchmarkdotnet.org/articles/guides/console-args.html); report fields follow its [JSON exporter documentation](https://benchmarkdotnet.org/articles/configs/exporters.html). The code is original DotCraft work under MIT OR Apache-2.0; no third-party code or assets were copied.

The benchmark program must use `BenchmarkSwitcher` and forward `args` from `Main` for gutter filters and JSON export to work. The selected project must be a runnable console app. Results are not compared against prior runs, and no live BenchmarkDotNet package run was available in the offline validation environment.
