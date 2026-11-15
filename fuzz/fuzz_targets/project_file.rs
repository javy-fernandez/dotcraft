#![no_main]

use libfuzzer_sys::fuzz_target;

// Project files come from untrusted repositories and are parsed while they are being edited.
fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }
    let Ok(text) = std::str::from_utf8(data) else { return };
    let _ = rc_dotnet::project::ProjectInfo::parse(text, std::path::Path::new("Fuzz.csproj"));
    let _ = rc_dotnet::project_xml::package_items(text);
    let _ = rc_dotnet::project_xml::context_at(text, text.len() / 2);
    let _ = rc_dotnet::launch::LaunchSettings::parse(text);
});
