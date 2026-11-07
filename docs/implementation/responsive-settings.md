# Responsive settings

The Settings dialog now sizes itself from the logical viewport. Its width and body height
remain stable when changing between Appearance, Editor, Keymap, and Tools. Narrow windows
use a category dropdown; wider windows keep a vertically scrollable sidebar. The page body
scrolls independently, leaving OK and Cancel outside the scroll area and within reach.

The Tools page retains DotCraft's .NET CLI, Roslyn, and netcoredbg paths. Each label appears
above its path field, and the field reserves space for its translated Browse button. Long
paths scroll within their text fields rather than expanding the dialog. Tool selection,
draft preferences, OK, and Cancel retain their existing behavior.

The viewport layout and stacked field approach are adapted selectively from CrabCraft
commit `475ade7` under MIT OR Apache-2.0. No Rust-specific settings, branding, assets, or
additional dependencies are imported.

The production layout and original two regression tests from existing
[PR 4](https://github.com/RafaelMendes1407/dotcraft/pull/4), commit
`6a7af0e4fdb59d7b1a43e50dda94d6ce324a74a6`, are preserved. Three additional tests extend
the validation of that implementation without replacing its layout.

## Validation

```sh
cargo test -p rc-app responsive_settings_ --offline -- --nocapture
```

Five headless tests passed, including the original tests:

- `responsive_settings_tests::settings_stay_bounded_and_keep_the_footer_visible_on_every_page`
  checks each page against the viewport with a real .NET solution model, long tool paths,
  visible footer and Browse buttons, and stable dimensions over twenty additional frames.
- `responsive_settings_tests::cancelling_settings_preserves_the_original_configuration`
  verifies compact category navigation and cancellation of modified draft preferences.

Additional coverage:

- `responsive_settings_keep_pages_and_confirmation_inside_the_viewport` checks all four
  pages at 1280×800, 800×500, 800×300, 640×400, and 480×320 logical pixels. Modal and button
  bounds remain within the viewport, widths match across pages, and heights remain stable
  over fifteen additional frames per page.
- `responsive_settings_scroll_to_the_last_editor_option_and_cancel_the_draft` navigates
  through the compact dropdown and wider sidebar, scrolls to the last editor checkbox,
  edits the draft, and cancels without changing saved preferences.
- `responsive_settings_tool_fields_fit_long_paths_without_horizontal_scrolling` verifies
  path field and Browse button horizontal bounds with long .NET tool paths at 480×320 and
  800×300.

The tested scope covers viewport layout and control reachability. The existing native Browse
dialog still runs synchronously; asynchronous and cancellable file selection is separate work.
Other dialogs retain their existing layout. No operating-system-specific window-manager or
screen-reader validation is claimed.
