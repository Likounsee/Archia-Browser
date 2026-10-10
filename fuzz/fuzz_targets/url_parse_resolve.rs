#![no_main]

use archia_browser::net::Url;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    let _ = Url::parse(input);

    // Exercise reference resolution with both network and local-file bases:
    // malformed references must return an error, never panic.
    if let Ok(base) = Url::parse("https://example.invalid/a/b/index.html?old=1#fragment") {
        let _ = base.resolve(input);
    }
    if let Ok(base) = Url::parse("file:///C:/Users/example/index.html") {
        let _ = base.resolve(input);
    }
});
