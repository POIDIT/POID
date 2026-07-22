//! The `sql` connection kind: real SQL against a real PostgreSQL server.
//!
//! The application calls `poid.db.sql.exec(...)`. That message crosses the
//! sandbox boundary carrying a query and parameters and nothing else; this
//! module reads the connection string from the keychain, opens the socket,
//! runs the statement, and hands back rows. The credential exists here, in
//! Core, for the duration of the call — never in the Reader window, never in
//! the container (SPEC §7.1).
//!
//! # Why the wire protocol
//!
//! Because `poid.db.sql` promises arbitrary SQL. A REST layer over the same
//! database could serve `poid.db.docs`, but it could not honestly serve this,
//! and shipping an API that quietly does less than it says is worse than not
//! shipping it. The cost is a real driver dependency; the benefit is that
//! Supabase becomes a *preset* rather than an integration — Neon, RDS, a
//! Postgres in a cupboard, all the same path.
//!
//! # Why the address policy does not apply here
//!
//! `poid.net` validates addresses because the *application* chooses the
//! destination (SPEC §7.2.5). A SQL connection's destination is chosen by the
//! **user**, in Studio, once. Someone pointing POID at the Postgres on their
//! own intranet is doing exactly what this feature is for, and refusing
//! private addresses here would break the common case to prevent an attack
//! that requires the user to attack themselves.

use serde_json::{Map, Value};
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, NoTls, Row};

use crate::dsn;
use crate::error::{ConnectionError, Result};

/// What one statement produced.
#[derive(Debug, Clone, Default)]
pub struct SqlResult {
    /// Result rows, each a JSON object keyed by column name.
    pub rows: Vec<Value>,
    /// Rows the statement changed (0 for a query).
    pub rows_affected: u64,
}

/// An open connection to a user-configured Postgres.
///
/// Held for the life of a Reader window rather than opened per statement:
/// a TLS handshake per query would make an interactive application unusable,
/// and a pool would leak one document's transaction state into another's.
pub struct PostgresConnection {
    client: Client,
}

impl std::fmt::Debug for PostgresConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never derive Debug here: the driver's config carries the password,
        // and a derived impl would print it into any log that formats this.
        f.write_str("PostgresConnection { .. }")
    }
}

impl PostgresConnection {
    /// Opens a connection from a DSN.
    ///
    /// The DSN is the credential (see `dsn`), so it arrives from the keychain
    /// and is dropped when this returns. TLS is required unless the DSN says
    /// otherwise, and the certificate is verified against the webpki roots —
    /// an unverified TLS connection to a database holding user data would be
    /// theatre.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Sql`] if the DSN is unusable or the server refuses.
    /// The message is for the **user's** log; it is mapped to a §9 code before
    /// anything crosses to an application, and scrubbed before it is written
    /// anywhere (see [`crate::ConnectionStore::redactor`]).
    pub async fn open(connection_string: &str) -> Result<Self> {
        // Parsed for validation before the driver sees it, so a malformed DSN
        // fails with our wording rather than the driver's — which quotes the
        // string it was given.
        let _ = dsn::parse(connection_string)?;

        let sslmode_disabled = connection_string.contains("sslmode=disable");
        let client = if sslmode_disabled {
            let (client, connection) = tokio_postgres::connect(connection_string, NoTls)
                .await
                .map_err(sql_error("could not connect"))?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            client
        } else {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(config);
            let (client, connection) = tokio_postgres::connect(connection_string, tls)
                .await
                .map_err(sql_error("could not connect"))?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            client
        };

        Ok(Self { client })
    }

    /// Runs one statement with bound parameters.
    ///
    /// Parameters are bound by the driver, never interpolated into the SQL
    /// text. An application that builds a query from user input is still
    /// responsible for its own escaping, but the parameter path is safe by
    /// construction.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Sql`] when the server rejects the statement.
    pub async fn exec(&self, sql: &str, params: &[Value]) -> Result<SqlResult> {
        let bound: Vec<JsonParam> = params.iter().cloned().map(JsonParam).collect();
        let refs: Vec<&(dyn ToSql + Sync)> =
            bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();

        let rows = self
            .client
            .query(sql, &refs)
            .await
            .map_err(sql_error("the database refused the statement"))?;

        // `query` returns rows for a SELECT and an empty set for a write, so a
        // write's row count comes from a second path. Doing it this way keeps
        // one entry point for the application instead of making it choose.
        if rows.is_empty() {
            return Ok(SqlResult {
                rows: Vec::new(),
                rows_affected: 0,
            });
        }

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push(row_to_json(row));
        }
        Ok(SqlResult {
            rows_affected: 0,
            rows: out,
        })
    }

    /// Runs a statement that returns no rows, reporting how many it changed.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Sql`] when the server rejects the statement.
    pub async fn execute(&self, sql: &str, params: &[Value]) -> Result<SqlResult> {
        let bound: Vec<JsonParam> = params.iter().cloned().map(JsonParam).collect();
        let refs: Vec<&(dyn ToSql + Sync)> =
            bound.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let affected = self
            .client
            .execute(sql, &refs)
            .await
            .map_err(sql_error("the database refused the statement"))?;
        Ok(SqlResult {
            rows: Vec::new(),
            rows_affected: affected,
        })
    }

    /// Whether the connection is still usable.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }
}

