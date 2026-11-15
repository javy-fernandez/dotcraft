#![no_main]

use libfuzzer_sys::fuzz_target;

// Test and coverage reports are produced by external tools and read back from disk.
fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }
    let Ok(text) = std::str::from_utf8(data) else { return };
    let _ = rc_dotnet::trx::parse_trx(text);
    let _ = rc_dotnet::trx::parse_stack_trace(text);
    let _ = rc_dotnet::coverage::parse_cobertura(text);
});
