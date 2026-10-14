use std::collections::BTreeMap;

fn is_html_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\u{000C}' | '\r' | ' ')
}

// Bound scanning and retained token data for untrusted documents.
const MAX_HTML_INPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_HTML_TOKENS: usize = 100_000;
const MAX_HTML_TAG_NAME_BYTES: usize = 1_024;
const MAX_HTML_ATTRIBUTES_PER_TAG: usize = 256;
const MAX_HTML_ATTRIBUTE_NAME_BYTES: usize = 1_024;
const MAX_HTML_ATTRIBUTE_VALUE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlToken {
    Doctype(String),
    StartTag {
        name: String,
        attributes: BTreeMap<String, String>,
        self_closing: bool,
    },
    EndTag(String),
    Text(String),
    Comment(String),
}

#[derive(Debug, Default)]
pub struct HtmlTokenizer;

impl HtmlTokenizer {
    pub fn tokenize(input: &str) -> Vec<HtmlToken> {
        // Truncate oversized documents only at a UTF-8 character boundary.
        let input = if input.len() > MAX_HTML_INPUT_BYTES {
            let mut end = MAX_HTML_INPUT_BYTES;
            while !input.is_char_boundary(end) {
                end -= 1;
            }
            &input[..end]
        } else {
            input
        };
        let mut tokens = Vec::new();
        let mut cursor = 0;
        let bytes = input.as_bytes();

        while cursor < bytes.len() && tokens.len() < MAX_HTML_TOKENS {
            if bytes[cursor] != b'<' {
                let end = input[cursor..]
                    .find('<')
                    .map_or(input.len(), |offset| cursor + offset);
                let text = &input[cursor..end];
                if !text.is_empty()
                    && !try_push_token(
                        &mut tokens,
                        HtmlToken::Text(decode_character_references(text)),
                    )
                {
                    return tokens;
                }
                cursor = end;
                continue;
            }

            if input[cursor..].starts_with("<!--") {
                if let Some(offset) = input[cursor + 4..].find("-->") {
                    let end = cursor + 4 + offset;
                    if !try_push_token(
                        &mut tokens,
                        HtmlToken::Comment(input[cursor + 4..end].to_owned()),
                    ) {
                        return tokens;
                    }
                    cursor = end + 3;
                    continue;
                }

                if !try_push_token(
                    &mut tokens,
                    HtmlToken::Comment(input[cursor + 4..].to_owned()),
                ) {
                    return tokens;
                }
                break;
            }

            let Some(offset) = find_tag_end(&input[cursor..]) else {
                if !try_push_token(&mut tokens, HtmlToken::Text(input[cursor..].to_owned())) {
                    return tokens;
                }
                break;
            };
            let end = cursor + offset;
            let inside = input[cursor + 1..end].trim();

            // HTML is untrusted input. Do not slice at an arbitrary byte
            // offset before checking the prefix: a malformed tag can start
            // with multi-byte UTF-8 and otherwise panic the tokenizer.
            if inside
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("!doctype"))
                && matches!(
                    inside.as_bytes().get(8),
                    None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
                )
            {
                if !try_push_token(
                    &mut tokens,
                    HtmlToken::Doctype(inside[8..].trim().to_owned()),
                ) {
                    return tokens;
                }
            } else if inside.starts_with('!') || inside.starts_with('?') {
                // In HTML syntax, unknown declarations, CDATA sections outside
                // foreign content, and processing-instruction-like markup are
                // bogus comments rather than element start tags. A leading '?'
                // is part of the comment data in HTML's bogus-comment state.
                let comment = if inside.starts_with('?') {
                    inside.to_owned()
                } else {
                    inside[1..].to_owned()
                };
                if !try_push_token(&mut tokens, HtmlToken::Comment(comment)) {
                    return tokens;
                }
            } else if let Some(name) = inside.strip_prefix('/') {
                // Keep closing-tag work bounded too; otherwise oversized names
                // bypass the start-tag cap and are retained as large tokens.
                let name = name.trim();
                if name.len() <= MAX_HTML_TAG_NAME_BYTES
                    && !try_push_token(&mut tokens, HtmlToken::EndTag(name.to_ascii_lowercase()))
                {
                    return tokens;
                }
            } else if let Some((name, attributes, self_closing)) = parse_start_tag(inside) {
                let is_raw_text = matches!(name.as_str(), "script" | "style");
                let is_rcdata = matches!(name.as_str(), "title" | "textarea");
                let is_plaintext = name == "plaintext";
                if !try_push_token(
                    &mut tokens,
                    HtmlToken::StartTag {
                        name: name.clone(),
                        attributes,
                        self_closing,
                    },
                ) {
                    return tokens;
                }

                if is_plaintext {
                    if end + 1 < input.len()
                        && tokens.len() < MAX_HTML_TOKENS
                        && !try_push_token(
                            &mut tokens,
                            HtmlToken::Text(input[end + 1..].replace('\0', "\u{FFFD}")),
                        )
                    {
                        return tokens;
                    }
                    break;
                }

                if is_raw_text || is_rcdata {
                    if let Some((text_end, close_end)) = find_raw_text_end(input, end + 1, &name) {
                        if text_end > end + 1 && tokens.len() < MAX_HTML_TOKENS {
                            let text = &input[end + 1..text_end];
                            let decoded = if is_rcdata {
                                decode_character_references(text)
                            } else {
                                text.replace('\0', "\u{FFFD}")
                            };
                            if !try_push_token(&mut tokens, HtmlToken::Text(decoded)) {
                                return tokens;
                            }
                        }
                        // The raw-text scanner consumes the closing tag's
                        // bytes, but tree construction still needs its token to
                        // pop the corresponding open element from the DOM stack.
                        if tokens.len() < MAX_HTML_TOKENS
                            && !try_push_token(&mut tokens, HtmlToken::EndTag(name.clone()))
                        {
                            return tokens;
                        }
                        cursor = close_end;
                        continue;
                    }

                    // Raw-text and RCDATA elements consume the remainder when
                    // their end tag is missing; inner markup must not be parsed
                    // as document elements.
                    if end + 1 < input.len() && tokens.len() < MAX_HTML_TOKENS {
                        let text = &input[end + 1..];
                        let decoded = if is_rcdata {
                            decode_character_references(text)
                        } else {
                            text.replace('\0', "\u{FFFD}")
                        };
                        if !try_push_token(&mut tokens, HtmlToken::Text(decoded)) {
                            return tokens;
                        }
                    }
                    break;
                }
            }
            cursor = end + 1;
        }
        tokens
    }
}

