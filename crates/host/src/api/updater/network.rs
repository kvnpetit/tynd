//! Host-trusted network settings of the updater: an optional proxy and fixed
//! request headers (private update servers).
//!
//! These come only from the backend-only `updater.configure` config (or the
//! CLI-provided `TYND_FRONTEND_RUNTIME_CONFIG`); a renderer-supplied `proxy` or
//! `headers` is never read. Headers are sent only to the origins of the
//! configured endpoints, so a redirect to another origin (a signed release CDN)
//! never receives them. With no configured proxy, ureq's environment lookup
//! (`HTTPS_PROXY` / `ALL_PROXY`) stays the fallback.

use std::collections::HashSet;

use serde_json::Value;
use ureq::http::{HeaderName, HeaderValue};

const MAX_HEADERS: usize = 32;
const MAX_HEADER_VALUE_BYTES: usize = 4096;

/// Names the transport owns; letting config set them would break framing or
/// route the request somewhere else.
const FORBIDDEN_HEADERS: [&str; 12] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "proxy-authorization",
    "proxy-connection",
    "proxy-authenticate",
];

#[derive(Clone, Default)]
pub(super) struct TrustedNetwork {
    proxy: Option<String>,
    headers: Vec<(HeaderName, HeaderValue)>,
    origins: Vec<url::Origin>,
}

impl TrustedNetwork {
    /// Read and validate the `proxy` and `headers` keys of a trusted updater
    /// config. `endpoints` are the configured manifest endpoints: their origins
    /// are the only ones that receive the headers.
    pub(super) fn from_config(config: &Value, endpoints: &[String]) -> Result<Self, String> {
        let proxy = match config.get("proxy") {
            None | Some(Value::Null) => None,
            Some(Value::String(raw)) => Some(validate_proxy(raw)?),
            Some(_) => return Err("updater proxy must be a string".into()),
        };
        let headers = match config.get("headers") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Object(map)) => parse_headers(map)?,
            Some(_) => return Err("updater headers must be an object of strings".into()),
        };
        let origins: Vec<url::Origin> = endpoints
            .iter()
            .filter_map(|endpoint| url::Url::parse(endpoint).ok())
            .map(|parsed| parsed.origin())
            // A placeholder in the scheme, host or port leaves no fixed origin.
            .filter(|origin| origin.is_tuple() && !origin.ascii_serialization().contains('{'))
            .collect();
        if !headers.is_empty() && origins.is_empty() {
            return Err(
                "updater headers need an endpoint with a fixed origin (no placeholder in the \
                 scheme, host or port)"
                    .into(),
            );
        }
        Ok(Self {
            proxy,
            headers,
            origins,
        })
    }

    pub(super) fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    /// Headers to send to `url`: the configured ones when `url` is on an
    /// endpoint origin, none otherwise.
    pub(super) fn headers_for(&self, url: &url::Url) -> &[(HeaderName, HeaderValue)] {
        if self.origins.contains(&url.origin()) {
            &self.headers
        } else {
            &[]
        }
    }
}

fn validate_proxy(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let parsed =
        url::Url::parse(raw).map_err(|_| "updater proxy is not a valid URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("updater proxy must use the http or https scheme".into());
    }
    if !parsed.has_host() || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("updater proxy must be a plain scheme://host[:port] URL".into());
    }
    ureq::Proxy::new(raw).map_err(|_| "updater proxy is not accepted by the HTTP client")?;
    Ok(raw.to_string())
}

