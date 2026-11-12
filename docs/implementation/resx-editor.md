# Multi-culture .resx resource editor

## Behavior

- Opening a `.resx` file shows a Resources tool window with one value column per sibling culture file and a selectable Comments column. Neutral resources appear first when present. Missing translations are highlighted and can be added explicitly.
- The grid adds, renames, and removes string resource keys across the culture family. A translation can also be removed from one culture. Save writes the changed files; Refresh reloads them, and Discard abandons unsaved grid edits. Open XML opens the selected file in DotCraft's text editor.
- Resource discovery, XML parsing, and writes run on the background runtime. Save checks that every source file still matches the version loaded by the grid, validates duplicate keys and XML, and writes only changed files.
- Untouched headers, XML comments, formatting, and typed or binary resource entries remain in place. String edits change only the affected XML spans.

## Validation

- Four focused `rc-dotnet` tests passed for culture-wide rename/add/remove, XML and binary-entry preservation, inline XML insertion, and external-change conflict handling.
- `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo check -p rc-app --locked --offline --target-dir /home/rafael-mendes/Documentos/dotcraft/target/issue-120 --jobs 2` passed on Linux with the existing `open_files::push` dead-code warning.
- Two focused headless app tests passed; one covers opening a resource file, showing the grid, adding a missing French translation, and saving it.

## Provenance and limits

The file family naming and string-entry model follow Microsoft's [localization guide](https://learn.microsoft.com/en-us/dotnet/core/extensions/localization) and [.resx format documentation](https://learn.microsoft.com/en-us/dotnet/core/extensions/create-resource-files). The implementation is original DotCraft code; no third-party code or assets were copied. MIT OR Apache-2.0 licensing applies.

The grid edits string `<data>` entries without `type` or `mimetype`; typed and binary entries stay untouched and can be viewed through Open XML. Conditional MSBuild resource inclusion is not evaluated. Saves preflight every culture file, but the file system cannot make a multi-file save atomic if a later write fails. Real Windows and macOS UI runs remain untested.
