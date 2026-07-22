//! Origin parsing and address classification — the checks that stand between
//! a POID and the user's LAN (SPEC §7.2.5).
//!
//! These are table-driven on purpose. Each row is a destination somebody has
//! actually used to reach a metadata service or an intranet host, and the
//! table is the record of which ones we know about.

use std::net::IpAddr;

use poid_broker::network::classify;
use poid_broker::{NetworkPolicy, Origin};

fn origin(value: &str) -> Origin {
    match Origin::parse(value) {
        Ok(o) => o,
        Err(e) => panic!("`{value}` should parse: {e}"),
    }
}

fn ip(value: &str) -> IpAddr {
    match value.parse() {
        Ok(a) => a,
        Err(_) => panic!("`{value}` is not an address"),
    }
}

// ------------------------------------------------------------------ origins

#[test]
fn canonicalises_case_and_default_ports() {
    assert_eq!(
        origin("https://API.Example.COM").to_string(),
        "https://api.example.com"
    );
    assert_eq!(
        origin("https://api.example.com:443").to_string(),
        "https://api.example.com"
    );
    assert_eq!(
        origin("http://api.example.com:80").to_string(),
        "http://api.example.com"
    );
    // A non-default port survives, because it is part of the origin.
    assert_eq!(
        origin("https://api.example.com:8443").to_string(),
        "https://api.example.com:8443"
    );
    // A trailing slash is tolerated; people write it.
    assert_eq!(
        origin("https://api.example.com/").to_string(),
        "https://api.example.com"
    );
}

#[test]
fn equality_is_scheme_host_and_port() {
    assert_eq!(origin("https://a.test"), origin("https://A.test:443"));
    assert_ne!(origin("https://a.test"), origin("http://a.test"));
    assert_ne!(origin("https://a.test"), origin("https://a.test:8443"));
    assert_ne!(origin("https://a.test"), origin("https://b.test"));
}

#[test]
fn refuses_anything_that_is_not_exactly_an_origin() {
    for value in [
        "api.example.com",                   // no scheme
        "ftp://example.com",                 // not http(s)
        "file:///etc/passwd",                // not http(s)
        "https://user:pass@example.com",     // carries credentials
        "https://example.com/path",          // has a path
        "https://example.com?q=1",           // has a query
        "https://example.com#frag",          // has a fragment
        "https://*.example.com",             // wildcard
        "https://",                          // no host
        "https://exa mple.com",              // whitespace
        "https://example.com; script-src *", // CSP injection
        "https://example.com\n",             // control character
        "https://example.com:notaport",      // bad port
        "https://-example.com",              // invalid label
        "https://example..com",              // empty label
    ] {
        assert!(
            Origin::parse(value).is_err(),
            "`{value}` must not parse as an origin"
        );
    }
}

#[test]
fn a_request_url_yields_its_origin_while_an_allowlist_entry_may_not_have_a_path() {
    // Two jobs, two functions. An allowlist entry with a path is a mistake
    // worth refusing — the user wrote something that does not mean what they
    // think. A request URL with a path is ordinary.
    assert!(Origin::parse("https://api.example.com/v1/things?q=1").is_err());
    assert_eq!(
        Origin::of_url("https://api.example.com/v1/things?q=1")
            .expect("a request URL has an origin")
            .to_string(),
        "https://api.example.com"
    );

    for (url, expected) in [
        ("https://a.test", "https://a.test"),
        ("https://a.test/", "https://a.test"),
        ("https://a.test:8443/deep/path", "https://a.test:8443"),
        ("http://a.test/x#frag", "http://a.test"),
        ("https://[2606:4700::1111]/x", "https://[2606:4700::1111]"),
    ] {
        assert_eq!(
            Origin::of_url(url).expect("parses").to_string(),
            expected,
            "{url}"
        );
    }

    // What `parse` refuses for being dangerous is still refused here: a
    // credential in the URL, a non-HTTP scheme, whitespace.
    for url in [
        "https://user:pass@api.example.com/x",
        "file:///etc/passwd",
        "ftp://example.com/x",
        "https://exa mple.com/x",
        "api.example.com/x",
    ] {
        assert!(Origin::of_url(url).is_err(), "{url}");
    }
}

#[test]
fn parses_ip_literals_including_ipv6() {
    assert_eq!(
        origin("https://[2606:4700::1111]").host(),
        "[2606:4700::1111]"
    );
    assert_eq!(origin("https://[2606:4700::1111]:8443").port(), 8443);
    assert_eq!(
        origin("https://93.184.216.34").literal_address(),
        Some(ip("93.184.216.34"))
    );
    // A name is not an address, and must not be mistaken for one.
    assert_eq!(origin("https://example.com").literal_address(), None);
}

// ---------------------------------------------------------------- allowlist

