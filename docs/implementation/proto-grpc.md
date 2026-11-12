# Protocol Buffers and gRPC

DotCraft recognizes `.proto` files and highlights Protocol Buffer declarations, scalar types, RPC names, strings, numbers and comments. The Structure window lists messages, enums, services, oneofs and RPCs from the active document and navigates to their declarations. These declarations also appear in Go to Type and Go to Symbol when the `.proto` file is active. An RPC's Structure context menu opens it in the gRPC Client. The client can also select an RPC from the active `.proto` file or accept a fully qualified method name entered by the user.

The client accepts a host and port, JSON request body, request metadata, and a TLS/plaintext choice. It uses the active `.proto` file and its parent directory as an import path when selected; otherwise grpcurl uses server reflection. An optional import path supports schemas with imports rooted elsewhere; relative paths are resolved from the active `.proto` directory. A modified `.proto` file must be saved before it is used by grpcurl. The response or error appears in the gRPC Client window. Requests run on a worker thread, can be cancelled, and time out after 65 seconds. The JSON request is validated and limited to 1 MiB before starting a process.

The request invocation follows the [grpcurl project documentation](https://github.com/fullstorydev/grpcurl): `-d @` reads JSON from stdin, `-proto` and `-import-path` supply local schemas, and `-plaintext` selects unencrypted transport. The optional `grpcurl` executable must be available on `PATH`; DotCraft reports its absence. No grpcurl or Protocol Buffer source code or assets were copied.

Validation: compact offline `cargo check -p rc-app`, focused lexical highlighting, declaration navigation and gRPC argument tests, locale JSON parsing, and `git diff --check`. The validation environment has no `grpcurl` executable or running gRPC service, so a live network call was not tested.

Current scope covers syntax highlighting and declaration navigation in the active `.proto` document, plus unary and server-streaming calls through grpcurl. Cross-file semantic references and interactive client-streaming requests are not implemented.
