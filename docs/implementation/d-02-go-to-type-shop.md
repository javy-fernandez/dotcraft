# D-02 — Go to Type on Shop

## Behavior

Go to Type displays C# types from the workspace symbol results and opens a chosen type at its declaration. Method results are excluded from the Types tab.

## Validation

`CARGO_TARGET_DIR=/var/tmp/dotcraft-go-to-type-shop-target CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test -p rc-app go_to_type_opens_shop_order_at_its_declaration -- --nocapture`

The egui acceptance test opens the real `tests/fixtures/Shop/Shop.sln`, derives the `Order` class and `OrderLine` record positions from their source files, supplies their workspace symbol results, and verifies that `Add` is absent from the Types tab. Clicking `Order` must close the popup, open `Order.cs`, place the caret at the exact class name, and focus the editor.

## Provenance and limits

The test uses repository-owned Shop fixtures and DotCraft code. No CrabCraft code or external assets were copied. This validates the Search Everywhere type filtering and navigation UI against real C# source positions. The .NET SDK/Roslyn server is unavailable in this environment, so live `workspace/symbol` delivery is unverified. This is a bounded D-02 UI slice, not full Rider parity.
