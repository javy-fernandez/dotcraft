# HTTP Client

DotCraft can send requests from the active `.http` or `.rest` editor buffer and inspect the response in the HTTP Client tool window. Requests run only after an explicit Send Request action. Opening or editing a request file performs no network activity.

## Usage

1. Open a workspace and create or open an `.http` or `.rest` file.
2. Open HTTP Client from the bottom tool-window stripe.
3. Select a request and click Send Request. Unsaved editor text is used directly.
4. Inspect the status code, elapsed time, retained body size, response headers, and response body. Click Cancel Request to stop a pending request.

```http
GET https://example.com/
Accept: text/html

### Send an XML payload
POST http://localhost:5000/items
Content-Type: application/xml

<item><name>Example</name></item>
```

## Implemented scope

- Multiple requests separated by lines starting with `###`.
- Explicit GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS, TRACE, and CONNECT method syntax; a bare absolute HTTP or HTTPS URL defaults to GET.
- Request headers and a text body separated by a blank line. Duplicate headers are retained.
- Comments starting with `#` or `//` before the request or among its headers.
- File variables declared with `@name = value` before request lines, with bounded literal `{{name}}` interpolation. See [HTTP file variables](http-variables.md) for declaration rules and limits.
- Request selection from the active editor buffer, with source line numbers in the selector.
- Asynchronous execution on the shared runtime and explicit cancellation during connection or body reads.
- The latest response's status, headers, elapsed time, and body, including HTTP error statuses.
- Read-only plain-text display. Binary bodies are identified without rendering their contents.
- Parser caching by document identity, path, and buffer version; unchanged frames do not reparse or copy the request file.
- Cancellation and state reset when closing or switching workspaces, and cancellation when the HTTP client state is dropped.

## Limits and unsupported syntax

Request files are limited to 1 MiB. Requests have a 60-second total timeout, retain at most 8 MiB of response body data, and display at most 512 Ki characters. The response view identifies truncation. These limits apply to retained/displayed data; the network stack manages its own read buffers.

Only one request runs at a time, and only the latest exchange is retained in memory. Responses are not persisted. Redirects are not followed automatically, so their original status and headers remain visible. Explicit HTTP version suffixes are not supported; protocol negotiation uses the HTTP library defaults.

Environment files, dynamic variables, expressions, directives, file-backed bodies (`< file`), scripts, response-file redirection, and non-HTTP protocols are rejected before execution. A file containing unsupported syntax blocks sending rather than silently omitting its behavior. XML and HTML text bodies are supported. URLs must be absolute and must not embed credentials; authentication headers can be written explicitly.

Request separators and line endings are processed as text: CRLF input is normalized to LF in body text. Binary uploads, multipart helpers, JSON formatting, authentication dialogs, custom TLS configuration, persistent history, gutter actions, and CLI execution remain outside this implementation.

The tool window is available within a workspace. The application's existing standalone-file layout does not display tool-window stripes.

## Validation

```bash
cargo test -p rc-app --offline http_
cargo test -p rc-app -p rc-kit --offline every_text_in_the_code_has_a_translation
```

Tests use local TCP servers and headless egui interaction. They cover:

- Multiple request parsing, source lines, CRLF input, headers, XML bodies, size limits, and unsupported syntax rejection.
- File-variable interpolation, undefined names, cycles, malformed placeholders, expansion bounds, validation after interpolation, and delivery of substituted URL/header/body values to a local server.
- Actual method, URL path, header, and XML body delivery to a local server.
- Response status/header capture and bounded body truncation.
- Cancellation and timeout while a server stalls during its response body.
- Explicit Send from the selected request, visible response controls, Cancel, and workspace state reset.

These tests exercise local HTTP. They do not establish HTTPS interoperability across all servers, proxy environments, authentication schemes, or target operating systems.

## Implementation and provenance

- [`http_client.rs`](../../crates/rc-app/src/http_client.rs) contains the parser, network operation, cancellation, cached request state, and local-server tests.
- [`http_window.rs`](../../crates/rc-app/src/ui/http_window.rs) contains request selection, response UI, and the headless interaction test.
- `app.rs`, `toolwindow.rs`, and the UI dispatcher connect the client to the application lifecycle and existing dock.

The design reuses [CrabCraft at commit 475ade7b5ef0020c953f68a51f1b897380c1ea06](https://github.com/RafaelMendes1407/CrabCraft/tree/475ade7b5ef0020c953f68a51f1b897380c1ea06)'s approach to explicit execution, cancellation, and bounded response retention. CrabCraft is available under MIT or Apache-2.0 and retains DotCraft's original notices. This implementation does not import CrabCraft's environment, OAuth, response-script, or syntax-highlighting subsystems. HTTP transport uses the existing dependency ecosystem's reqwest 0.13.5 with rustls.

This completes the basic in-editor HTTP request execution slice of the web-development backlog. Advanced HTTP Client integrations remain separate work.
