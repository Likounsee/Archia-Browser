#![no_main]

use archia_browser::net::{CookieJar, HttpMethod, Url};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(attribute) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(url) = Url::parse("https://sub.example.invalid/account/page") else {
        return;
    };
    let Ok(first_party) = Url::parse("https://example.invalid/") else {
        return;
    };

    let mut jar = CookieJar::new();
    jar.store(&url, attribute);
    let _ = jar.header_for_context(&url, Some(&first_party), true, HttpMethod::Get);
});