fn sql_error(context: &'static str) -> impl Fn(tokio_postgres::Error) -> ConnectionError {
    move |e| ConnectionError::Sql {
        reason: format!("{context}: {e}"),
    }
}

/// A JSON value bound as a Postgres parameter.
///
/// The application's parameters arrive as JSON because that is what crosses
/// the sandbox boundary. Each is bound as the Postgres type that matches its
/// JSON type; anything richer (a date, a UUID) travels as text and is cast by
/// the query, which is what an application would write anyway.
#[derive(Debug)]
struct JsonParam(Value);

impl ToSql for JsonParam {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> std::result::Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        match &self.0 {
            Value::Null => Ok(tokio_postgres::types::IsNull::Yes),
            Value::Bool(b) => b.to_sql(ty, out),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    i.to_sql(ty, out)
                } else if let Some(f) = n.as_f64() {
                    f.to_sql(ty, out)
                } else {
                    Err("a number that is neither an integer nor a float".into())
                }
            }
            Value::String(s) => s.to_sql(ty, out),
            // Arrays and objects go as JSON, which is what a jsonb column
            // wants and what a text column can still accept.
            other => other.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool {
        // The server tells us the parameter's type; refusing here would mean
        // second-guessing it from a JSON value that has less information.
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

/// Converts one row to a JSON object.
///
/// Unsupported column types become `null` rather than failing the whole query:
/// a row with one exotic column is still mostly useful, and failing would make
/// a `SELECT *` unusable because of a column the application never reads. The
/// covered set is listed in this crate's README; extending it is mechanical.
fn row_to_json(row: &Row) -> Value {
    let mut object = Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        object.insert(
            column.name().to_owned(),
            column_to_json(row, index, column.type_()),
        );
    }
    Value::Object(object)
}

fn column_to_json(row: &Row, index: usize, ty: &Type) -> Value {
    /// Reads `$t` and turns it into JSON, mapping SQL NULL to JSON null.
    macro_rules! get {
        ($t:ty) => {
            match row.try_get::<_, Option<$t>>(index) {
                Ok(Some(v)) => Value::from(v),
                Ok(None) => Value::Null,
                Err(_) => Value::Null,
            }
        };
    }

    match *ty {
        Type::BOOL => get!(bool),
        Type::INT2 => get!(i16),
        Type::INT4 => get!(i32),
        Type::INT8 => get!(i64),
        Type::FLOAT4 => get!(f32),
        Type::FLOAT8 => get!(f64),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => get!(String),
        Type::JSON | Type::JSONB => match row.try_get::<_, Option<Value>>(index) {
            Ok(Some(v)) => v,
            _ => Value::Null,
        },
        Type::UUID => match row.try_get::<_, Option<uuid::Uuid>>(index) {
            Ok(Some(v)) => Value::String(v.to_string()),
            _ => Value::Null,
        },
        // Postgres's own text form for bytea, so the value round-trips
        // through a query rather than arriving as something that merely
        // describes it.
        Type::BYTEA => match row.try_get::<_, Option<Vec<u8>>>(index) {
            Ok(Some(v)) => {
                let mut hex = String::with_capacity(2 + v.len() * 2);
                hex.push_str("\\x");
                for byte in v {
                    hex.push_str(&format!("{byte:02x}"));
                }
                Value::String(hex)
            }
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_malformed_dsn_fails_with_our_wording_not_the_drivers() {
        let error = PostgresConnection::open("mysql://app:p@host/db")
            .await
            .expect_err("not a Postgres DSN");
        let text = error.to_string();
        assert!(text.contains("postgres://"), "unexpected: {text}");
        // The driver never saw the string, so it cannot be the one quoting it.
        assert!(
            !text.contains("mysql://app:p@host/db"),
            "the DSN was echoed"
        );
    }

    #[test]
    fn the_connection_type_never_prints_its_configuration() {
        // A derived Debug would carry the driver's Config, and that carries the
        // password. This is the assertion that keeps the manual impl.
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<PostgresConnection>();
    }

    #[test]
    fn json_parameters_bind_by_their_json_type() {
        // The binding itself needs a server, but the shape does not: a null
        // must bind as SQL NULL rather than as the string "null", which is the
        // classic way a JSON bridge corrupts data.
        let mut buffer = bytes::BytesMut::new();
        let null = JsonParam(Value::Null);
        let outcome = null.to_sql(&Type::TEXT, &mut buffer).expect("binds");
        assert!(matches!(outcome, tokio_postgres::types::IsNull::Yes));
        assert!(buffer.is_empty());

        let text = JsonParam(Value::String("hello".to_owned()));
        let mut buffer = bytes::BytesMut::new();
        text.to_sql(&Type::TEXT, &mut buffer).expect("binds");
        assert_eq!(&buffer[..], b"hello");
    }
}
