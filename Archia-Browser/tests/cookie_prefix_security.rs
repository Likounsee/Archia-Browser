use archia_browser::net::{CookieJar, Url};

#[test]
fn host_prefix_requires_secure_host_only_and_explicit_root_path() {
    let https = Url::parse("https://example.org/account").unwrap();
    let mut jar = CookieJar::new();

    jar.store(&https, "__Host-valid=1; Secure; Path=/");
    jar.store(&https, "__Host-no-path=1; Secure");
    jar.store(&https, "__Host-subpath=1; Secure; Path=/account");
    jar.store(&https, "__Host-domain=1; Secure; Path=/; Domain=example.org");

    assert_eq!(
        jar.header_for(&Url::parse("https://example.org/").unwrap())
            .as_deref(),
        Some("__Host-valid=1")
    );
    assert_eq!(
        jar.header_for(&Url::parse("https://sub.example.org/").unwrap()),
        None,
        "a __Host- cookie must not be shared with subdomains"
    );
}

#[test]
fn secure_and_http_prefixes_require_their_security_attributes() {
    let https = Url::parse("https://example.org/").unwrap();
    let http = Url::parse("http://example.org/").unwrap();
    let mut jar = CookieJar::new();

    jar.store(&http, "__Secure-insecure=1; Secure");
    jar.store(&https, "__Secure-missing=1");
    jar.store(&https, "__Http-not-http-only=1; Secure");
    jar.store(
        &https,
        "__Host-Http-missing-path=1; Secure; HttpOnly; Path=/account",
    );
    jar.store(&https, "__Secure-valid=1; Secure");
    jar.store(&https, "__Http-valid=1; Secure; HttpOnly");
    jar.store(&https, "__Host-Http-valid=1; Secure; HttpOnly; Path=/");

    assert_eq!(
        jar.header_for(&https).as_deref(),
        Some("__Secure-valid=1; __Http-valid=1; __Host-Http-valid=1")
    );
}
