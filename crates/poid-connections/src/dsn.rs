//! Parsing a PostgreSQL connection string into the parts that are safe to
//! show and the parts that are not.
//!
//! A DSN is a single string that mixes both: `postgres://user:hunter2@db
//! .example.com:5432/app`. The user needs to see *which* database a connection
//! points at; nobody needs to see the password. So the whole DSN is stored in
//! the keychain, and only a [`SqlTarget`] — the same string with the
//! credentials removed — is written to the registry file.
//!
//! Deriving the display form here, rather than accepting one from a caller,
//! is deliberate: it means there is exactly one place where a DSN turns into
//! something persistable, and that place is auditable in isolation.

use serde::{Deserialize, Serialize};

use crate::error::{ConnectionError, Result};

/// The non-secret parts of a SQL connection: enough to identify the database,
/// never enough to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlTarget {
    /// Hostname or IP literal.
    pub host: String,
    /// Port, defaulted to 5432 when the DSN omits it.
    pub port: u16,
    /// Database name.
    pub database: String,
    /// The role the connection authenticates as.
    ///
    /// A username is not a credential — it is half of one, and showing it is
    /// how the user tells `work-postgres` from `work-postgres-readonly`.
    pub user: String,
}

impl SqlTarget {
    /// A one-line description for the connection manager.
    #[must_use]
    pub fn display(&self) -> String {
        format!(
            "{}@{}:{}/{}",
            self.user, self.host, self.port, self.database
        )
    }
}

/// A parsed DSN: the display parts, plus the password held separately so the
/// caller cannot persist it by accident.
#[derive(Debug, Clone)]
pub struct ParsedDsn {
    /// The parts that may be written down.
    pub target: SqlTarget,
    /// The part that may not.
    pub password: String,
}

/// Parses a `postgres://` / `postgresql://` connection string.
///
/// # Errors
///
/// [`ConnectionError::InvalidConfig`] when the string is not a DSN this crate
/// can connect with. Refusals are strict on purpose: a DSN that "mostly"
/// parses is a DSN that connects somewhere other than the user believes.
pub fn parse(dsn: &str) -> Result<ParsedDsn> {
    let bad = |why: &'static str| ConnectionError::InvalidConfig { why };

    let rest = dsn
        .strip_prefix("postgresql://")
        .or_else(|| dsn.strip_prefix("postgres://"))
        .ok_or_else(|| bad("it must start with postgres:// or postgresql://"))?;

    // Query parameters (sslmode, application_name, …) are kept in the stored
    // DSN and dropped from the display form.
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);

    // The password may itself contain '@' once percent-decoded, so the
    // authority split is at the *last* '@'.
    let (userinfo, hostpath) = rest.rsplit_once('@').ok_or_else(|| bad("it has no user"))?;

    let (user, password) = match userinfo.split_once(':') {
        Some((u, p)) => (u, p),
        None => (userinfo, ""),
    };
    let user = percent_decode(user);
    let password = percent_decode(password);
    if user.is_empty() {
        return Err(bad("it has no user"));
    }

    let (authority, database) = hostpath
        .split_once('/')
        .ok_or_else(|| bad("it names no database"))?;
    let database = percent_decode(database);
    if database.is_empty() {
        return Err(bad("it names no database"));
    }

    let (host, port) = split_host_port(authority).ok_or_else(|| bad("its port is not a number"))?;
    if host.is_empty() {
        return Err(bad("it has no host"));
    }

    Ok(ParsedDsn {
        target: SqlTarget {
            host,
            port,
            database,
            user,
        },
        password,
    })
}

fn split_host_port(authority: &str) -> Option<(String, u16)> {
    const DEFAULT_PORT: u16 = 5432;
    if let Some(end) = authority.find(']') {
        // Bracketed IPv6 literal; the port colon is the one after ']'.
        let (host, tail) = authority.split_at(end + 1);
        return match tail {
            "" => Some((host.to_owned(), DEFAULT_PORT)),
            _ => tail
                .strip_prefix(':')
                .and_then(|p| p.parse().ok())
                .map(|port| (host.to_owned(), port)),
        };
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => port.parse().ok().map(|port| (host.to_owned(), port)),
        None => Some((authority.to_owned(), DEFAULT_PORT)),
    }
}

/// Decodes `%XX` escapes. Invalid escapes are left as written rather than
/// dropped: silently altering a credential produces a confusing auth failure
/// instead of an honest parse error.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_dsn() {
        let parsed = parse("postgres://app:hunter2@db.example.com:6543/appdb")
            .expect("a well-formed DSN parses");
        assert_eq!(parsed.target.host, "db.example.com");
        assert_eq!(parsed.target.port, 6543);
        assert_eq!(parsed.target.database, "appdb");
        assert_eq!(parsed.target.user, "app");
        assert_eq!(parsed.password, "hunter2");
    }

    #[test]
    fn defaults_the_port_and_accepts_both_schemes() {
        for dsn in [
            "postgres://app:p@db.example.com/appdb",
            "postgresql://app:p@db.example.com/appdb",
        ] {
            let parsed = parse(dsn).expect("parses");
            assert_eq!(parsed.target.port, 5432);
        }
    }

    #[test]
    fn a_password_containing_at_or_colon_survives() {
        // Both characters are structural in a DSN, and both turn up in real
        // generated passwords. Splitting on the first '@' gets this wrong.
        let parsed = parse("postgres://app:p%40ss:word@db.example.com/appdb").expect("parses");
        assert_eq!(parsed.password, "p@ss:word");
        assert_eq!(parsed.target.host, "db.example.com");
    }

    #[test]
    fn query_parameters_do_not_reach_the_display_form() {
        let parsed =
            parse("postgres://app:p@db.example.com/appdb?sslmode=require").expect("parses");
        assert_eq!(parsed.target.database, "appdb");
        assert!(!parsed.target.display().contains("sslmode"));
    }

    #[test]
    fn the_display_form_never_contains_the_password() {
        let parsed = parse("postgres://app:sup3rs3cret@db.example.com/appdb").expect("parses");
        let shown = parsed.target.display();
        assert!(!shown.contains("sup3rs3cret"));
        assert!(shown.contains("db.example.com"));
        assert!(shown.contains("appdb"));

        // And the same holds once it is serialised into the registry file.
        let json = serde_json::to_string(&parsed.target).expect("serialises");
        assert!(!json.contains("sup3rs3cret"));
    }

    #[test]
    fn parses_an_ipv6_literal_host() {
        let parsed = parse("postgres://app:p@[2001:db8::1]:5433/appdb").expect("parses");
        assert_eq!(parsed.target.host, "[2001:db8::1]");
        assert_eq!(parsed.target.port, 5433);
    }

    #[test]
    fn refuses_a_dsn_it_cannot_connect_with() {
        for dsn in [
            "mysql://app:p@db.example.com/appdb",         // wrong engine
            "db.example.com/appdb",                       // no scheme
            "postgres://db.example.com/appdb",            // no user
            "postgres://app:p@db.example.com",            // no database
            "postgres://app:p@db.example.com/",           // empty database
            "postgres://:p@db.example.com/appdb",         // no user name
            "postgres://app:p@db.example.com:nope/appdb", // bad port
        ] {
            assert!(parse(dsn).is_err(), "`{dsn}` must not parse");
        }
    }
}
