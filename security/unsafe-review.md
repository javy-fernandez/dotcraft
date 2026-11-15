# Own-code `unsafe` and FFI review map

`cargo-geiger` produces the machine-readable dependency inventory. The local
audit also lists source locations in `unsafe-own-code.json`; that lexical list
includes tests and needs human review. The production groups below explain the
intended safety argument for each known block; they do not prove a block safe.

| Source locations | Purpose and safety condition | Review priority |
| --- | --- | --- |
| `rc-app/src/main.rs:93` | Set `PATH` before starting application threads; changing process environment later would race. | Medium |
| `rc-app/src/open_files.rs:41,42,48,51` | macOS Objective-C callback, nullable URL array pointer, ABI conversion and method registration. The declared encoding and callback signature must match; inspect on macOS when changing objc2. | High |
| `rc-app/src/ui/toolwindows.rs:610,621` | `localtime_r` and `GetTimeZoneInformation` fill zeroed structs owned by the caller. | Low |
| `rc-dotnet/src/process.rs:223,243` | Signal the process group created for a child by the shared process runner; a zero PID is rejected first. | Medium |
| `rc-dotnet/src/nuget_auth.rs:344,349,350` | Windows DPAPI decryption of a `nuget.config` password: input blobs point at live buffers; the output is copied once and released with `LocalFree`. | High |
| `rc-vcs/src/hosting/secrets.rs:79,96,112` | Windows Credential Manager read/write/delete with NUL-terminated wide strings. `get` builds a slice from `CredentialBlob`; confirm a zero-length credential cannot yield a null pointer before `from_raw_parts`. | High |
| `rc-vcs/src/git.rs:169` | Kill the Git child process group on drop; confirm group creation remains paired with this path. | Medium |

Test-only uses of `unsafe` set `DOTCRAFT_HOME` in isolated fixtures (`rc-core`
tests serialize this with `paths::TEST_HOME_LOCK`; `rc-app` tests use
`screens::HOME_LOCK`) and probe child liveness with `kill(pid, 0)`. They appear
in the generated source inventory and should be checked when those tests change.
Embedded C/C++ code (bundled SQLite, tree-sitter grammars) and operating-system
libraries are outside `cargo-geiger`'s Rust source scan.

The Objective-C ABI bridge, the Windows credential and DPAPI boundaries, and
process-group lifetimes are the first places for a manual platform review. This
inventory does not change the implementation or add lint suppressions.
