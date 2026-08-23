#![deny(missing_docs)]

//! A [refinery] migration driver for [SurrealDB], so schema migrations can be
//! written in SurrealQL.
//!
//! [refinery]: https://github.com/rust-db/refinery
//! [SurrealDB]: https://surrealdb.com/
//!
//! # Quick start
//!
//! ```no_run
//! use surrealdb_refinery::{MigrationConnection, Runner, load_migrations};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let migrations = load_migrations("migrations")?;
//!
//! let db = surrealdb::engine::any::connect("mem://").await?;
//! // A namespace and database must be selected before migrating.
//! db.use_ns("myapp").await?;
//! db.use_db("myapp").await?;
//!
//! let mut connection = MigrationConnection(&db);
//! let report = Runner::new(&migrations).run_async(&mut connection).await?;
//! println!("applied {} migration(s)", report.applied_migrations().len());
//! # Ok(())
//! # }
//! ```
//!
//! # Migration files
//!
//! Files are named `V{version}__{description}.surql`, matching refinery's
//! convention; `.sql` is also accepted. [`load_migrations`] discovers them at
//! runtime. refinery's own `embed_migrations!` cannot be used, because it only
//! recognises `.sql` and `.rs`.
//!
//! # How migrations are applied
//!
//! Each migration body and the history row recording it are executed in a
//! single SurrealDB transaction, so a migration cannot be applied without being
//! recorded. If any statement fails, the transaction is cancelled and nothing
//! from that migration persists.
//!
//! Because the driver opens that transaction itself, a migration must not open
//! its own — SurrealDB rejects a nested `BEGIN`. For compatibility, a `BEGIN` /
//! `COMMIT` pair wrapping the whole body is detected and removed, so migrations
//! written in that style keep working unchanged.
//!
//! # History table
//!
//! Applied migrations are recorded in `refinery_schema_history`, which can be
//! renamed with `Runner::set_migration_table_name`. The name must be a bare
//! SurrealDB identifier: ASCII letters, digits and underscores, not starting
//! with a digit. refinery interpolates it into its own `INSERT` unquoted, so
//! anything needing quotes is rejected rather than silently mishandled.
//!
//! A `UNIQUE` index on `version` makes double-application an error rather than
//! a duplicate row, which is what protects concurrent runners from each other.
//!
//! # Async only
//!
//! Only refinery's `AsyncMigrate` is implemented; there is no blocking
//! `Migrate` implementation.

mod discover;
mod error;
mod history;
mod sql;

pub use discover::{DiscoverError, MIGRATION_EXTENSIONS, load_migrations};
pub use error::{Error, StatementError};

/// Re-exported from refinery so callers need only depend on this crate.
pub use refinery_core::{Migration, Report, Runner, SchemaVersion, Target};

use async_trait::async_trait;
use refinery_core::traits::r#async::{AsyncMigrate, AsyncQuery, AsyncTransaction};
use surrealdb::engine::any::Any;
use surrealdb::{Connection, Surreal};

use crate::history::HistoryRow;

/// A refinery connection backed by a SurrealDB client.
///
/// This borrows the client rather than owning it, so the same handle can be
/// used for migrations and for application queries:
///
/// ```no_run
/// # use surrealdb_refinery::MigrationConnection;
/// # async fn run(db: &surrealdb::Surreal<surrealdb::engine::any::Any>) {
/// let mut connection = MigrationConnection(db);
/// # let _ = &mut connection;
/// # }
/// ```
///
/// The namespace and database must already be selected. Do not change either
/// while a migration run is in progress: the run holds no session of its own
/// and would continue against the new target.
pub struct MigrationConnection<'a, C: Connection = Any>(pub &'a Surreal<C>);

impl<'a, C: Connection> MigrationConnection<'a, C> {
    /// Wrap a SurrealDB client.
    pub fn new(db: &'a Surreal<C>) -> Self {
        Self(db)
    }

    /// The wrapped client.
    pub fn db(&self) -> &'a Surreal<C> {
        self.0
    }
}

#[async_trait]
impl<C: Connection> AsyncTransaction for MigrationConnection<'_, C> {
    type Error = Error;

    /// Run every statement in one SurrealDB transaction.
    ///
    /// refinery passes the migration body and the history `INSERT` that records
    /// it together, and expects them to be atomic. On any failure the whole
    /// transaction is cancelled, which for SurrealDB rolls back DDL too.
    async fn execute<'a, T: Iterator<Item = &'a str> + Send>(
        &mut self,
        queries: T,
    ) -> Result<usize, Self::Error> {
        let queries: Vec<&str> = queries
            .map(sql::strip_outer_transaction)
            .filter(|query| !query.trim().is_empty())
            .collect();
        if queries.is_empty() {
            return Ok(0);
        }

        let transaction = self.0.clone().begin().await.map_err(Error::Client)?;

        for (index, query) in queries.iter().enumerate() {
            log::debug!(target: "surrealdb_refinery", "statement {index}: {query}");

            let failure = match transaction.query(*query).await {
                Err(error) => Some(Error::Client(error)),
                Ok(mut response) => {
                    let errors = response.take_errors();
                    (!errors.is_empty()).then(|| Error::from_statement_errors(errors))
                }
            };

            if let Some(error) = failure {
                log::error!(target: "surrealdb_refinery", "statement {index} failed, rolling back: {error}");
                if let Err(error) = transaction.cancel().await {
                    log::error!(target: "surrealdb_refinery", "rollback failed: {error}");
                }
                return Err(error);
            }
        }

        transaction.commit().await.map_err(Error::Client)?;
        Ok(queries.len())
    }
}

#[async_trait]
impl<C: Connection> AsyncQuery<Vec<Migration>> for MigrationConnection<'_, C> {
    async fn query(
        &mut self,
        query: &str,
    ) -> Result<Vec<Migration>, <Self as AsyncTransaction>::Error> {
        log::debug!(target: "surrealdb_refinery", "query: {query}");

        let mut response = self.0.query(query).await.map_err(Error::Client)?;
        let errors = response.take_errors();
        if !errors.is_empty() {
            return Err(Error::from_statement_errors(errors));
        }

        let rows: Vec<HistoryRow> = response.take(0).map_err(Error::Row)?;
        rows.into_iter().map(Migration::try_from).collect()
    }
}

/// Only the query builders are overridden. The trait's default
/// `get_applied_migrations` and `get_last_applied_migration` then do the right
/// thing, including wrapping errors with the operation that failed.
#[async_trait]
impl<C: Connection> AsyncMigrate for MigrationConnection<'_, C> {
    fn assert_migrations_table_query(migration_table_name: &str) -> String {
        history::assert_table(migration_table_name)
    }

    fn get_applied_migrations_query(migration_table_name: &str) -> String {
        history::applied_migrations(migration_table_name)
    }

    fn get_last_applied_migration_query(migration_table_name: &str) -> String {
        history::last_applied_migration(migration_table_name)
    }
}
