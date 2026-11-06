# DotCraft working rules

- Write new documentation, comments, UI source strings, commit messages, and pull requests in English. Existing translations may remain; new locale entries may use explicit English fallback values.
- Include `Co-authored-by: Codex <codex@openai.com>` in every commit created by Codex.
- Use parallel agents for independent work when available. Assign separate files or coordinate ownership before editing shared modules.
- Open a separate pull request in `RafaelMendes1407/dotcraft` for each completed feature or independently reviewable slice. Document behavior, validation, provenance, and remaining limitations. Do not merge or release without authorization.
- Track the backlog in `docs/features.md` and feature evidence in `docs/implementation/`. Mark a feature complete only for its tested scope; partial implementations do not establish Rider parity.
- Reuse CrabCraft selectively while preserving DotCraft's .NET behavior, branding, copyright notices, and MIT OR Apache-2.0 licensing. Record source commits and asset licenses when copying code or assets.
- Keep the desktop app in native Rust and egui. Keep I/O and processes off the UI thread, make long operations cancellable, and route UI source text through `rc_kit::i18n::tr` or `trf`.
- Follow `docs/architecture.md` for crate boundaries and test conventions, except where these rules or the user's instructions override it.