#[test]
fn policy_is_the_intersection_of_request_and_approval() {
    let declared = [origin("https://a.test"), origin("https://b.test")];
    let approved = [origin("https://b.test"), origin("https://c.test")];
    let policy = NetworkPolicy::new(&declared, &approved);

    // Declared but not approved: the user did not say yes.
    assert!(policy.check_origin(&origin("https://a.test")).is_err());
    // Both: allowed.
    assert!(policy.check_origin(&origin("https://b.test")).is_ok());
    // Approved but never declared: the manifest did not ask, so it is not a
    // grant this application holds.
    assert!(policy.check_origin(&origin("https://c.test")).is_err());
}

#[test]
fn an_empty_allowlist_reaches_nothing() {
    let policy = NetworkPolicy::new(&[], &[]);
    assert!(policy.check_origin(&origin("https://a.test")).is_err());
    assert!(policy.approved().is_empty());
}

// ---------------------------------------------------------------- addresses

#[test]
fn blocks_every_address_range_that_reaches_the_user() {
    for value in [
        // The prize: cloud instance metadata.
        "169.254.169.254",
        // Loopback, in its many spellings.
        "127.0.0.1",
        "127.1.2.3",
        "0.0.0.0",
        // RFC 1918.
        "10.0.0.1",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.1",
        // Carrier-grade NAT.
        "100.64.0.1",
        // Benchmarking, documentation, protocol assignments.
        "198.18.0.1",
        "192.0.2.1",
        "198.51.100.1",
        "203.0.113.1",
        "192.0.0.1",
        "192.88.99.1",
        // Multicast, reserved, broadcast.
        "224.0.0.1",
        "240.0.0.1",
        "255.255.255.255",
        // IPv6 equivalents.
        "::",
        "::1",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "ff02::1",
        "2001:db8::1",
        "100::1",
    ] {
        assert!(
            classify(ip(value)).is_some(),
            "{value} must be refused as a destination"
        );
    }
}

#[test]
fn unwraps_ipv4_hidden_inside_ipv6() {
    // Each of these is 127.0.0.1 or 169.254.169.254 wearing a different hat.
    // An implementation that checks `is_loopback()` on the v6 address and
    // stops there connects to every one of them.
    for value in [
        "::ffff:127.0.0.1",       // IPv4-mapped, dotted
        "::ffff:7f00:1",          // IPv4-mapped, hex
        "::ffff:169.254.169.254", // IPv4-mapped metadata
        "::127.0.0.1",            // IPv4-compatible (deprecated)
        "64:ff9b::127.0.0.1",     // NAT64
        "64:ff9b::a9fe:a9fe",     // NAT64 metadata, hex
        "2002:7f00:1::",          // 6to4
        "2002:a9fe:a9fe::",       // 6to4 metadata
    ] {
        assert!(
            classify(ip(value)).is_some(),
            "{value} is a private destination in disguise and must be refused"
        );
    }
}

#[test]
fn allows_ordinary_public_addresses() {
    for value in [
        "93.184.216.34", // example.com
        "1.1.1.1",       // a public resolver
        "8.8.8.8",
        "172.32.0.1",      // just outside 172.16/12
        "100.128.0.1",     // just outside the CGNAT block
        "2606:4700::1111", // public IPv6
        "2001:4860:4860::8888",
    ] {
        assert_eq!(
            classify(ip(value)),
            None,
            "{value} is a normal public destination and must be allowed"
        );
    }
}

#[test]
fn resolution_keeps_only_the_addresses_that_passed() {
    let policy = NetworkPolicy::new(&[origin("https://a.test")], &[origin("https://a.test")]);

    // A name resolving to both public and private addresses is served by the
    // public one — a browser would do the same.
    let usable = policy
        .usable_addresses("a.test", &[ip("127.0.0.1"), ip("93.184.216.34")])
        .expect("one address is usable");
    assert_eq!(usable, vec![ip("93.184.216.34")]);

    // A name resolving only into private space is refused outright, rather
    // than falling back to "well, connect anyway".
    assert!(policy
        .usable_addresses("a.test", &[ip("127.0.0.1"), ip("10.0.0.1")])
        .is_err());
}

#[test]
fn enterprise_policy_can_permit_private_ranges_and_nothing_enables_it_by_default() {
    let default = NetworkPolicy::new(&[], &[]);
    assert!(default.check_address("intranet", ip("10.0.0.5")).is_err());

    let managed = NetworkPolicy::new(&[], &[]).with_private_addresses_allowed(true);
    assert!(managed.check_address("intranet", ip("10.0.0.5")).is_ok());
}

#[test]
fn a_blocked_address_tells_the_application_nothing_useful() {
    let policy = NetworkPolicy::new(&[], &[]);
    let err = policy
        .check_address("metadata.internal", ip("169.254.169.254"))
        .expect_err("must be refused");

    // The reader's own diagnostic is specific, because the user reads it.
    let diagnostic = err.to_string();
    assert!(diagnostic.contains("169.254.169.254"));
    assert!(diagnostic.contains("link-local"));

    // What the application sees carries none of it: no address, no host, no
    // hint about the shape of the user's network.
    let safe = err.safe();
    assert_eq!(safe.code.as_str(), "PERMISSION_DENIED");
    assert!(!safe.to_string().contains("169.254"));
    assert!(!safe.to_string().contains("metadata.internal"));
}
