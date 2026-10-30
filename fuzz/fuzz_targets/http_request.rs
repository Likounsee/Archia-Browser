#![no_main]

use archia_browser::net::{validate_http_request, HttpMethod, Request, Url};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, raw)) = data.split_first() else {
        return;
    };

    let method = match selector % 8 {
        0 => HttpMethod::Get,
        1 => HttpMethod::Head,
        2 => HttpMethod::Post,
        3 => HttpMethod::Put,
        4 => HttpMethod::Delete,
        5 => HttpMethod::Patch,
        6 => HttpMethod::Options,
        _ => HttpMethod::Connect,
    };
    let url = match Url::parse("http://fuzz.invalid/") {
        Ok(url) => url,
        Err(_) => return,
    };
    let mut request = Request::new(url).with_method(method);

    // Use a deterministic selector bit to exercise body/method and framing
    // interactions. The fuzzer controls every byte but cannot trigger I/O.
    if selector & 0x80 != 0 {
        request.body = raw.to_vec();
    }

    // Request.headers is public; retain differently-cased duplicate names so
    // the validator is fuzzed against the same adversarial states callers can
    // construct without the normalizing builder API.
    for line in raw.split(|byte| *byte == b'\\n').take(300) {
        let Some(separator) = line.iter().position(|byte| *byte == b':') else {
            continue;
        };
        let name = String::from_utf8_lossy(&line[..separator]).into_owned();
        let value = String::from_utf8_lossy(&line[separator + 1..]).into_owned();
        request.headers.insert(name, value);
    }

    let _ = validate_http_request(&request);
});
