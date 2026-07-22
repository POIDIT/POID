//! The brokered network path (SPEC §7.2.5) — `poid.net.fetch` on the far side
//! of the boundary.
//!
//! This is the first outbound request POID makes on an application's behalf,
//! and the order of operations is the whole security argument:
//!
//! 1. Parse the URL into an origin, and check it against the allowlist. A
//!    disallowed origin is refused **before** any name is resolved, so it does
//!    not even produce a DNS query an observer could use.
//! 2. Resolve the name here, and validate **every** address it produced.
//! 3. Pin the connection to an address that passed. This is the step that
//!    matters: checking a hostname and then handing the name to an HTTP client
//!    is defeated by DNS rebinding — the name resolves public while it is
//!    being checked and to `169.254.169.254` when it is connected to.
//! 4. Strip whatever the application put in `Authorization`, then attach the
//!    real credential only if this origin maps to a Connection.
//! 5. Follow redirects **ourselves**, repeating 1–4 for every hop. A client
//!    that follows redirects internally would take the second hop with none of
//!    these checks applied, which is the same hole wearing a different hat.

use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use std::time::Duration;

use poid_broker::{NetworkPolicy, Origin, PolicyError};

use crate::error::{ConnectionError, Result};

/// Headers an application may never set.
///
/// `authorization` is the one SPEC §7.2.5 names. The rest are here because
/// they authenticate or redirect in the same way: a cookie is a credential, a
/// proxy header reroutes the request, and `host` decides which virtual host a
/// pinned address serves — an application that could set it would reach a
/// different site at an allowlisted address.
const FORBIDDEN_REQUEST_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "proxy-authorization",
    "host",
    "content-length",
];

/// Response headers that never cross back to the application.
///
/// `set-cookie` would let a backend plant state the reader would then replay
/// on the app's behalf without the user having agreed to anything.
const FORBIDDEN_RESPONSE_HEADERS: &[&str] = &["set-cookie", "set-cookie2"];

/// Bounds on one brokered request.
#[derive(Debug, Clone, Copy)]
pub struct FetchLimits {
    /// Largest response body accepted, in bytes.
    pub max_body_bytes: usize,
    /// Redirect hops followed before giving up.
    pub max_redirects: usize,
    /// Wall-clock budget for a single hop.
    pub timeout: Duration,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            // Generous for an API response and far below anything that would
            // let a POID exhaust memory by asking for a large file.
            max_body_bytes: 8 * 1024 * 1024,
            max_redirects: 5,
            timeout: Duration::from_secs(30),
        }
    }
}

/// What the application asked for, after the guard stripped it.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// Absolute URL.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Headers the application set. Forbidden ones are dropped here.
    pub headers: Vec<(String, String)>,
    /// Request body, if any.
    pub body: Option<String>,
}

/// What the application gets back. No request echo: an app must not be able to
/// read the headers the broker actually sent, because those carry the
/// credential.
#[derive(Debug, Clone)]
pub struct FetchResponse {
    /// HTTP status code.
    pub status: u16,
    /// HTTP reason phrase.
    pub status_text: String,
    /// Response headers, minus the ones in [`FORBIDDEN_RESPONSE_HEADERS`].
    pub headers: Vec<(String, String)>,
    /// Response body as text.
    pub body: String,
}

/// The credentials the broker may attach, by origin.
///
/// Holds real secrets, so it is built per request in the process that already
/// holds them and dropped when the request finishes. Nothing here is returned
/// to a caller that could send it onward.
#[derive(Debug, Default)]
pub struct OriginCredentials {
    entries: Vec<(Origin, String)>,
}

impl OriginCredentials {
    /// An empty set: every request goes out unauthenticated.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the credential to attach for one origin.
    pub fn insert(&mut self, origin: Origin, credential: impl Into<String>) {
        self.entries.push((origin, credential.into()));
    }

    /// The credential for exactly this origin, if any.
    ///
    /// Exact match, never a suffix or prefix rule: a looser comparison would
    /// send the user's key to `api.example.com.attacker.test`.
    #[must_use]
    fn for_origin(&self, origin: &Origin) -> Option<&str> {
        self.entries
            .iter()
            .find(|(o, _)| o == origin)
            .map(|(_, c)| c.as_str())
    }
}

