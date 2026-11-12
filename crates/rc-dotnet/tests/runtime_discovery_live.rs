use std::path::Path;

use rc_dotnet::sdk::{Dotnet, parse_runtimes};
use rc_dotnet::ProjectInfo;

#[test]
fn runtime_parser_accepts_cli_whitespace_and_rejects_extra_tokens() {
    let text = "Microsoft.NETCore.App\t8.0.34 [/sdk/shared/Microsoft.NETCore.App]\nMicrosoft.AspNetCore.App   8.0.34 [/sdk/shared/Microsoft.AspNetCore.App]\nBad 8.0 extra [/sdk/shared/Bad]\n";
    let runtimes = parse_runtimes(text);
    assert_eq!(runtimes.len(), 2);
    assert_eq!(runtimes[0].name, "Microsoft.NETCore.App");
    assert_eq!(runtimes[0].version, "8.0.34");
    assert_eq!(runtimes[1].name, "Microsoft.AspNetCore.App");
}

#[test]
#[ignore = "requires a local .NET runtime via DOTCRAFT_TEST_DOTNET"]
fn installed_runtimes_cover_real_shop_api_target() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET").expect("set DOTCRAFT_TEST_DOTNET to a real dotnet host");
    let dotnet = Dotnet::locate(Some(Path::new(&host))).unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Api/Shop.Api.csproj");
    let info = ProjectInfo::load(&project).unwrap();
    assert!(info.is_web());
    assert_eq!(info.target_frameworks, ["net8.0"]);
    let runtimes = dotnet.list_runtimes().unwrap();
    assert!(runtimes.iter().all(|r| r.path.join(&r.version).is_dir()));
    for name in ["Microsoft.NETCore.App", "Microsoft.AspNetCore.App"] {
        assert!(runtimes.iter().any(|r| r.name == name && r.version.starts_with("8.0.")), "missing {name}: {runtimes:?}");
    }
    if let Ok(expected) = std::env::var("DOTCRAFT_TEST_RUNTIME_VERSION") {
        assert!(runtimes.iter().filter(|r| r.name == "Microsoft.AspNetCore.App" || r.name == "Microsoft.NETCore.App").all(|r| r.version == expected));
    }
}
