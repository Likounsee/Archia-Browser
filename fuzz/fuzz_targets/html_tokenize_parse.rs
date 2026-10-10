#![no_main]

use archia_browser::html::{parse, HtmlTokenizer};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let tokens = HtmlTokenizer::tokenize(input);
    let _ = parse(&tokens);
});
