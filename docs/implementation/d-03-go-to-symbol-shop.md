# D-03 — Go to Symbol on Shop

## Behavior

The Symbols tab in Search Everywhere excludes type declarations and opens a chosen member at its source location. In Shop, the class and its constructor are both named `Order`; the Symbols tab should offer the constructor and navigate to its exact declaration rather than the class declaration.

## Validation

`CARGO_TARGET_DIR=/var/tmp/dotcraft-go-to-symbol-shop-target CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p rc-app go_to_symbol_opens_shop_order_constructor_not_class -- --nocapture`

The egui acceptance test opens the real `tests/fixtures/Shop/Shop.sln`, derives the `Order` class and constructor positions from `Order.cs`, supplies their workspace symbol results, verifies that only the constructor is shown in the Symbols tab, clicks it, and checks the active file, exact caret line and column, closed popup, and editor focus.

## Provenance and limits

The test uses repository-owned Shop fixtures and existing DotCraft UI. No CrabCraft code or external assets were copied. This validates the Search Everywhere symbol filter and navigation against real C# source positions. The .NET SDK/Roslyn server is unavailable here, so live `workspace/symbol` delivery remains unverified. This is a bounded D-03 UI slice, not full Rider parity.
