//! Property-based tests for the broker's policy (CONVENTIONS: this crate and
//! `poid-core` require them, because these are the code paths an attacker
//! controls).
//!
//! Unit tests check the destinations we thought of. These check the ones we
//! did not: the invariants below must hold for *every* input, including the
//! ones nobody wrote a table row for.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use poid_broker::binding::{candidates, resolve, Binding, BindingRequest};
use poid_broker::network::classify;
use poid_broker::{ConnectionId, ConnectionKind, ConnectionRef, NetworkPolicy, Origin, Redactor};
use poid_core::RequireKind;
use proptest::prelude::*;

fn any_require() -> impl Strategy<Value = RequireKind> {
    prop_oneof![
        Just(RequireKind::Kv),
        Just(RequireKind::Sql),
        Just(RequireKind::Docs),
        Just(RequireKind::Files),
    ]
}

fn any_kind() -> impl Strategy<Value = ConnectionKind> {
    prop_oneof![
        Just(ConnectionKind::Kv),
        Just(ConnectionKind::Sql),
        Just(ConnectionKind::Docs),
        Just(ConnectionKind::Files),
        Just(ConnectionKind::Ai),
        Just(ConnectionKind::Mcp),
        Just(ConnectionKind::Net),
        Just(ConnectionKind::Sync),
    ]
}

/// Every IPv4 range the broker refuses, as (network, prefix length).
///
/// Kept here rather than imported so the test states the expectation
/// independently of the implementation: if someone deletes a range from
/// `classify`, this list still demands it be blocked and the property fails.
const BLOCKED_V4_RANGES: &[([u8; 4], u32)] = &[
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 0, 2, 0], 24),
    ([192, 88, 99, 0], 24),
    ([192, 168, 0, 0], 16),
    ([198, 18, 0, 0], 15),
    ([198, 51, 100, 0], 24),
    ([203, 0, 113, 0], 24),
    ([224, 0, 0, 0], 4),
    ([240, 0, 0, 0], 4),
];

/// An address drawn from somewhere inside a blocked range.
///
/// Generated rather than filtered: random IPv4 is ~93% public, so a
/// `prop_assume!` here throws away almost every case and proptest gives up
/// before it has tested anything interesting.
fn blocked_v4() -> impl Strategy<Value = Ipv4Addr> {
    (0..BLOCKED_V4_RANGES.len(), any::<u32>()).prop_map(|(index, host_bits)| {
        let (network, prefix) = BLOCKED_V4_RANGES[index];
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        let base = u32::from(Ipv4Addr::from(network)) & mask;
        Ipv4Addr::from(base | (host_bits & !mask))
    })
}

fn any_connection() -> impl Strategy<Value = ConnectionRef> {
    (any_kind(), "[a-z0-9-]{1,12}", "[a-zA-Z0-9 -]{0,20}").prop_map(|(kind, id, label)| {
        ConnectionRef {
            id: ConnectionId::new(id),
            kind,
            label,
            origins: Vec::new(),
        }
    })
}