/// Retain a token only when its storage can grow without an infallible Vec
/// allocation. Token payload construction may itself allocate; input and
/// per-token limits remain the independent bounds for those payloads.
fn try_push_token(tokens: &mut Vec<HtmlToken>, token: HtmlToken) -> bool {
    if tokens.len() >= MAX_HTML_TOKENS || tokens.try_reserve(1).is_err() {
        return false;
    }
    tokens.push(token);
    true
}

fn find_tag_end(input: &str) -> Option<usize> {
    let mut quote = None;
    for (index, ch) in input.char_indices() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

fn parse_start_tag(input: &str) -> Option<(String, BTreeMap<String, String>, bool)> {
    let mut chars = input.chars().peekable();
    let mut name = String::new();
    while let Some(&ch) = chars.peek() {
        if is_html_whitespace(ch) || ch == '/' {
            break;
        }
        if name.len().saturating_add(ch.len_utf8()) > MAX_HTML_TAG_NAME_BYTES {
            return None;
        }
        name.push(ch.to_ascii_lowercase());
        chars.next();
    }
    if name.is_empty() {
        return None;
    }

    let mut attributes = BTreeMap::new();
    let mut parsed_attribute_count = 0usize;
    let mut self_closing = false;
    loop {
        while chars.peek().is_some_and(|c| is_html_whitespace(*c)) {
            chars.next();
        }
        if chars.peek().is_none() || parsed_attribute_count >= MAX_HTML_ATTRIBUTES_PER_TAG {
            break;
        }
        if chars.peek() == Some(&'/') {
            chars.next();
            self_closing = true;
            while chars.peek().is_some_and(|c| is_html_whitespace(*c)) {
                chars.next();
            }
            break;
        }

        // Bound work as well as retained map entries. Duplicate attributes
        // do not increase BTreeMap::len(), so using that length alone lets an
        // attacker force a full scan of millions of duplicate attributes.
        parsed_attribute_count += 1;
        let mut attr_name = String::new();
        let mut attr_name_too_long = false;
        while let Some(&ch) = chars.peek() {
            if is_html_whitespace(ch) || matches!(ch, '=' | '/') {
                break;
            }
            if attr_name.len() + ch.len_utf8() <= MAX_HTML_ATTRIBUTE_NAME_BYTES {
                attr_name.push(ch.to_ascii_lowercase());
            } else {
                attr_name_too_long = true;
            }
            chars.next();
        }
        if attr_name.is_empty() && !attr_name_too_long {
            chars.next();
            continue;
        }
        while chars.peek().is_some_and(|c| is_html_whitespace(*c)) {
            chars.next();
        }

        let mut value = String::new();
        let mut value_too_long = false;
        if chars.peek() == Some(&'=') {
            chars.next();
            while chars.peek().is_some_and(|c| is_html_whitespace(*c)) {
                chars.next();
            }
            let quote = chars.peek().copied().filter(|c| *c == '\'' || *c == '"');
            if let Some(q) = quote {
                chars.next();
                while let Some(&ch) = chars.peek() {
                    chars.next();
                    if ch == q {
                        break;
                    }
                    if value.len() + ch.len_utf8() <= MAX_HTML_ATTRIBUTE_VALUE_BYTES {
                        value.push(ch);
                    } else {
                        value_too_long = true;
                    }
                }
            } else {
                // Slashes are valid inside unquoted attribute values
                // (for example, href=https://example.test/path). A slash
                // only marks a self-closing tag when encountered between
                // attributes, not while consuming a value.
                while let Some(&ch) = chars.peek() {
                    if is_html_whitespace(ch) {
                        break;
                    }
                    if value.len().saturating_add(ch.len_utf8()) <= MAX_HTML_ATTRIBUTE_VALUE_BYTES {
                        value.push(ch);
                    } else {
                        value_too_long = true;
                    }
                    chars.next();
                }
            }
        }
        // Discard overlong names/values instead of retaining a truncated
        // security-sensitive attribute (for example, href or src).
        if attr_name_too_long || value_too_long {
            continue;
        }
        // HTML ignores duplicate attributes after the first occurrence.
        // Keeping the first value avoids parser differentials where policy
        // checks and later DOM consumers could interpret different values.
        attributes
            .entry(attr_name)
            .or_insert_with(|| decode_character_references(&value));
    }
    Some((name, attributes, self_closing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_and_attribute_separation_uses_only_html_whitespace() {
        let tokens = HtmlTokenizer::tokenize("<div id='first'\\u{2003}class='second'></div>");
        let HtmlToken::StartTag { name, attributes, .. } = &tokens[0] else {
            panic!("expected start tag");
        };

        assert_eq!(name, "div");
        assert_eq!(attributes.get("id").map(String::as_str), Some("first"));
        assert!(!attributes.contains_key("class"));
        assert_eq!(
            attributes.get("\\u{2003}class").map(String::as_str),
            Some("second")
        );
    }

    #[test]
    fn doctype_keyword_requires_a_delimiter() {
        let tokens = HtmlTokenizer::tokenize("<!doctypefoo><p>ok</p>");
        assert!(matches!(&tokens[0], HtmlToken::Comment(value) if value == "doctypefoo"));
        assert!(matches!(&tokens[1], HtmlToken::StartTag { name, .. } if name == "p"));
    }

    #[test]
    fn unknown_declarations_and_processing_instructions_become_comments() {
        let tokens = HtmlTokenizer::tokenize(
            "<!DOCTYPE html><!unknown value><?xml version='1.0'?><div>ok</div>",
        );
        assert!(matches!(&tokens[0], HtmlToken::Doctype(_)));
        assert!(matches!(&tokens[1], HtmlToken::Comment(value) if value == "unknown value"));
        assert!(matches!(&tokens[2], HtmlToken::Comment(value) if value == "?xml version='1.0'?"));
        assert!(matches!(&tokens[3], HtmlToken::StartTag { name, .. } if name == "div"));
    }

    #[test]
    fn cdata_syntax_outside_foreign_content_is_not_an_element() {
        let tokens = HtmlTokenizer::tokenize("<![CDATA[not HTML CDATA]]><p>text</p>");
        assert!(
            matches!(&tokens[0], HtmlToken::Comment(value) if value == "[CDATA[not HTML CDATA]]")
        );
        assert!(matches!(&tokens[1], HtmlToken::StartTag { name, .. } if name == "p"));
    }

    #[test]
    fn malformed_unicode_tag_does_not_panic() {
        let tokens = HtmlTokenizer::tokenize("<ééééé>");
        assert!(matches!(tokens.as_slice(), [HtmlToken::StartTag { name, .. }] if name == "ééééé"));
    }

    #[test]
    fn oversized_input_is_truncated_at_a_utf8_boundary() {
        let mut input = "a".repeat(MAX_HTML_INPUT_BYTES);
        input.push('é');
        let tokens = HtmlTokenizer::tokenize(&input);
        assert_eq!(tokens.len(), 1);
        let HtmlToken::Text(text) = &tokens[0] else {
            panic!("expected text token");
        };
        assert_eq!(text.len(), MAX_HTML_INPUT_BYTES);
    }

    #[test]
    fn token_count_is_bounded_for_many_small_tags() {
        let input = "<b></b>".repeat(MAX_HTML_TOKENS);
        let tokens = HtmlTokenizer::tokenize(&input);
        assert_eq!(tokens.len(), MAX_HTML_TOKENS);
    }

    #[test]
    fn raw_text_cannot_push_token_count_over_limit() {
        let repeated_tags = "<b></b>".repeat((MAX_HTML_TOKENS - 1) / 2);
        let input = format!("{repeated_tags}x<script>payload</script>");
        let tokens = HtmlTokenizer::tokenize(&input);
        assert_eq!(tokens.len(), MAX_HTML_TOKENS);
    }

    #[test]
    fn title_and_textarea_use_rcdata_without_parsing_inner_markup() {
        let tokens = HtmlTokenizer::tokenize(
            "<title>A &amp; <b>title</title><textarea>x &lt; <b>y</textarea>",
        );
        assert!(matches!(&tokens[1], HtmlToken::Text(text) if text == "A & <b>title"));
        assert!(matches!(&tokens[4], HtmlToken::Text(text) if text == "x < <b>y"));
        assert!(matches!(&tokens[2], HtmlToken::EndTag(name) if name == "title"));
        assert!(matches!(&tokens[5], HtmlToken::EndTag(name) if name == "textarea"));
        assert_eq!(tokens.len(), 6);
    }

    #[test]
    fn plaintext_consumes_markup_through_eof() {
        let tokens = HtmlTokenizer::tokenize("<plaintext>hello <b>world</b>");
        assert!(matches!(&tokens[1], HtmlToken::Text(text) if text == "hello <b>world</b>"));
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn many_small_raw_text_elements_preserve_each_body() {
        let input = "<script>a</script><style>b</style>".repeat(2_000);
        let tokens = HtmlTokenizer::tokenize(&input);
        assert_eq!(tokens.len(), 12_000);
        assert!(matches!(&tokens[0], HtmlToken::StartTag { name, .. } if name == "script"));
        assert_eq!(tokens[1], HtmlToken::Text("a".to_owned()));
        assert!(matches!(&tokens[2], HtmlToken::EndTag(name) if name == "script"));
        assert!(matches!(&tokens[3], HtmlToken::StartTag { name, .. } if name == "style"));
        assert_eq!(tokens[4], HtmlToken::Text("b".to_owned()));
        assert!(matches!(&tokens[5], HtmlToken::EndTag(name) if name == "style"));
    }

    #[test]
    fn tokenizes_basic_document() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        assert_eq!(
            tokens,
            vec![
                HtmlToken::StartTag {
                    name: "html".into(),
                    attributes: BTreeMap::new(),
                    self_closing: false
                },
                HtmlToken::StartTag {
                    name: "body".into(),
                    attributes: BTreeMap::new(),
                    self_closing: false
                },
                HtmlToken::Text("Hello".into()),
                HtmlToken::EndTag("body".into()),
                HtmlToken::EndTag("html".into()),
            ]
        );
    }

    #[test]
    fn caps_attributes_retained_from_a_single_tag() {
        let attributes = (0..(MAX_HTML_ATTRIBUTES_PER_TAG + 100))
            .map(|index| format!(" a{index}=x"))
            .collect::<String>();
        let tokens = HtmlTokenizer::tokenize(&format!("<div{attributes}>"));

        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else {
            panic!("expected start tag");
        };
        assert_eq!(attributes.len(), MAX_HTML_ATTRIBUTES_PER_TAG);
    }

    #[test]
    fn duplicate_attributes_count_toward_per_tag_parse_limit() {
        let duplicates = (0..(MAX_HTML_ATTRIBUTES_PER_TAG + 20))
            .map(|_| " id=first")
            .collect::<String>();
        let input = format!("<div{duplicates} title=late>");
        let tokens = HtmlTokenizer::tokenize(&input);

        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else {
            panic!("expected start tag");
        };
        assert_eq!(attributes.get("id").map(String::as_str), Some("first"));
        assert!(!attributes.contains_key("title"));
    }

    #[test]
    fn discards_overlong_attribute_names_and_values() {
        let long_name = "n".repeat(MAX_HTML_ATTRIBUTE_NAME_BYTES + 1);
        let long_value = "v".repeat(MAX_HTML_ATTRIBUTE_VALUE_BYTES + 1);
        let input = format!("<a href=\"safe\" {long_name}=x title=\"{long_value}\">");
        let tokens = HtmlTokenizer::tokenize(&input);

        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else {
            panic!("expected start tag");
        };
        assert_eq!(attributes.get("href").map(String::as_str), Some("safe"));
        assert!(!attributes.contains_key(&long_name));
        assert!(!attributes.contains_key("title"));
    }

    #[test]
    fn discards_overlong_tag_names() {
        let name = "x".repeat(MAX_HTML_TAG_NAME_BYTES + 1);
        let tokens = HtmlTokenizer::tokenize(&format!("<{name} id=retained>"));
        assert!(tokens.is_empty());
    }

    #[test]
    fn discards_overlong_end_tag_names_but_keeps_following_markup() {
        let name = "x".repeat(MAX_HTML_TAG_NAME_BYTES + 1);
        let tokens = HtmlTokenizer::tokenize(&format!("</{name}><p>retained</p>"));

        assert_eq!(
            tokens,
            vec![
                HtmlToken::StartTag {
                    name: "p".to_owned(),
                    attributes: BTreeMap::new(),
                    self_closing: false,
                },
                HtmlToken::Text("retained".to_owned()),
                HtmlToken::EndTag("p".to_owned()),
            ]
        );
    }

    #[test]
    fn keeps_first_duplicate_attribute_value() {
        let tokens = HtmlTokenizer::tokenize(
            r#"<a href="https://safe.example/" href="https://other.example/">link</a>"#,
        );
        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else {
            panic!("expected anchor start tag");
        };
        assert_eq!(
            attributes.get("href").map(String::as_str),
            Some("https://safe.example/")
        );
    }

    #[test]
    fn preserves_slashes_in_unquoted_attribute_values() {
        let tokens = HtmlTokenizer::tokenize(
            "<a href=https://example.test/path>link</a><img src=/assets/icon.svg>",
        );
        let HtmlToken::StartTag {
            attributes: link, ..
        } = &tokens[0]
        else {
            panic!("expected anchor start tag");
        };
        assert_eq!(
            link.get("href").map(String::as_str),
            Some("https://example.test/path")
        );
        let HtmlToken::StartTag {
            attributes: image, ..
        } = &tokens[3]
        else {
            panic!("expected image start tag");
        };
        assert_eq!(
            image.get("src").map(String::as_str),
            Some("/assets/icon.svg")
        );
    }

    #[test]
    fn tokenizes_attributes_and_quoted_gt() {
        let tokens =
            HtmlTokenizer::tokenize(r#"<div id="main" class='card active' title="a > b">x</div>"#);
        let HtmlToken::StartTag { attributes, .. } = &tokens[0] else {
            panic!("expected start tag");
        };
        assert_eq!(attributes.get("id"), Some(&"main".to_string()));
        assert_eq!(attributes.get("class"), Some(&"card active".to_string()));
        assert_eq!(attributes.get("title"), Some(&"a > b".to_string()));
    }

    #[test]
    fn tokenizes_comments_and_doctype() {
        let tokens = HtmlTokenizer::tokenize("<!DOCTYPE html><!--x-->");
        assert_eq!(tokens[0], HtmlToken::Doctype("html".into()));
        assert_eq!(tokens[1], HtmlToken::Comment("x".into()));
    }
}

fn find_raw_text_end(input: &str, start: usize, tag_name: &str) -> Option<(usize, usize)> {
    let bytes = input.as_bytes();
    let marker = format!("</{tag_name}");
    let marker_bytes = marker.as_bytes();
    let mut search_from = start;

    // Search the original bytes instead of lowercasing the entire remaining
    // document for every <script>/<style>. With many small raw-text elements,
    // repeated tail allocations/copies made this otherwise bounded tokenizer
    // quadratic in total input size.
    while search_from <= bytes.len().saturating_sub(marker_bytes.len()) {
        let relative = bytes[search_from..]
            .windows(marker_bytes.len())
            .position(|window| window.eq_ignore_ascii_case(marker_bytes))?;
        let text_end = search_from + relative;
        let after_name = text_end + marker_bytes.len();
        let next = bytes.get(after_name).copied();

        // A raw-text end tag must end its name here. Without this boundary
        // check, </scripture> would incorrectly terminate a <script> block.
        if next.is_some_and(|byte| byte == b'>' || byte == b'/' || byte.is_ascii_whitespace()) {
            if let Some(offset) = input[after_name..].find('>') {
                return Some((text_end, after_name + offset + 1));
            }
            return None;
        }

        search_from = after_name;
    }
    None
}

fn decode_numeric_character_reference(codepoint: u32) -> char {
    // HTML replaces null and invalid Unicode scalar values in numeric
    // character references rather than preserving the source spelling.
    if codepoint == 0 || char::from_u32(codepoint).is_none() {
        return '\u{FFFD}';
    }

    // HTML's numeric-reference algorithm remaps these legacy Windows-1252
    // control values to their printable characters.
    const WINDOWS_1252_REPLACEMENTS: &[(u32, char)] = &[
        (0x80, '€'),
        (0x82, '‚'),
        (0x83, 'ƒ'),
        (0x84, '„'),
        (0x85, '…'),
        (0x86, '†'),
        (0x87, '‡'),
        (0x88, 'ˆ'),
        (0x89, '‰'),
        (0x8A, 'Š'),
        (0x8B, '‹'),
        (0x8C, 'Œ'),
        (0x8E, 'Ž'),
        (0x91, '‘'),
        (0x92, '’'),
        (0x93, '“'),
        (0x94, '”'),
        (0x95, '•'),
        (0x96, '–'),
        (0x97, '—'),
        (0x98, '˜'),
        (0x99, '™'),
        (0x9A, 'š'),
        (0x9B, '›'),
        (0x9C, 'œ'),
        (0x9E, 'ž'),
        (0x9F, 'Ÿ'),
    ];
    if let Some((_, replacement)) = WINDOWS_1252_REPLACEMENTS
        .iter()
        .find(|(value, _)| *value == codepoint)
    {
        return *replacement;
    }

    // The remaining C1 controls are retained by the HTML algorithm.
    char::from_u32(codepoint).unwrap_or('\u{FFFD}')
}

fn decode_character_references(input: &str) -> String {
    // HTML input preprocessing replaces literal U+0000 with U+FFFD, even
    // when the input contains no character references.
    if !input.contains('&') {
        return input.replace('\0', "\u{FFFD}");
    }
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(relative) = input[cursor..].find('&') {
        let start = cursor + relative;
        output.push_str(&input[cursor..start]);

        // Numeric character references may omit their trailing semicolon in HTML.
        // Consume only valid digits so following text remains untouched.
        let reference_start = start + 1;
        if input[reference_start..].starts_with('#') {
            let digits_start = reference_start + 1;
            let (radix, number_start) = if input[digits_start..].starts_with('x')
                || input[digits_start..].starts_with('X')
            {
                (16, digits_start + 1)
            } else {
                (10, digits_start)
            };
            let digits_end = input[number_start..]
                .char_indices()
                .take_while(|(_, ch)| ch.is_digit(radix))
                .map(|(offset, ch)| offset + ch.len_utf8())
                .last()
                .map(|length| number_start + length);
            if let Some(end) = digits_end {
                if end > number_start {
                    let has_semicolon = input[end..].starts_with(';');
                    let digits = &input[number_start..end];
                    let decoded = u32::from_str_radix(digits, radix)
                        .map(decode_numeric_character_reference)
                        .unwrap_or('\u{FFFD}');
                    output.push(decoded);
                    cursor = end + usize::from(has_semicolon);
                    continue;
                }
            }
        }

        let Some(end_relative) = input[start + 1..].find(';') else {
            output.push_str(&input[start..]);
            return output.replace('\0', "\u{FFFD}");
        };
        // An ampersand before the semicolon starts a separate candidate
        // reference; do not swallow a valid reference that follows an unknown one.
        if let Some(next_ampersand) = input[start + 1..start + 1 + end_relative].find('&') {
            let next_start = start + 1 + next_ampersand;
            // Preserve the entire unknown candidate, including its name.
            output.push_str(&input[start..next_start]);
            cursor = next_start;
            continue;
        }
        let end = start + 1 + end_relative;
        let reference = &input[start + 1..end];

        let decoded = match reference {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            // Common named references used by ordinary prose and typography.
            // This is intentionally a bounded subset, not the full WHATWG table.
            "nbsp" => Some('\u{00A0}'),
            "copy" => Some('\u{00A9}'),
            "reg" => Some('\u{00AE}'),
            "trade" => Some('\u{2122}'),
            "hellip" => Some('\u{2026}'),
            "mdash" => Some('\u{2014}'),
            "ndash" => Some('\u{2013}'),
            "lsquo" => Some('\u{2018}'),
            "rsquo" => Some('\u{2019}'),
            "ldquo" => Some('\u{201C}'),
            "rdquo" => Some('\u{201D}'),
            "bull" => Some('\u{2022}'),
            "euro" => Some('\u{20AC}'),
            "times" => Some('\u{00D7}'),
            "divide" => Some('\u{00F7}'),
            "plusmn" => Some('\u{00B1}'),
            "cent" => Some('\u{00A2}'),
            "pound" => Some('\u{00A3}'),
            "yen" => Some('\u{00A5}'),
            _ if reference.starts_with("#x") || reference.starts_with("#X") => {
                u32::from_str_radix(&reference[2..], 16)
                    .ok()
                    .map(decode_numeric_character_reference)
            }
            _ if reference.starts_with('#') => reference[1..]
                .parse::<u32>()
                .ok()
                .map(decode_numeric_character_reference),
            _ => None,
        };

        if let Some(character) = decoded {
            output.push(character);
            cursor = end + 1;
        } else {
            output.push_str(&input[start..=end]);
            cursor = end + 1;
        }
    }

    output.push_str(&input[cursor..]);
    output.replace('\0', "\u{FFFD}")
}

#[cfg(test)]
mod character_reference_tests {
    use super::*;

    #[test]
    fn decodes_basic_character_references() {
        assert_eq!(
            decode_character_references("&lt;div&gt; &amp; &#65; &#x1f600;"),
            "<div> & A 😀"
        );
    }

    #[test]
    fn decodes_common_named_typographic_references() {
        assert_eq!(
            decode_character_references(
                "&nbsp;&copy; &reg; &trade; &hellip; &mdash; &ndash; &lsquo;x&rsquo; &ldquo;y&rdquo; &bull; &euro; &times; &divide; &plusmn; &cent; &pound; &yen;"
            ),
            "\u{00A0}\u{00A9} \u{00AE} \u{2122} \u{2026} \u{2014} \u{2013} \u{2018}x\u{2019} \u{201C}y\u{201D} \u{2022} \u{20AC} \u{00D7} \u{00F7} \u{00B1} \u{00A2} \u{00A3} \u{00A5}"
        );
    }

    #[test]
    fn replaces_invalid_numeric_character_references() {
        assert_eq!(
            decode_character_references("&#0; &#xD800; &#x110000;"),
            "\u{FFFD} \u{FFFD} \u{FFFD}"
        );
    }

    #[test]
    fn decodes_numeric_references_without_semicolons() {
        assert_eq!(
            decode_character_references("&#65 &#x1f600! &#0"),
            "A 😀! \u{FFFD}"
        );
    }

    #[test]
    fn replaces_overflowing_numeric_references() {
        assert_eq!(
            decode_character_references("&#999999999999999999999; &#xFFFFFFFFFFFFFFFF"),
            "\u{FFFD} \u{FFFD}"
        );
    }

    #[test]
    fn maps_legacy_numeric_control_references() {
        assert_eq!(decode_character_references("&#x80; &#x91;"), "€ ‘");
    }

    #[test]
    fn replaces_literal_nulls_in_text_and_attribute_values() {
        let tokens = HtmlTokenizer::tokenize("<p>a\0b</p><input value='x\0y'>");
        assert!(matches!(&tokens[1], HtmlToken::Text(text) if text == "a\u{FFFD}b"));
        let HtmlToken::StartTag { attributes, .. } = &tokens[3] else {
            panic!("expected input start tag");
        };
        assert_eq!(
            attributes.get("value").map(String::as_str),
            Some("x\u{FFFD}y")
        );
    }

    #[test]
    fn replaces_literal_nulls_inside_raw_text() {
        let tokens = HtmlTokenizer::tokenize("<script>a\0b</script>");
        assert!(matches!(&tokens[1], HtmlToken::Text(text) if text == "a\u{FFFD}b"));
    }

    #[test]
    fn decodes_valid_references_after_unknown_references() {
        assert_eq!(
            decode_character_references("&unknown&copy; &bad&amp;"),
            "&unknown© &bad&"
        );
    }

    #[test]
    fn leaves_unknown_references_untouched() {
        assert_eq!(decode_character_references("&unknown;"), "&unknown;");
    }

    #[test]
    fn preserves_raw_script_text() {
        let tokens = HtmlTokenizer::tokenize("<script>if (a < b) { c++; }</script>");
        assert!(matches!(
            tokens.get(1),
            Some(HtmlToken::Text(value)) if value.contains("a < b")
        ));
    }

    #[test]
    fn unterminated_raw_text_element_consumes_remainder_as_text() {
        let tokens = HtmlTokenizer::tokenize("<script>if (a < b) { const x = '<p>not markup</p>';");

        assert!(matches!(
            tokens.as_slice(),
            [
                HtmlToken::StartTag { name, .. },
                HtmlToken::Text(value)
            ] if name == "script" && value.contains("<p>not markup</p>")
        ));
    }

    #[test]
    fn raw_script_does_not_close_on_longer_tag_name_prefix() {
        let tokens = HtmlTokenizer::tokenize(
            "<script>const x = '</scripture>'; run();</script><p>after</p>",
        );
        assert!(matches!(
            tokens.get(1),
            Some(HtmlToken::Text(value)) if value.contains("</scripture>") && value.contains("run();")
        ));
        assert!(matches!(
            tokens.get(2),
            Some(HtmlToken::EndTag(name)) if name == "script"
        ));
        assert!(matches!(
            tokens.get(3),
            Some(HtmlToken::StartTag { name, .. }) if name == "p"
        ));
    }
}
