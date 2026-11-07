# Reviewing the combined implementation

Each feature has its own pull request, linked from [the implementation checklist](../features.md). Some changes touch the same app state, menu, settings dialog, or locale catalogs. Review and merge those changes with their documented dependencies rather than replacing a shared file with a single feature's version.

The [combined integration branch](https://github.com/RafaelMendes1407/dotcraft/tree/integration/rider-implementation-batch) provides the assembled source for testing these changes together. It is a snapshot for review, while the feature PRs remain separate and unmerged.

The combined working tree preserves the HTTP and SQLite tools, SDK chooser, responsive Settings, debugger attachment, EditorConfig indentation, and solution-filter lifecycle together. [Validation evidence](validation.md) distinguishes combined checks from independent feature checks and live .NET tests.

## Feature dependencies

- PRs 2, 3, 4, and 6 build on inventory PR 1.
- SDK selection PR 5 builds on responsive Settings PR 4.
- HTTP variables PR 15 builds on HTTP Client PR 13.
- SQLite PR 16 builds on documentation PR 6.
- Scrollable main menu PR 17 builds on debugger attachment PR 14.

Feature branches are delivery candidates. The default branch has not been changed by this implementation batch. The inventory includes unfinished capabilities; a working combined build does not establish complete Rider parity.

## Local validation

Use a different Cargo target directory for each checkout. Run the commands in [validation.md](validation.md) and each feature note. Live tests additionally require the documented SDKs, language server, debugger adapter, package restores, and supported host permissions. Keep the temporary SDK, NuGet, and CLI-home paths local to the validation command.

CrabCraft adaptations retain their source notices in the implementation notes and the accompanying MIT notice files. New code, source UI strings, documentation, PRs, and commit messages use English. Commits created for this batch include Codex as co-author.
