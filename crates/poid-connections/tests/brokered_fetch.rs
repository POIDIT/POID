//! End-to-end checks for the brokered network path (SPEC §7.2.5), against a
//! real HTTP server on a real socket.
//!
//! The server runs on loopback, which the broker refuses by default — so these
//! tests turn on the enterprise-policy escape hatch
//! (`with_private_addresses_allowed`) deliberately. That is the honest way to
//! test this: the alternative is a mock that proves the mock works, and the
//! whole point of the exercise is what happens on a socket.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use poid_broker::{NetworkPolicy, Origin};
use poid_connections::{
    brokered_fetch, FetchLimits, FetchRequest, FetchResponse, OriginCredentials,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What one request looked like on the wire.
#[derive(Debug, Clone, Default)]
struct Seen {
    requests: Vec<String>,
}

impl Seen {
    fn last(&self) -> &str {
        self.requests.last().map(String::as_str).unwrap_or("")
    }
}

/// A one-shot HTTP server that replies with scripted responses in order.
async fn server(responses: Vec<String>) -> (u16, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let sink = Arc::clone(&seen);

    tokio::spawn(async move {
        for response in responses {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut buffer = vec![0_u8; 8192];
            let read = socket.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            if let Ok(mut guard) = sink.lock() {
                guard.requests.push(request);
            }
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });

    (port, seen)
}

fn ok_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nSet-Cookie: sid=abc\r\nX-Fine: yes\r\n\r\n{}",
        body.len(),
        body
    )
}

fn policy_for(port: u16) -> NetworkPolicy {
    let origin = Origin::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let allowed = [origin];
    // Loopback is refused by default; an intranet destination is exactly the
    // case SPEC §13 lets a managed policy permit, and it is what makes a real
    // socket testable here.
    NetworkPolicy::new(&allowed, &allowed).with_private_addresses_allowed(true)
}

fn request(port: u16, path: &str) -> FetchRequest {
    FetchRequest {
        url: format!("http://127.0.0.1:{port}{path}"),
        method: "GET".to_owned(),
        headers: Vec::new(),
        body: None,
    }
}

fn limits() -> FetchLimits {
    FetchLimits {
        timeout: Duration::from_secs(5),
        ..FetchLimits::default()
    }
}

async fn fetch(
    port: u16,
    path: &str,
    headers: Vec<(String, String)>,
    credentials: &OriginCredentials,
) -> poid_connections::Result<FetchResponse> {
    let mut req = request(port, path);
    req.headers = headers;
    brokered_fetch(req, &policy_for(port), credentials, limits()).await
}

#[tokio::test]
async fn an_allowed_request_goes_out_and_the_body_comes_back() {
    let (port, seen) = server(vec![ok_response("hello")]).await;
    let response = fetch(port, "/x", Vec::new(), &OriginCredentials::new())
        .await
        .expect("the request is allowed");

    assert_eq!(response.status, 200);
    assert_eq!(response.body, "hello");
    assert!(seen.lock().unwrap().last().starts_with("GET /x"));
}

#[tokio::test]
async fn the_applications_authorization_header_never_reaches_the_wire() {
    let (port, seen) = server(vec![ok_response("ok")]).await;
    fetch(
        port,
        "/x",
        vec![
            ("Authorization".to_owned(), "Bearer app-forged".to_owned()),
            ("Cookie".to_owned(), "sid=stolen".to_owned()),
            ("X-Allowed".to_owned(), "yes".to_owned()),
        ],
        &OriginCredentials::new(),
    )
    .await
    .expect("allowed");

    let wire = seen.lock().unwrap().last().to_owned();
    // The app cannot authenticate as anyone, nor replay a cookie.
    assert!(!wire.to_lowercase().contains("app-forged"));
    assert!(!wire.to_lowercase().contains("sid=stolen"));
    // Ordinary headers still work — this is a filter, not a straitjacket.
    assert!(wire.contains("x-allowed: yes") || wire.contains("X-Allowed: yes"));
}

#[tokio::test]
async fn the_real_credential_is_attached_by_the_broker_for_a_mapped_origin() {
    let (port, seen) = server(vec![ok_response("ok")]).await;
    let mut credentials = OriginCredentials::new();
    credentials.insert(
        Origin::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
        "sk-live-real-key",
    );

    fetch(
        port,
        "/x",
        vec![("Authorization".to_owned(), "Bearer app-forged".to_owned())],
        &credentials,
    )
    .await
    .expect("allowed");

    let wire = seen.lock().unwrap().last().to_owned();
    // The broker's credential went out...
    assert!(wire.contains("sk-live-real-key"));
    // ...and the application's attempt did not, even though it set the same
    // header. The order is: strip, then attach.
    assert!(!wire.contains("app-forged"));
}

