# B-12: Bounded NuGet search and feed responses

## Behavior

`NugetClient::search` now rejects queries over 256 bytes or containing control characters, `take` values outside 1–1000, and `skip` values above 3000 before contacting a feed. NuGet V3 text responses are limited to 4 MiB, both when `Content-Length` advertises the size and while receiving a streamed response with no length. Search query parameters remain URL encoded.

The page bounds follow [Microsoft's NuGet Search API documentation](https://learn.microsoft.com/en-us/nuget/api/search-query-service-resource), which specifies positive `take` values and documents nuget.org's 3000/1000 skip/take limits. No third-party code or assets were copied.

## Validation

- `CARGO_TARGET_DIR=/var/tmp/dotcraft-nuget-search-target cargo test -p rc-dotnet --test nuget_search_bounds`: two tests passed. A local fake V3 feed searches for the real Shop.Api project's `Swashbuckle.AspNetCore` package ID, checks encoded query and paging parameters, verifies invalid requests fail before network access, and rejects oversized responses with and without `Content-Length`.
- `CARGO_TARGET_DIR=/var/tmp/dotcraft-nuget-search-target cargo test -p rc-dotnet --lib`: 132 tests passed.

## Limits

The client uses nuget.org-oriented page limits for all configured feeds; a private feed that supports larger pages may be more restrictive here. The 4 MiB response bound also applies to service indexes and version lists. No live nuget.org call, package install, or desktop search interaction was exercised. B-12 remains partial beyond this tested client scope.
