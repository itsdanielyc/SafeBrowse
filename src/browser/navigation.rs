//! Shared address-bar parsing and validation for external web navigation.

use url::Url;

const SEARCH_URL: &str = "https://duckduckgo.com/";
const SUPPORTED_SCHEMES: &[&str] = &["http", "https"];

/// Validates an explicit HTTP(S) URL and returns its canonical representation.
///
/// Embedded credentials and backslashes are rejected because they can obscure the
/// destination shown to someone checking a payment or banking address.
pub fn validate_web_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    reject_control_characters(input)?;
    if input.contains('\\') {
        return Err("Web addresses must not contain backslashes".to_string());
    }
    let parsed = Url::parse(input).map_err(|error| format!("Invalid web address: {error}"))?;
    if !SUPPORTED_SCHEMES.contains(&parsed.scheme()) || parsed.host_str().is_none() {
        return Err("Only HTTP and HTTPS web addresses are supported".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(
            "Web addresses containing a username or password are not supported".to_string(),
        );
    }
    Ok(parsed.into())
}

/// Resolves an address or search phrase, preferring HTTPS for an omitted scheme.
///
/// Explicit unsupported schemes fail instead of being loaded or searched. Search
/// query encoding is delegated to the URL library so punctuation cannot add query
/// parameters or redirect the destination.
///
/// Time: O(N). Space: O(N), where N is the input length.
pub fn normalize_navigation_input(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a web address or search term".to_string());
    }
    reject_control_characters(input)?;

    if input.starts_with("//") {
        return validate_web_url(&format!("https:{input}"));
    }

    if has_explicit_scheme(input) && !looks_like_host_with_port(input) {
        return validate_web_url(input);
    }

    if !input.chars().any(char::is_whitespace) && looks_like_address(input) {
        return validate_web_url(&format!("https://{input}"));
    }

    let mut search = Url::parse(SEARCH_URL).expect("The search URL is a valid constant");
    search.query_pairs_mut().append_pair("q", input);
    Ok(search.into())
}

/// Reports HTTPS transport without treating an arbitrary string prefix as a URL.
pub fn uses_https(input: &str) -> bool {
    Url::parse(input)
        .map(|url| url.scheme() == "https" && url.host_str().is_some())
        .unwrap_or(false)
}

fn reject_control_characters(input: &str) -> Result<(), String> {
    if input.chars().any(char::is_control) {
        return Err("Web addresses and searches must not contain control characters".to_string());
    }
    Ok(())
}

fn has_explicit_scheme(input: &str) -> bool {
    let Some((scheme, _)) = input.split_once(':') else {
        return false;
    };
    let mut characters = scheme.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn looks_like_host_with_port(input: &str) -> bool {
    let authority = input.split(['/', '?', '#']).next().unwrap_or(input);
    let Some((host, port)) = authority.rsplit_once(':') else {
        return false;
    };
    !port.is_empty()
        && port.chars().all(|character| character.is_ascii_digit())
        && (host.contains('.') || host.eq_ignore_ascii_case("localhost") || host.starts_with('['))
}

fn looks_like_address(input: &str) -> bool {
    let authority = input.split(['/', '?', '#']).next().unwrap_or(input);
    authority.contains('.')
        || authority.eq_ignore_ascii_case("localhost")
        || authority.starts_with('[')
        || looks_like_host_with_port(input)
}
