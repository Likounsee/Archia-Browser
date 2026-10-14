use archia_browser::net::{CookieJar, HttpMethod, Url};

fn url(value: &str) -> Url {
    Url::parse(value).expect("test URL must parse")
}

#[test]
fn insecure_origin_cannot_replace_or_delete_secure_cookie() {
    let mut jar = CookieJar::new();
    jar.store(&url("https://example.test/account/login"), "sid=trusted; Secure; Path=/account");
    jar.store(&url("http://example.test/account/logout"), "sid=attacker; Path=/account");
    jar.store(&url("http://example.test/account/logout"), "sid=; Max-Age=0; Path=/account");

    assert_eq!(
        jar.header_for_context(
            &url("https://example.test/account/profile"),
            Some(&url("https://example.test")),
            true,
            HttpMethod::Get,
        )
        .as_deref(),
        Some("sid=trusted"),
        "an insecure origin must not shadow or remove a secure cookie"
    );
}

#[test]
fn secure_cookie_is_never_sent_over_http() {
    let mut jar = CookieJar::new();
    jar.store(&url("https://example.test/"), "sid=secret; Secure; Path=/");

    assert_eq!(
        jar.header_for_context(
            &url("http://example.test/"),
            Some(&url("http://example.test")),
            true,
            HttpMethod::Get,
        ),
        None
    );
}

#[test]
fn host_prefix_requires_secure_host_only_root_cookie() {
    let mut jar = CookieJar::new();
    let origin = url("https://example.test/");

    jar.store(&origin, "__Host-sid=good; Secure; Path=/");
    jar.store(
        &origin,
        "__Host-sid=bad; Secure; Domain=example.test; Path=/",
    );

    assert_eq!(
        jar.header_for_context(
            &origin,
            Some(&origin),
            true,
            HttpMethod::Get,
        )
        .as_deref(),
        Some("__Host-sid=good")
    );
}

#[test]
fn insecure_samesite_none_cookie_is_rejected() {
    let mut jar = CookieJar::new();
    let origin = url("https://example.test/");

    jar.store(&origin, "cross=bad; SameSite=None; Path=/");

    assert_eq!(
        jar.header_for_context(&origin, Some(&origin), true, HttpMethod::Get),
        None
    );
}

#[test]
fn domain_cookie_does_not_match_lookalike_suffix() {
    let mut jar = CookieJar::new();
    jar.store(
        &url("https://example.test/"),
        "session=valid; Domain=example.test; Path=/",
    );

    assert_eq!(
        jar.header_for_context(
            &url("https://notexample.test/"),
            Some(&url("https://notexample.test/")),
            true,
            HttpMethod::Get,
        ),
        None
    );
}

#[test]
fn file_urls_cannot_seed_network_cookie_state() {
    let mut jar = CookieJar::new();
    jar.store(&url("file:///tmp/document.html"), "session=local; Path=/");

    assert_eq!(
        jar.header_for_context(
            &url("https://example.test/"),
            Some(&url("https://example.test/")),
            true,
            HttpMethod::Get,
        ),
        None
    );
}
