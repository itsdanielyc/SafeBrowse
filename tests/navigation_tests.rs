use safebrowse::browser::navigation::{normalize_navigation_input, validate_web_url};
use url::Url;

#[test]
fn navigation_defaults_addresses_to_https_and_encodes_searches() {
    assert_eq!(
        normalize_navigation_input("example.com/login").unwrap(),
        "https://example.com/login"
    );
    assert_eq!(
        normalize_navigation_input("localhost:8443").unwrap(),
        "https://localhost:8443/"
    );
    assert_eq!(
        normalize_navigation_input("[::1]:8080").unwrap(),
        "https://[::1]:8080/"
    );
    assert_eq!(
        normalize_navigation_input("HTTP://example.com").unwrap(),
        "http://example.com/"
    );

    let search = normalize_navigation_input("bank accounts & safety #guide").unwrap();
    let parsed = Url::parse(&search).unwrap();
    assert_eq!(parsed.host_str(), Some("duckduckgo.com"));
    assert_eq!(
        parsed.query_pairs().collect::<Vec<_>>(),
        vec![("q".into(), "bank accounts & safety #guide".into())]
    );
    assert!(parsed.fragment().is_none());
}

#[test]
fn unsupported_schemes_and_deceptive_credentials_are_rejected() {
    for address in [
        "javascript:alert(1)",
        "data:text/html,test",
        "file:///C:/Windows/win.ini",
        "safebrowse://settings",
        "mailto:person@example.com",
        "https://bank.example@evil.example/",
        "https://user:password@example.com",
        "https://example.com\\@evil.example",
        "https://exam\nple.com",
        "",
        "   ",
    ] {
        assert!(
            normalize_navigation_input(address).is_err(),
            "Unexpectedly allowed: {address}"
        );
    }
}

#[test]
fn bookmark_validation_requires_an_explicit_web_address() {
    assert!(validate_web_url("bank.example").is_err());
    assert!(validate_web_url("about:blank").is_err());
    assert_eq!(
        validate_web_url(" HTTPS://EXAMPLE.COM ").unwrap(),
        "https://example.com/"
    );
}
