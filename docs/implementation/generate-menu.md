# C# Generate menu: local member generators

## Behavior

- The Code menu, editor context menu, and ⌘N / Alt+Insert open a member picker for the class or struct at the caret. The picker uses the current syntax tree and shows fields and properties from that type.
- Constructor generation uses selected fields and properties as parameters. Property generation exposes selected fields, with read-only properties for read-only fields. Equality and hash code generation use `EqualityComparer<T>` and `System.HashCode`. `ToString` and a formatting overload use selected members.
- Generated code is inserted into the current editor buffer as an undoable text edit. The picker refuses to apply an edit if the document changed while it was open. Existing constructors, equality methods, and `ToString` implementations are guarded against duplicate generation.
- Implement, Override, and Delegate tabs offer methods, properties, events, and indexers from interface, base, and field/property types declared in the same file. Type arguments in local generic references are substituted into generated signatures. When no local candidates are found, the picker offers Roslyn code actions as a fallback.

## Validation

- Five focused model tests cover C# syntax-tree member discovery, constructor, property, equality, hash code, `ToString`, formatting, duplicate guards, and generated C# syntax. A headless UI test covers the picker inserting a constructor into the editor. Keymap tests cover ⌘N and Alt+Insert without duplicate bindings.
- `cargo check -p rc-app --locked --offline` validates the app integration.
- Focused parser tests cover local interface, override, and delegation candidates, including generic interface arguments, events, and indexers. An opt-in integration test generated all three groups into one C# file and successfully built it with .NET 10 on Linux.

## Provenance and limits

The implementation is original DotCraft code and uses its existing tree-sitter C# grammar. No CrabCraft code or assets were copied. MIT OR Apache-2.0 licensing applies.

This remains a partial implementation of issue #132. The picker resolves candidate types only when their declarations are in the active file. Roslyn code actions can handle some external cases, but those candidates are not listed in the member picker. Members from referenced projects or assemblies are not generated locally. Advanced generic constraints and explicit interface implementations are not covered. The issue remains open pending semantic member discovery through Roslyn and validation across project references.
