# HTTP file variables

HTTP request files can declare reusable text values and interpolate them in requests. This extends the [basic HTTP Client](http-client.md) without adding environment access or script execution.

```http
@host = localhost:5000
@base = http://{{host}}/api
@header = X-Example
@name = Example

### Create an item
POST {{base}}/items
{{header}}: {{name}}
Content-Type: application/xml

<item><name>{{name}}</name></item>

### Read items
GET {{base}}/items
```

## Declaration and interpolation rules

- Declare `@name = value` before the request line of a `###` block. A block can contain declarations only.
- Declarations apply to the entire file, including earlier request blocks. Each name must be unique.
- Names contain 1–128 ASCII letters, digits, underscores, hyphens, or dots. Names are case-sensitive.
- Whitespace around declaration names, values, and placeholder names is removed. Empty values are allowed.
- Values can refer to other declared values using `{{name}}`; nesting and forward references are supported.
- Interpolation applies to request lines, header names, header values, and text bodies. Source line numbers continue to refer to the original editor buffer.
- URLs, methods, and headers are validated after interpolation. Expanding a value cannot bypass the existing URL scheme or header validation.

Values are literal strings. Shell syntax such as `$HOME` or `$(command)` is preserved as text. Expressions, generated values, environment-file lookup, OS environment expansion, file reads, and response handlers are not evaluated.

Declarations after a request line are rejected; start another `###` block for additional declarations. Duplicate declarations, undefined references, cycles, invalid names, and unclosed placeholders produce explicit request errors. Consecutive closing braces outside a placeholder remain literal text, so nested JSON bodies work. An invalid request file blocks execution, consistent with the basic parser's behavior.

## Expansion limits

The parser accepts at most 256 declarations and a nesting depth of 32 values. Resolving more than 4,096 placeholder references fails, including references to cached or empty values.

Each expanded string, the combined cache of resolved variable values, and the combined expanded request lines, header names/values, and bodies across the file are each limited to 1 MiB. The existing 1 MiB source-file limit also applies. Expansion fails instead of truncating values, preventing a request from being sent with altered contents.

These limits bound substitution work and retained expanded text. URL canonicalization and the HTTP library maintain their own validated representations. The existing HTTP timeout, cancellation, response limits, and explicit Send action remain unchanged.

## Validation

```bash
cargo test -p rc-app --offline http_
```

Tests cover:

- Nested and forward references, empty and Unicode values, repeated use across request blocks, and preservation of literal shell text.
- Undefined names, cycles, malformed placeholders, duplicate declarations, and unsupported expressions.
- Invalid expanded URL schemes, invalid expanded header names, and CR injection in expanded header values.
- Exponential growth, excessive nesting, declaration count, reference count, and aggregate expansion limits.
- An actual local HTTP server receiving the substituted URL path, header name/value, and XML body, and returning a response.
- Existing HTTP request parsing, bounded responses, timeout/cancellation, parser caching, and headless Send/Cancel behavior.

Local HTTP tests do not establish support for environment files, generated/dynamic variables, external authentication providers, or scripts.

## Implementation and provenance

The parser API remains `parse_requests(&str) -> Result<Vec<Request>, HttpError>`. The resolver and declaration extraction live in [`http_client.rs`](../../crates/rc-app/src/http_client.rs), so the existing editor selection and response UI require no new wiring or dependencies.

The bounded resolver follows the caching, recursion tracking, and size-limit approach in [CrabCraft's http_vars.rs at commit 475ade7b5ef0020c953f68a51f1b897380c1ea06](https://github.com/RafaelMendes1407/CrabCraft/blob/475ade7b5ef0020c953f68a51f1b897380c1ea06/crates/rc-syntax/src/http_vars.rs), available under MIT or Apache-2.0. DotCraft additionally bounds reference count and aggregate expanded request text and rejects duplicate declarations.
