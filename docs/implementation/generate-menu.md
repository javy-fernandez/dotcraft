# C# Generate menu: local member generators

## Behavior

- The Code menu, editor context menu, and ⌘N / Alt+Insert open a member picker for the class or struct at the caret. The picker uses the current syntax tree and shows fields and properties from that type.
- Constructor generation uses selected fields and properties as parameters. Property generation exposes selected fields, with read-only properties for read-only fields. Equality and hash code generation use `EqualityComparer<T>` and `System.HashCode`. `ToString` and a formatting overload use selected members.
- Generated code is inserted into the current editor buffer as an undoable text edit. The picker refuses to apply an edit if the document changed while it was open. Existing constructors, equality methods, and `ToString` implementations are guarded against duplicate generation.

## Validation

- Five focused model tests cover C# syntax-tree member discovery, constructor, property, equality, hash code, `ToString`, formatting, duplicate guards, and generated C# syntax. A headless UI test covers the picker inserting a constructor into the editor. Keymap tests cover ⌘N and Alt+Insert without duplicate bindings.
- `cargo check -p rc-app --locked --offline` validates the app integration.

## Provenance and limits

The implementation is original DotCraft code and uses its existing tree-sitter C# grammar. No CrabCraft code or assets were copied. MIT OR Apache-2.0 licensing applies.

This is a first slice of issue #132. It does not yet generate interface implementations, overrides, or delegation. It reads members from the current type only, and does not resolve symbols from referenced projects or assemblies. The issue remains open until those generators and their member picker are implemented and validated.