fn parse_headers(
    map: &serde_json::Map<String, Value>,
) -> Result<Vec<(HeaderName, HeaderValue)>, String> {
    if map.len() > MAX_HEADERS {
        return Err(format!("updater headers: at most {MAX_HEADERS} allowed"));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(map.len());
    for (name, value) in map {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("updater headers: invalid header name '{name}'"))?;
        if FORBIDDEN_HEADERS.contains(&header_name.as_str()) {
            return Err(format!(
                "updater headers: '{name}' is set by the HTTP client and cannot be configured"
            ));
        }
        if !seen.insert(header_name.clone()) {
            return Err(format!("updater headers: '{name}' is listed twice"));
        }
        let text = value
            .as_str()
            .ok_or_else(|| format!("updater headers: the value of '{name}' must be a string"))?;
        if text.len() > MAX_HEADER_VALUE_BYTES {
            return Err(format!(
                "updater headers: the value of '{name}' is too long"
            ));
        }
        // The value is never echoed: it is usually a credential.
        let header_value = HeaderValue::from_str(text)
            .map_err(|_| format!("updater headers: the value of '{name}' is not valid"))?;
        out.push((header_name, header_value));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn endpoints() -> Vec<String> {
        vec!["https://updates.example.com/{{channel}}/latest.json".to_string()]
    }

    fn url(raw: &str) -> url::Url {
        url::Url::parse(raw).unwrap()
    }

    #[test]
    fn absent_settings_mean_no_proxy_and_no_headers() {
        let net = TrustedNetwork::from_config(&json!({}), &endpoints()).unwrap();
        assert!(net.proxy().is_none());
        assert_eq!(net.headers_for(&url("https://updates.example.com/x")), []);
    }

    #[test]
    fn headers_go_only_to_endpoint_origins() {
        let cfg = json!({ "headers": { "Authorization": "Bearer t0ken", "X-Api-Key": "k" } });
        let net = TrustedNetwork::from_config(&cfg, &endpoints()).unwrap();
        let on_origin = net.headers_for(&url("https://updates.example.com/app.exe"));
        assert_eq!(on_origin.len(), 2);
        assert!(on_origin
            .iter()
            .any(|(n, v)| n == "authorization" && v == "Bearer t0ken"));
        // Cross-origin redirect target, other scheme, other port: no headers.
        for other in [
            "https://cdn.other.net/app.exe",
            "http://updates.example.com/app.exe",
            "https://updates.example.com:8443/app.exe",
            "https://updates.example.com.evil.net/app.exe",
        ] {
            assert_eq!(net.headers_for(&url(other)), [], "{other}");
        }
    }

    #[test]
    fn transport_owned_headers_are_rejected() {
        for name in [
            "Host",
            "content-length",
            "Transfer-Encoding",
            "Proxy-Authorization",
        ] {
            let cfg = json!({ "headers": { name: "x" } });
            assert!(
                TrustedNetwork::from_config(&cfg, &endpoints()).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn malformed_headers_are_rejected_without_echoing_the_value() {
        let bad_name = json!({ "headers": { "bad name": "x" } });
        assert!(TrustedNetwork::from_config(&bad_name, &endpoints()).is_err());
        let not_string = json!({ "headers": { "X-A": 1 } });
        assert!(TrustedNetwork::from_config(&not_string, &endpoints()).is_err());
        let newline = json!({ "headers": { "X-A": "secret\r\nInjected: 1" } });
        let err = TrustedNetwork::from_config(&newline, &endpoints())
            .err()
            .unwrap();
        assert!(!err.contains("secret"), "{err}");
        let dup = json!({ "headers": { "X-A": "1", "x-a": "2" } });
        assert!(TrustedNetwork::from_config(&dup, &endpoints()).is_err());
        assert!(TrustedNetwork::from_config(&json!({ "headers": ["x"] }), &endpoints()).is_err());
    }

    #[test]
    fn headers_need_a_fixed_endpoint_origin() {
        let cfg = json!({ "headers": { "Authorization": "t" } });
        let templated = vec!["https://{{channel}}.example.com/m.json".to_string()];
        assert!(TrustedNetwork::from_config(&cfg, &templated).is_err());
        // Without headers the same endpoints are fine.
        assert!(TrustedNetwork::from_config(&json!({}), &templated).is_ok());
    }

    #[test]
    fn proxy_is_validated() {
        let ok = TrustedNetwork::from_config(
            &json!({ "proxy": " http://proxy.corp:3128 " }),
            &endpoints(),
        )
        .unwrap();
        assert_eq!(ok.proxy(), Some("http://proxy.corp:3128"));
        for bad in [
            "not a url",
            "ftp://proxy.corp",
            "file:///etc/passwd",
            "http://proxy.corp:3128/?x=1",
            "",
        ] {
            let cfg = json!({ "proxy": bad });
            assert!(
                TrustedNetwork::from_config(&cfg, &endpoints()).is_err(),
                "{bad}"
            );
        }
        assert!(TrustedNetwork::from_config(&json!({ "proxy": 3 }), &endpoints()).is_err());
    }
}
