//! Lightning Swap Connect URI parsing (`lightning+swapconnect://host[/path]?key=..&secret=..`),
//! ported rule-for-rule from OpenReceive's `lsc-uri.ts`. An LSC URI is a bearer credential, so
//! no error message ever includes the key or secret.

use reqwest::Url;

const LSC_SCHEME: &str = "lightning+swapconnect";
const MAX_LSC_URI_LENGTH: usize = 8_192;
const MAX_LSC_CREDENTIAL_LENGTH: usize = 2_048;

/// A parsed LSC URI. `base_url` is always HTTPS and ends with `/`.
#[derive(Clone, PartialEq, Eq)]
pub struct LscConnection {
    pub base_url: String,
    pub provider_id: String,
    pub key: String,
    pub secret: String,
}

/// Debug without the credentials, so a stray `{:?}` cannot leak them into logs.
impl std::fmt::Debug for LscConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LscConnection")
            .field("base_url", &self.base_url)
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

pub fn parse_lsc_uri(value: &str) -> Result<LscConnection, String> {
    let input = required_credential(value, "LSC URI", MAX_LSC_URI_LENGTH)?;
    let uri = Url::parse(input).map_err(|_| "LSC URI is not a valid absolute URI.".to_string())?;

    if uri.scheme() != LSC_SCHEME {
        return Err(format!("LSC URI must use {LSC_SCHEME}://."));
    }
    if !uri.username().is_empty() || uri.password().is_some() {
        return Err("LSC URI must not use URI userinfo.".into());
    }
    let host = uri.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err("LSC URI requires a provider hostname.".into());
    }
    if uri.fragment().is_some_and(|f| !f.is_empty()) {
        return Err("LSC URI must not contain a fragment.".into());
    }
    if !valid_percent_encoding(uri.query().unwrap_or_default()) {
        return Err("LSC URI query encoding is invalid.".into());
    }
    if uri
        .query_pairs()
        .any(|(name, _)| name != "key" && name != "secret")
    {
        return Err("LSC URI contains an unsupported query parameter.".into());
    }

    let key = single_parameter(&uri, "key")?;
    let key = required_credential(&key, "LSC URI key", MAX_LSC_CREDENTIAL_LENGTH)?.to_string();
    let secret = single_parameter(&uri, "secret")?;
    let secret =
        required_credential(&secret, "LSC URI secret", MAX_LSC_CREDENTIAL_LENGTH)?.to_string();

    let host_port = match uri.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    Ok(LscConnection {
        base_url: format!("https://{host_port}{}", normalize_endpoint_path(uri.path())),
        provider_id: provider_id(host, uri.port(), uri.path())?,
        key,
        secret,
    })
}

fn single_parameter(uri: &Url, name: &str) -> Result<String, String> {
    let values: Vec<String> = uri
        .query_pairs()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .collect();
    match values.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err(format!("LSC URI requires exactly one {name} parameter.")),
    }
}

fn required_credential<'a>(value: &'a str, label: &str, max: usize) -> Result<&'a str, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!("{label} must not be empty."));
    }
    if normalized.len() > max {
        return Err(format!("{label} is too long."));
    }
    Ok(normalized)
}

/// Rejects a `%` not followed by two hex digits (`/%(?![0-9a-f]{2})/i`).
fn valid_percent_encoding(query: &str) -> bool {
    let bytes = query.as_bytes();
    bytes.iter().enumerate().all(|(i, &b)| {
        b != b'%'
            || (bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    })
}

fn normalize_endpoint_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        "/".to_string()
    } else if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    }
}

/// Lower-case `host[-port][-path-segments]`, unsupported runs replaced by `-`, max 64 chars.
fn provider_id(host: &str, port: Option<u16>, path: &str) -> Result<String, String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut raw = host.to_string();
    if let Some(port) = port {
        raw.push_str(&format!("-{port}"));
    }
    if !segments.is_empty() {
        raw.push('-');
        raw.push_str(&segments.join("-"));
    }
    let mut id = String::new();
    for c in raw.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            id.push(c);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id: String = id.trim_matches('-').chars().take(64).collect();
    if id.is_empty() {
        return Err("LSC URI could not derive a provider id.".into());
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_example() {
        let c = parse_lsc_uri(
            "lightning+swapconnect://swap.example/v1?key=example-key&secret=example-secret",
        )
        .unwrap();
        assert_eq!(c.base_url, "https://swap.example/v1/");
        assert_eq!(c.provider_id, "swap-example-v1");
        assert_eq!(c.key, "example-key");
        assert_eq!(c.secret, "example-secret");
    }

    #[test]
    fn default_path_port_and_percent_decoding() {
        let c = parse_lsc_uri("lightning+swapconnect://ff.io?key=k%2B1&secret=s").unwrap();
        assert_eq!(c.base_url, "https://ff.io/");
        assert_eq!(c.provider_id, "ff-io");
        assert_eq!(c.key, "k+1");
        let c =
            parse_lsc_uri("lightning+swapconnect://Swap.Example:8443/a/b/?key=k&secret=s").unwrap();
        assert!(c.base_url.ends_with(":8443/a/b/"));
        assert_eq!(c.provider_id, "swap-example-8443-a-b");
    }

    #[test]
    fn rejects_malformed_uris() {
        for bad in [
            "",
            "   ",
            "https://swap.example?key=k&secret=s",
            "lightning+swapconnect://user:pw@swap.example?key=k&secret=s",
            "lightning+swapconnect://swap.example?key=k&secret=s#frag",
            "lightning+swapconnect://swap.example?key=k&secret=s&extra=1",
            "lightning+swapconnect://swap.example?key=k&key=k2&secret=s",
            "lightning+swapconnect://swap.example?secret=s",
            "lightning+swapconnect://swap.example?key=&secret=s",
            "lightning+swapconnect://swap.example?key=k%zz&secret=s",
            "lightning+swapconnect:swap.example?key=k&secret=s",
        ] {
            assert!(parse_lsc_uri(bad).is_err(), "should reject {bad:?}");
        }
        let long = format!(
            "lightning+swapconnect://x?key={}&secret=s",
            "k".repeat(2049)
        );
        assert!(parse_lsc_uri(&long).is_err());
    }

    #[test]
    fn errors_and_debug_never_contain_credentials() {
        let secret = "topsecretvalue";
        for uri in [
            format!("lightning+swapconnect://swap.example?key=apikeyvalue&secret={secret}&x=1"),
            format!(
                "lightning+swapconnect://u:{secret}@swap.example?key=apikeyvalue&secret={secret}"
            ),
            format!("https://swap.example?key=apikeyvalue&secret={secret}"),
        ] {
            let err = parse_lsc_uri(&uri).unwrap_err();
            assert!(
                !err.contains(secret) && !err.contains("apikeyvalue"),
                "{err}"
            );
        }
        let ok = parse_lsc_uri(&format!(
            "lightning+swapconnect://swap.example?key=apikeyvalue&secret={secret}"
        ))
        .unwrap();
        let debug = format!("{ok:?}");
        assert!(!debug.contains(secret) && !debug.contains("apikeyvalue"));
    }
}
