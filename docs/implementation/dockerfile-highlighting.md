# Dockerfile syntax highlighting (#122, partial)

Dockerfile, Dockerfile variants such as `Dockerfile.dev`, Containerfile, and Containerfile variants use DotCraft's native lexical highlighter. It marks instructions, comments, quoted strings, and variable references, and enables `#` line comments. The lexer runs over the displayed lines without invoking Docker.

Validation: two focused `rc-syntax` tests cover filename detection and highlighted Dockerfile content. The implementation was written for DotCraft; no external code or assets were copied.

Docker run and debug configurations, compose support, and the Services window remain for later work on #122.
