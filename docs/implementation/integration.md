# Reviewing the combined implementation

The branch now integrates original upstream `main` at `9e849cd` for [upstream PR 108](https://github.com/javy-fernandez/dotcraft/pull/108). See [current integration evidence](upstream-integration.md) before using the historical validation counts below. Upstream HTTP Client/Endpoints, coverage, monitoring, bookmarks, templates, and history are retained. The fork HTTP workflow is separately named HTTP Scratchpad.

The current batch is delivered through **one upstream PR: [108](https://github.com/javy-fernandez/dotcraft/pull/108)**. Its head is `RafaelMendes1407:integration/rider-implementation-batch` and its base is `javy-fernandez/dotcraft:main`. The separate fork PRs and rebased branches are historical review slices; no additional upstream PRs are required for this batch.

The [combined integration branch](https://github.com/RafaelMendes1407/dotcraft/tree/integration/rider-implementation-batch) provides the assembled source for testing these changes together. It contains the integrated feature implementations and their dependency resolutions. The original feature branches remain available for provenance; they are not separate delivery requirements.

The combined working tree preserves the HTTP and SQLite tools, SDK chooser, responsive Settings, debugger attachment, EditorConfig indentation, and solution-filter lifecycle together. [Validation evidence](validation.md) distinguishes combined checks from independent feature checks and live .NET tests.

## Historical feature dependencies

- PRs 2, 3, 4, and 6 build on inventory PR 1.
- SDK selection PR 5 builds on responsive Settings PR 4.
- HTTP variables PR 15 builds on HTTP Client PR 13.
- SQLite PR 16 builds on documentation PR 6.
- Scrollable main menu PR 17 builds on debugger attachment PR 14.

These dependencies have already been reconciled in the combined branch. [The consolidation audit](branch-consolidation.md) records all 24 source branches. The default branch has not been changed by this implementation batch. The inventory includes unfinished capabilities; a working combined build does not establish complete Rider parity.

## Local validation

Use a different Cargo target directory for each checkout. Run the commands in [validation.md](validation.md) and each feature note. Live tests additionally require the documented SDKs, language server, debugger adapter, package restores, and supported host permissions. Keep the temporary SDK, NuGet, and CLI-home paths local to the validation command.

CrabCraft adaptations retain their source notices in the implementation notes and the accompanying MIT notice files. New code, source UI strings, documentation, PRs, and commit messages use English. Commits created for this batch include Codex as co-author.
