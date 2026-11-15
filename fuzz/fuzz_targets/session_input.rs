#![no_main]

use libfuzzer_sys::fuzz_target;

// Persisted session and settings JSON can be edited or corrupted outside DotCraft.
fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }
    if let Ok(session) = serde_json::from_slice::<rc_core::session::Session>(data) {
        for options in session.run_options.values() {
            let _ = options.env_pairs();
        }
    }
    let _ = serde_json::from_slice::<rc_core::settings::Settings>(data);
});
