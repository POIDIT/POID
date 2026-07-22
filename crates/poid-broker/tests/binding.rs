//! Binding a manifest's declared need to a configured backend (SPEC §7.2.3).
//!
//! The property under test throughout: **the application is never an input.**
//! Every case here drives the decision from the manifest plus the reader's
//! records, and there is no argument an attacker-controlled message could
//! reach.

use poid_broker::binding::{candidates, resolve, unbound, Binding, BindingRequest, Resolved};
use poid_broker::{ConnectionId, ConnectionKind, ConnectionRef, Origin};
use poid_core::{ExtraFields, RequireKind, StorageRequires};

fn connection(id: &str, kind: ConnectionKind, label: &str) -> ConnectionRef {
    ConnectionRef {
        id: ConnectionId::new(id),
        kind,
        label: label.to_owned(),
        origins: Vec::new(),
    }
}

fn request(kind: RequireKind, hint: Option<&str>) -> BindingRequest {
    BindingRequest {
        require: kind,
        hint: hint.map(str::to_owned),
    }
}

#[test]
fn reads_the_request_straight_out_of_the_manifest() {
    let requires = StorageRequires {
        kind: RequireKind::Sql,
        hint: Some("supabase".to_owned()),
        extra: ExtraFields::new(),
    };
    let parsed = BindingRequest::from_manifest(&requires);
    assert_eq!(parsed.require, RequireKind::Sql);
    assert_eq!(parsed.hint.as_deref(), Some("supabase"));
}

#[test]
fn satisfies_follows_the_spec_table() {
    use ConnectionKind as K;
    use RequireKind as R;

    // A relational backend can serve the tiers that layer over it.
    assert!(K::Sql.satisfies(R::Sql));
    assert!(K::Sql.satisfies(R::Kv));
    assert!(K::Sql.satisfies(R::Docs));

    // The reverse never holds: kv cannot answer a query.
    assert!(!K::Kv.satisfies(R::Sql));
    assert!(!K::Docs.satisfies(R::Sql));
    assert!(!K::Kv.satisfies(R::Docs));

    // Same-kind matches.
    assert!(K::Kv.satisfies(R::Kv));
    assert!(K::Docs.satisfies(R::Docs));
    assert!(K::Files.satisfies(R::Files));

    // Non-storage kinds satisfy no storage requirement at all. An application
    // must not be able to have its data "stored" in a model provider.
    for kind in [K::Ai, K::Mcp, K::Net, K::Sync] {
        for require in [R::Kv, R::Sql, R::Docs, R::Files] {
            assert!(
                !kind.satisfies(require),
                "{kind} must not satisfy a {require:?} requirement"
            );
        }
    }
}

#[test]
fn offers_only_connections_that_can_serve_the_need() {
    let available = [
        connection("c1", ConnectionKind::Ai, "my-anthropic"),
        connection("c2", ConnectionKind::Sql, "work-postgres"),
        connection("c3", ConnectionKind::Kv, "some-kv"),
    ];
    let offered = candidates(&request(RequireKind::Sql, None), &available);
    let labels: Vec<&str> = offered.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["work-postgres"]);
}

#[test]
fn the_hint_orders_the_choices_and_never_makes_one() {
    let available = [
        connection("c1", ConnectionKind::Sql, "work-postgres"),
        connection("c2", ConnectionKind::Sql, "my-supabase"),
    ];
    let offered = candidates(&request(RequireKind::Sql, Some("supabase")), &available);

    // The hinted one is offered first...
    assert_eq!(offered[0].label, "my-supabase");
    // ...and the other is still offered. A hint that removed choices would be
    // a manifest field selecting the user's database.
    assert_eq!(offered.len(), 2);
    assert_eq!(offered[1].label, "work-postgres");
}

#[test]
fn keep_local_always_resolves() {
    // Even with nothing configured at all: the user's right to decline every
    // connection does not depend on having one (SPEC §7.2.3 clause 2).
    let resolved = resolve(&request(RequireKind::Sql, None), &Binding::KeepLocal, &[])
        .expect("keeping data local is always available");
    assert_eq!(resolved, Resolved::Local);
}

#[test]
fn a_recorded_binding_is_rechecked_against_the_world_as_it_is_now() {
    let request = request(RequireKind::Sql, None);
    let bound = Binding::Connection(ConnectionId::new("c1"));

    // Deleted since the user chose it.
    let err = resolve(&request, &bound, &[]).expect_err("a deleted connection cannot resolve");
    assert_eq!(err.safe().code.as_str(), "CONNECTION_REQUIRED");

    // Edited into a kind that can no longer serve this application.
    let now = [connection("c1", ConnectionKind::Kv, "was-postgres")];
    let err = resolve(&request, &bound, &now).expect_err("a kv backend cannot serve sql");
    assert_eq!(err.safe().code.as_str(), "CONNECTION_REQUIRED");

    // Still valid.
    let now = [connection("c1", ConnectionKind::Sql, "work-postgres")];
    let resolved = resolve(&request, &bound, &now).expect("still satisfiable");
    assert_eq!(resolved, Resolved::Connection(&now[0]));
}

#[test]
fn an_unbound_connection_call_is_connection_required() {
    let err = unbound(&request(RequireKind::Sql, None));
    assert_eq!(err.safe().code.as_str(), "CONNECTION_REQUIRED");
    // The application learns the shape of its own manifest, which it already
    // knows — and nothing about what the user has configured.
    assert!(!err.safe().to_string().contains("postgres"));
}

#[test]
fn credentials_are_attached_by_exact_origin_only() {
    let parse = |v: &str| Origin::parse(v).expect("valid origin");
    let conn = ConnectionRef {
        id: ConnectionId::new("c1"),
        kind: ConnectionKind::Net,
        label: "my-api".to_owned(),
        origins: vec![parse("https://api.example.com")],
    };

    assert!(conn.covers(&parse("https://api.example.com")));
    assert!(conn.covers(&parse("https://api.example.com:443")));

    // The near-misses an attacker registers precisely because they look right.
    assert!(!conn.covers(&parse("https://api.example.com.attacker.test")));
    assert!(!conn.covers(&parse("http://api.example.com")));
    assert!(!conn.covers(&parse("https://api.example.com:8443")));
    assert!(!conn.covers(&parse("https://evil.api.example.com")));
}