#[tokio::test]
async fn no_credential_is_attached_to_an_origin_that_is_not_mapped() {
    let (port, seen) = server(vec![ok_response("ok")]).await;
    let mut credentials = OriginCredentials::new();
    // A credential for a *different* origin than the one being requested.
    credentials.insert(
        Origin::parse("https://api.example.com").unwrap(),
        "sk-live-other-key",
    );

    fetch(port, "/x", Vec::new(), &credentials)
        .await
        .expect("allowed");

    let wire = seen.lock().unwrap().last().to_owned();
    assert!(
        !wire.contains("sk-live-other-key"),
        "a credential leaked to an origin it was not configured for"
    );
}

#[tokio::test]
async fn set_cookie_does_not_come_back_to_the_application() {
    let (port, _seen) = server(vec![ok_response("ok")]).await;
    let response = fetch(port, "/x", Vec::new(), &OriginCredentials::new())
        .await
        .expect("allowed");

    let names: Vec<String> = response
        .headers
        .iter()
        .map(|(n, _)| n.to_ascii_lowercase())
        .collect();
    assert!(!names.contains(&"set-cookie".to_owned()));
    // Ordinary response headers still cross.
    assert!(names.contains(&"x-fine".to_owned()));
}

#[tokio::test]
async fn a_redirect_to_a_disallowed_origin_is_refused_at_the_hop() {
    // The hole this closes: a client that follows redirects internally takes
    // the second hop with none of the allowlist or address checks applied.
    let (port, _seen) = server(vec![
        "HTTP/1.1 302 Found\r\nLocation: https://evil.example.com/steal\r\nContent-Length: 0\r\n\r\n"
            .to_owned(),
    ])
    .await;

    let error = fetch(port, "/x", Vec::new(), &OriginCredentials::new())
        .await
        .expect_err("the second hop is not allowlisted");
    assert!(
        error.to_string().contains("approved allowlist"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn a_redirect_within_the_allowlist_is_followed() {
    let (port, seen) = server(vec![
        "HTTP/1.1 302 Found\r\nLocation: /second\r\nContent-Length: 0\r\n\r\n".to_owned(),
        ok_response("arrived"),
    ])
    .await;

    let response = fetch(port, "/first", Vec::new(), &OriginCredentials::new())
        .await
        .expect("both hops are allowed");
    assert_eq!(response.body, "arrived");

    let requests = seen.lock().unwrap().requests.clone();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /first"));
    assert!(requests[1].starts_with("GET /second"));
}

#[tokio::test]
async fn a_redirect_loop_gives_up_rather_than_spinning() {
    let hop = "HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n".to_owned();
    let (port, _seen) = server(vec![
        hop.clone(),
        hop.clone(),
        hop.clone(),
        hop.clone(),
        hop,
    ])
    .await;

    let error = brokered_fetch(
        request(port, "/again"),
        &policy_for(port),
        &OriginCredentials::new(),
        FetchLimits {
            max_redirects: 2,
            timeout: Duration::from_secs(5),
            ..FetchLimits::default()
        },
    )
    .await
    .expect_err("the loop is cut");
    assert!(
        error.to_string().contains("redirects"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn an_oversized_response_is_refused_rather_than_buffered() {
    let big = "x".repeat(4096);
    let (port, _seen) = server(vec![ok_response(&big)]).await;

    let error = brokered_fetch(
        request(port, "/big"),
        &policy_for(port),
        &OriginCredentials::new(),
        FetchLimits {
            max_body_bytes: 512,
            timeout: Duration::from_secs(5),
            ..FetchLimits::default()
        },
    )
    .await
    .expect_err("too large");
    assert!(
        error.to_string().contains("larger than"),
        "unexpected: {error}"
    );
}

#[tokio::test]
async fn a_loopback_destination_is_refused_without_the_managed_policy() {
    // The same server, the same allowlist — but the default posture. This is
    // the assertion that the escape hatch above is genuinely an escape hatch
    // and not the normal path.
    let (port, _seen) = server(vec![ok_response("ok")]).await;
    let origin = Origin::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let allowed = [origin];
    let policy = NetworkPolicy::new(&allowed, &allowed);

    let error = brokered_fetch(
        request(port, "/x"),
        &policy,
        &OriginCredentials::new(),
        limits(),
    )
    .await
    .expect_err("loopback is refused by default");
    let text = error.to_string();
    assert!(
        text.contains("loopback") || text.contains("did not resolve"),
        "unexpected: {text}"
    );
}