proptest! {
    /// No string whatsoever parses into an origin that could break out of a
    /// CSP directive. This is the property that lets `buildCsp` interpolate an
    /// approved origin without escaping it.
    #[test]
    fn a_parsed_origin_can_never_break_a_csp_directive(value in ".*") {
        if let Ok(origin) = Origin::parse(&value) {
            let rendered = origin.to_string();
            for forbidden in [' ', ';', '\'', '"', ',', '\n', '\r', '\t', '*'] {
                prop_assert!(
                    !rendered.contains(forbidden),
                    "`{rendered}` contains `{forbidden}`, which would escape a CSP directive"
                );
            }
        }
    }

    /// Parsing is idempotent: an origin's rendered form parses back to itself.
    /// Without this, two readers could disagree on whether an allowlist entry
    /// matches a request.
    #[test]
    fn origin_rendering_round_trips(value in ".*") {
        if let Ok(origin) = Origin::parse(&value) {
            let reparsed = Origin::parse(&origin.to_string())
                .expect("an origin's own rendering must parse");
            prop_assert_eq!(origin, reparsed);
        }
    }

    /// Origin parsing never panics, on any input at all.
    #[test]
    fn origin_parsing_never_panics(value in ".*") {
        let _ = Origin::parse(&value);
    }

    /// **The load-bearing one.** No allowlist an application can request, and
    /// no approval a user can give, makes a blocked address reachable. The
    /// allowlist governs *names*; the address check governs *destinations*,
    /// and no amount of the former unlocks the latter.
    #[test]
    fn no_allowlist_ever_unlocks_a_blocked_address(
        v4 in blocked_v4(),
        hosts in prop::collection::vec("[a-z]{1,10}\\.test", 0..4),
    ) {
        let address = IpAddr::V4(v4);
        // The range table and the classifier must agree — if they ever stop
        // agreeing, that is the bug, and it surfaces here rather than in
        // production.
        prop_assert!(
            classify(address).is_some(),
            "{} is in a blocked range but the classifier allows it",
            v4
        );

        let origins: Vec<Origin> = hosts
            .iter()
            .filter_map(|h| Origin::parse(&format!("https://{h}")).ok())
            .collect();
        let policy = NetworkPolicy::new(&origins, &origins);

        prop_assert!(policy.check_address("anything", address).is_err());
        prop_assert!(policy.usable_addresses("anything", &[address]).is_err());
    }

    /// The v6 classifier agrees with the v4 classifier about every address
    /// that is really an IPv4 address in disguise. This is the invariant that
    /// closes `::ffff:169.254.169.254` and its relatives, for all of them
    /// rather than the eight the unit test lists.
    #[test]
    fn embedded_ipv4_is_judged_as_ipv4(octets in any::<[u8; 4]>()) {
        let v4 = Ipv4Addr::from(octets);
        let expected = classify(IpAddr::V4(v4));
        let [a, b, c, d] = octets;
        let (hi, lo) = (u16::from(a) << 8 | u16::from(b), u16::from(c) << 8 | u16::from(d));

        let mapped = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, hi, lo);
        let nat64 = Ipv6Addr::new(0x0064, 0xff9b, 0, 0, 0, 0, hi, lo);
        let sixtofour = Ipv6Addr::new(0x2002, hi, lo, 0, 0, 0, 0, 0);

        for disguised in [mapped, nat64, sixtofour] {
            prop_assert_eq!(
                classify(IpAddr::V6(disguised)),
                expected,
                "{} embeds {} and must be judged the same way",
                disguised,
                v4
            );
        }
    }

    /// Address classification is total and never panics.
    #[test]
    fn classification_never_panics(v4 in any::<[u8; 4]>(), v6 in any::<[u16; 8]>()) {
        let _ = classify(IpAddr::V4(Ipv4Addr::from(v4)));
        let _ = classify(IpAddr::V6(Ipv6Addr::from(v6)));
    }

    /// An application's storage requirement is only ever offered backends that
    /// can serve it — whatever mix of connections the user has configured.
    #[test]
    fn candidates_always_satisfy_the_requirement(
        require in any_require(),
        hint in prop::option::of("[a-z]{0,8}"),
        available in prop::collection::vec(any_connection(), 0..8),
    ) {
        let request = BindingRequest { require, hint };
        for candidate in candidates(&request, &available) {
            prop_assert!(candidate.satisfies(require));
        }
    }

    /// Offering is not filtering-by-luck: every connection that *can* serve the
    /// requirement is offered, so the user is never quietly denied a choice
    /// they configured.
    #[test]
    fn candidates_omit_nothing_that_qualifies(
        require in any_require(),
        available in prop::collection::vec(any_connection(), 0..8),
    ) {
        let request = BindingRequest { require, hint: None };
        let offered = candidates(&request, &available).len();
        let qualifying = available.iter().filter(|c| c.satisfies(require)).count();
        prop_assert_eq!(offered, qualifying);
    }

    /// Resolving a binding can never hand back a connection that cannot serve
    /// the requirement — no matter what was recorded or what exists now.
    #[test]
    fn resolve_never_returns_an_unsatisfying_connection(
        require in any_require(),
        id in "[a-z0-9-]{1,12}",
        available in prop::collection::vec(any_connection(), 0..8),
    ) {
        let request = BindingRequest { require, hint: None };
        let binding = Binding::Connection(ConnectionId::new(id));
        if let Ok(poid_broker::Resolved::Connection(c)) = resolve(&request, &binding, &available) {
            prop_assert!(c.satisfies(require));
        }
    }

    /// Redaction removes every registered secret, whatever it is embedded in.
    #[test]
    fn redaction_leaves_no_secret_behind(
        secret in "[a-zA-Z0-9!@#$%^&*()_+/=-]{6,40}",
        prefix in ".{0,40}",
        suffix in ".{0,40}",
    ) {
        let mut redactor = Redactor::new();
        redactor.insert(secret.clone());

        let text = format!("{prefix}{secret}{suffix}");
        let cleaned = redactor.redact(&text);

        prop_assert!(!cleaned.contains(&secret), "`{cleaned}` still contains the secret");
        prop_assert!(!redactor.leaks(&cleaned));
    }

    /// Redaction catches a secret that a URL builder has percent-encoded on
    /// its way into an error message — the form a database driver actually
    /// hands back when the connection string is malformed.
    #[test]
    fn redaction_survives_percent_encoding(
        secret in "[a-zA-Z0-9!@#$%^&*()_+/=-]{6,40}",
    ) {
        let mut redactor = Redactor::new();
        redactor.insert(secret.clone());

        let encoded: String = secret
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();

        let message = format!("connection failed: {encoded}");
        let cleaned = redactor.redact(&message);
        prop_assert!(!redactor.leaks(&cleaned));
    }

    /// Redaction never touches text that holds no secret. A redactor that
    /// mangled ordinary diagnostics would be turned off, and then it would
    /// protect nothing.
    #[test]
    fn redaction_leaves_innocent_text_alone(
        secret in "[a-z]{20,40}",
        text in "[A-Z0-9 .:-]{0,60}",
    ) {
        let mut redactor = Redactor::new();
        redactor.insert(secret);
        prop_assert_eq!(redactor.redact(&text), text);
    }
}
