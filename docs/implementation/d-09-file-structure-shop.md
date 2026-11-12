# D-09 — File Structure window on Shop

## Behavior

The Structure tool window shows a document's symbol hierarchy and moves the editor caret to a selected symbol. An empty symbol response now shows the existing “No structure” message instead of a blank panel. The empty-state title is also exposed to accessibility queries.

## Validation

`cargo test -p rc-app shop_order_structure_shows_empty_state_and_navigates_members -- --nocapture`

The egui test opens the real `tests/fixtures/Shop/Shop.sln` and `src/Shop.Core/Order.cs`. It derives the `Order`, `Add`, and `Sample` locations from that C# source, supplies the symbol hierarchy that the LSP response normally provides, verifies the empty state, checks that the hierarchy appears, clicks `Add`, and verifies the resulting caret line and editor focus.

## Provenance and scope

The test uses DotCraft's existing UI and repository-owned Shop fixture. No CrabCraft code or assets were copied. The fixture is covered by this repository's licensing.

This validates the Structure window's rendering and navigation with a real C# file. The symbols are supplied by the test because the .NET SDK and Roslyn server are unavailable in this environment; live LSP symbol delivery remains unverified. This is a bounded UI slice of D-09, not full Rider parity.
