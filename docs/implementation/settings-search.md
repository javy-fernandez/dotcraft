# Settings search

Search Everywhere includes destinations for the four existing settings pages: Appearance,
Editor, Keymap, and Tools. Results appear in the All and Actions tabs. Search a page name
or combine it with Settings, such as `Settings Editor`, then press Enter or click the result.

Each destination opens the settings dialog on its matching page with a draft of the current
preferences. Selecting a result closes Search Everywhere and keeps focus with settings.
Opening a page does not save or change preferences; the existing OK and Cancel behavior applies.

## Search behavior

- Settings results use the existing fuzzy matcher and translated page names.
- Settings is included in the matching text and displayed as the result group and secondary label.
- File, type, and symbol tabs exclude settings destinations.
- An empty All search retains its existing recent-file behavior without settings results.
- An empty Actions search includes the available settings destinations.

The scope is navigation to implemented sections. Individual controls and preference values
are not indexed, and no additional settings pages are introduced. Language intelligence and
workspace indexing are not prerequisites for settings navigation.

## Implementation

`settings_items` in `crates/rc-app/src/search.rs` maps translated labels to existing
`SettingsPage` variants. `crates/rc-app/src/ui/search_popup.rs` ranks and renders these
destinations and opens the requested page when selected. Existing action results remain available.

The updated All search hint has an English source entry in
`scripts/i18n/settings-search.tsv`. Other locales currently use the same English fallback
for this new hint; existing translated settings and page labels are reused.

## Validation

Run the headless regression tests without a .NET SDK or language server:

```sh
cargo test -p rc-app settings_results_ -- --nocapture
```

Verified result: two tests passed.

- `ui::search_popup::tests::settings_results_open_each_existing_page_from_search_everywhere`
  opens all four destinations with Enter from All and with a pointer click from Actions.
  It verifies the selected page, current preference draft, unchanged preferences, closed
  search popup, and settings focus.
- `ui::search_popup::tests::settings_results_stay_out_of_file_type_and_symbol_searches`
  verifies exclusions from file, type, and symbol searches and the empty All search.

Translation coverage also passed:

```sh
cargo test -p rc-app -p rc-kit every_text_in_the_code_has_a_translation --offline
```

Including `rc-app` enables the Linux window-platform dependencies needed by the shared UI crates.

These tests exercise native egui widgets headlessly; they do not establish screen-reader
support or operating-system-specific desktop behavior.
