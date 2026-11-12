# Searchable Explorer Add menu

## Behavior

The Solution Explorer's **Add** submenu has a focused search field and a
height-limited result list. The field filters the existing .NET item templates
by their translated names, ignoring surrounding spaces and case. Enter selects
the first match; an unmatched query shows **No results** and creates nothing.
The original .NET template actions and target directory are unchanged.

## Provenance

Adapted selectively from CrabCraft `8ee8d1d`, specifically commits
`95e6a4200da83f009b14f431216446202e82414b` and
`2356dd9909bda2aed8e7c4a07648c08dd34e332c` in
`crates/rc-app/src/ui/explorer.rs`. CrabCraft is licensed MIT OR Apache-2.0;
its MIT notice is already included in `docs/third-party/CrabCraft-LICENSE-MIT.txt`.
No assets are copied.

## Validation

Focused headless menu tests exercise typing into the search field, filtering,
Enter selection of a .NET template, and an empty result:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test -p rc-app template_picker_tests --locked --offline \
  --target-dir target/validation-visual-add --jobs 2
```

Both tests passed. The locales have an explicit English fallback for the new
search hint, with Brazilian Portuguese translated. All seven locale JSON files
parsed, and the new key was verified under each `additems` section.

An additional standalone `rc-kit` i18n test attempt could not compile because
that package's independent test feature set leaves `winit` without a platform
backend. The focused application tests compiled the same locale and UI code.

## Remaining limits

This slice changes the Explorer Add submenu only. It does not change project
creation dialogs, template definitions, or .NET project handling.