/// Installs the TLS provider once per process.
///
/// `reqwest` is built with `rustls-no-provider`, so the choice is explicit
/// here rather than implied by a feature flag somebody could change without
/// noticing which cryptography they switched to.
fn ensure_tls_provider() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // An error means a provider is already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Resolves a host to the addresses the broker is willing to connect to.
///
/// A literal address needs no resolution — and gets no chance to rebind.
async fn usable_addresses(
    policy: &NetworkPolicy,
    origin: &Origin,
) -> std::result::Result<Vec<IpAddr>, PolicyError> {
    if let Some(literal) = origin.literal_address() {
        policy.check_address(origin.host(), literal)?;
        return Ok(vec![literal]);
    }

    let host = origin.host().to_owned();
    let port = origin.port();
    let resolved: Vec<IpAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| PolicyError::NoUsableAddress { host: host.clone() })?
        .map(|addr| addr.ip())
        .collect();

    policy.usable_addresses(&host, &resolved)
}

/// Performs one request on an application's behalf.
///
/// # Errors
///
/// [`ConnectionError::Network`] for a refusal or a transport failure. The
/// message is for the **user's** log; the caller maps it to a
/// `RUNTIME-API.md` §9 code before anything crosses to the application.
pub async fn brokered_fetch(
    request: FetchRequest,
    policy: &NetworkPolicy,
    credentials: &OriginCredentials,
    limits: FetchLimits,
) -> Result<FetchResponse> {
    ensure_tls_provider();

    let mut url = request.url.clone();
    let mut method = request.method.clone();
    let mut body = request.body.clone();

    for _hop in 0..=limits.max_redirects {
        let origin = Origin::of_url(&url).map_err(|e| ConnectionError::Network {
            reason: e.to_string(),
        })?;

        // (1) Allowlist first — before any name is resolved.
        policy
            .check_origin(&origin)
            .map_err(|e| ConnectionError::Network {
                reason: e.to_string(),
            })?;

        // (2) + (3) Resolve here, validate every answer, pin to one that passed.
        let addresses =
            usable_addresses(policy, &origin)
                .await
                .map_err(|e| ConnectionError::Network {
                    reason: e.to_string(),
                })?;
        let pinned = addresses.first().copied().ok_or(ConnectionError::Network {
            reason: "no usable address".to_owned(),
        })?;

        let bare_host = origin
            .host()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let client = reqwest::Client::builder()
            // Redirects are ours to follow, one checked hop at a time.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(limits.timeout)
            .resolve(&bare_host, SocketAddr::new(pinned, origin.port()))
            .build()
            .map_err(|_| ConnectionError::Network {
                reason: "the network stack could not be started".to_owned(),
            })?;

        let verb = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| {
            ConnectionError::Network {
                reason: format!("`{method}` is not an HTTP method"),
            }
        })?;
        let mut builder = client.request(verb, &url);

        // (4) The application's headers, minus the ones it may not set.
        for (name, value) in &request.headers {
            if FORBIDDEN_REQUEST_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                continue;
            }
            builder = builder.header(name, value);
        }
        // ...then the real credential, attached here and nowhere earlier, and
        // only for an origin the user mapped to a Connection.
        if let Some(credential) = credentials.for_origin(&origin) {
            builder = builder.bearer_auth(credential);
        }
        if let Some(payload) = body.clone() {
            builder = builder.body(payload);
        }

        let response = builder.send().await.map_err(|e| ConnectionError::Network {
            // reqwest's message names the host and the transport error, which
            // the user's log should have and the application must not.
            reason: format!("request to {origin} failed: {e}"),
        })?;

        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(ConnectionError::Network {
                    reason: "a redirect without a destination".to_owned(),
                })?;
            // Resolve relative destinations against the hop we are on, then
            // loop — which re-runs (1) to (4) for the new origin.
            url = resolve_redirect(&url, location)?;
            // 303, and 301/302 on POST, become GET without a body, as every
            // HTTP client does.
            if status.as_u16() == 303 || (matches!(status.as_u16(), 301 | 302) && method != "GET") {
                method = "GET".to_owned();
                body = None;
            }
            continue;
        }

        return read_response(response, limits.max_body_bytes).await;
    }

    Err(ConnectionError::Network {
        reason: format!("more than {} redirects", limits.max_redirects),
    })
}

/// Joins a redirect destination against the URL it came from.
fn resolve_redirect(current: &str, location: &str) -> Result<String> {
    if location.contains("://") {
        return Ok(location.to_owned());
    }
    let origin = Origin::of_url(current).map_err(|e| ConnectionError::Network {
        reason: e.to_string(),
    })?;
    if let Some(path) = location.strip_prefix('/') {
        return Ok(format!("{origin}/{path}"));
    }
    Ok(format!("{origin}/{location}"))
}

