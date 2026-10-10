#![no_main]

use archia_browser::net::transport::parse_http_response;
use libfuzzer_sys::fuzz_target;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_RESPONSE_BYTES {
        return;
    }
    let _ = parse_http_response(data, MAX_RESPONSE_BYTES, MAX_HEADER_BYTES);
});
