#![no_main]

use libfuzzer_sys::fuzz_target;
use megabase_server::gateway_component_for_target;

fuzz_target!(|data: &[u8]| {
    let _ = gateway_component_for_target(data);
});
