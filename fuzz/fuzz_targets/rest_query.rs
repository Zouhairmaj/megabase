#![no_main]

use libfuzzer_sys::fuzz_target;
use megabase_rest::walk_query_string;

fuzz_target!(|data: &[u8]| {
    if let Ok(query) = std::str::from_utf8(data) {
        let _ = walk_query_string(query);
    }
});