/// Reads a body, refusing to buffer more than the cap.
///
/// Streamed rather than `bytes()`-ed so an oversized response is abandoned
/// while it arrives, instead of being fully buffered and then rejected.
async fn read_response(
    mut response: reqwest::Response,
    max_body_bytes: usize,
) -> Result<FetchResponse> {
    let status = response.status();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter(|(name, _)| {
            !FORBIDDEN_RESPONSE_HEADERS.contains(&name.as_str().to_ascii_lowercase().as_str())
        })
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_owned(), v.to_owned()))
        })
        .collect();

    let mut buffer: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ConnectionError::Network {
            reason: "the response ended unexpectedly".to_owned(),
        })?
    {
        if buffer.len() + chunk.len() > max_body_bytes {
            return Err(ConnectionError::Network {
                reason: format!("the response is larger than {max_body_bytes} bytes"),
            });
        }
        buffer.extend_from_slice(&chunk);
    }

    Ok(FetchResponse {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_owned(),
        headers,
        body: String::from_utf8_lossy(&buffer).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(value: &str) -> Origin {
        Origin::parse(value).expect("a valid origin")
    }

    #[test]
    fn a_credential_is_attached_only_to_its_exact_origin() {
        let mut credentials = OriginCredentials::new();
        credentials.insert(origin("https://api.example.com"), "sk-live-secret");

        assert_eq!(
            credentials.for_origin(&origin("https://api.example.com")),
            Some("sk-live-secret")
        );
        // The default port is the same origin.
        assert_eq!(
            credentials.for_origin(&origin("https://api.example.com:443")),
            Some("sk-live-secret")
        );
        // The near-misses an attacker registers precisely because they look
        // right to a prefix or suffix comparison.
        for other in [
            "https://api.example.com.attacker.test",
            "https://evil.api.example.com",
            "http://api.example.com",
            "https://api.example.com:8443",
        ] {
            assert_eq!(credentials.for_origin(&origin(other)), None, "{other}");
        }
    }

    #[test]
    fn the_forbidden_header_lists_cover_the_ways_a_request_authenticates() {
        for header in ["authorization", "cookie", "proxy-authorization", "host"] {
            assert!(FORBIDDEN_REQUEST_HEADERS.contains(&header), "{header}");
        }
        assert!(FORBIDDEN_RESPONSE_HEADERS.contains(&"set-cookie"));
    }

    #[test]
    fn a_relative_redirect_resolves_against_the_hop_it_came_from() {
        assert_eq!(
            resolve_redirect("https://a.test/one", "/two").expect("resolves"),
            "https://a.test/two"
        );
        assert_eq!(
            resolve_redirect("https://a.test/one", "two").expect("resolves"),
            "https://a.test/two"
        );
        // An absolute destination is taken as-is — and then re-checked against
        // the allowlist by the caller's next loop iteration, which is the
        // point of following redirects ourselves.
        assert_eq!(
            resolve_redirect("https://a.test/one", "https://b.test/x").expect("resolves"),
            "https://b.test/x"
        );
    }

    #[tokio::test]
    async fn a_disallowed_origin_is_refused_before_dns() {
        // `.invalid` never resolves (RFC 6761), so if this returned a DNS
        // failure rather than a policy refusal we would know the check ran in
        // the wrong order.
        let policy = NetworkPolicy::new(&[], &[]);
        let error = brokered_fetch(
            FetchRequest {
                url: "https://nothing.invalid/x".to_owned(),
                method: "GET".to_owned(),
                headers: Vec::new(),
                body: None,
            },
            &policy,
            &OriginCredentials::new(),
            FetchLimits::default(),
        )
        .await
        .expect_err("must be refused");
        assert!(error.to_string().contains("approved allowlist"));
    }

    #[tokio::test]
    async fn an_allowlisted_name_that_resolves_into_private_space_is_still_refused() {
        // The DNS-rebinding shape: the origin is approved, and the address is
        // not. localhost resolves to loopback on every platform.
        let allowed = [origin("http://localhost:9")];
        let policy = NetworkPolicy::new(&allowed, &allowed);
        let error = brokered_fetch(
            FetchRequest {
                url: "http://localhost:9/x".to_owned(),
                method: "GET".to_owned(),
                headers: Vec::new(),
                body: None,
            },
            &policy,
            &OriginCredentials::new(),
            FetchLimits::default(),
        )
        .await
        .expect_err("must be refused");
        let text = error.to_string();
        assert!(
            text.contains("loopback") || text.contains("did not resolve"),
            "unexpected refusal: {text}"
        );
    }
}
