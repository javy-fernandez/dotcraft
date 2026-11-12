# B-11: launch settings profiles and browser URLs

## Behavior

`LaunchProfile::browser_url` now treats `launchUrl` as absolute only when a URI scheme followed by `://` appears at its beginning. A relative path such as `swagger?next=https://example.test` is joined to the profile's `applicationUrl`; the embedded `https://` in the query no longer makes it an absolute URL. Explicit absolute URLs and profiles with browser launch disabled keep their prior behavior.

## Validation

- `CARGO_TARGET_DIR=/tmp/dotcraft-launch-url-target cargo test -p rc-dotnet --test launch_profiles` — 2 passed on Linux.
- `CARGO_TARGET_DIR=/tmp/dotcraft-launch-url-target cargo test -p rc-dotnet launch::tests` — 2 existing launch parser tests passed.
- Real `Shop.Api/Properties/launchSettings.json` fixture retains its `http` Project profile before IIS Express, `http://localhost:5112/swagger` browser URL, Development environment variable, and project-folder loading path.
- Synthetic cases cover relative paths containing `://`, absolute HTTPS and custom schemes, invalid leading schemes, and disabled browser launch.
- `git diff --check` passed.

## Provenance and limits

This extends DotCraft's existing launch parser at upstream commit `becfb0fe316203df2d8d7c1e99c6a54bd37cd13b`; no external source or assets were copied. The default Project-profile ordering follows [Microsoft's ASP.NET Core environment documentation](https://learn.microsoft.com/aspnet/core/fundamentals/environments). Actual browser opening, SDK execution, and application UI selection were not exercised. URI scheme detection does not validate the remainder of an absolute URL. This is a tested B-11 parser and URL-planning scope, not full Rider parity.
