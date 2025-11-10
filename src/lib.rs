use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::debug;
use refinery_core::Migration;
use refinery_core::error::WrapMigrationError;
use refinery_core::traits::r#async::{AsyncMigrate, AsyncQuery, AsyncTransaction};

use std::collections::HashMap;
use std::fmt::Debug;
use std::fmt::Display;
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb_types::Number;
use surrealdb_types::Value;
use time::OffsetDateTime;

#[derive(Debug)]
pub enum State {
    Applied,
    Unapplied,
}

impl SurrealValue for State {
    fn kind_of() -> surrealdb_types::Kind {
        surrealdb_types::Kind::Int
    }

    fn into_value(self) -> surrealdb_types::Value {
        match self {
            Self::Applied => Value::Number(Number::Int(1)),
            Self::Unapplied => Value::Number(Number::Int(1)),
        }
    }

    fn from_value(value: surrealdb_types::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        match value {
            Value::Number(Number::Int(1)) => Ok(State::Applied),
            Value::Number(Number::Int(0)) => Ok(State::Unapplied),
            _ => Err(anyhow::anyhow!(
                "invalid value for State enum: expected 0 or 1"
            )),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, SurrealValue)]
struct MigrationInner {
    state: State,
    name: String,
    version: i32,
    checksum: ChecksumType,
    sql: Option<String>,
    applied_on: Option<String>,
}

#[derive(Debug, Eq, PartialEq, Hash)]
struct ChecksumType(u64);

impl SurrealValue for ChecksumType {
    fn kind_of() -> surrealdb_types::Kind {
        surrealdb_types::Kind::String
    }

    fn into_value(self) -> surrealdb_types::Value {
        Value::String(self.0.to_string())
    }

    fn from_value(value: surrealdb_types::Value) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        match value {
            Value::String(s) => s
                .parse::<u64>()
                .map(ChecksumType)
                .map_err(|e| anyhow::anyhow!("failed to parse checksum string: {}", e)),
            _ => Err(anyhow::anyhow!(
                "invalid value for ChecksumType: expected String"
            )),
        }
    }
}

impl From<MigrationInner> for Migration {
    fn from(inner: MigrationInner) -> Self {
        match inner.applied_on {
            Some(applied_on) => {
                let native_applied_on = OffsetDateTime::parse(
                    &applied_on,
                    &time::format_description::well_known::Rfc3339,
                )
                .expect(format!("failed to parse applied_on timestamp: {}", &applied_on).as_str());
                Migration::applied(
                    inner.version,
                    inner.name,
                    native_applied_on,
                    inner.checksum.0,
                )
            }
            None => Migration::unapplied(&inner.name, &inner.sql.unwrap()).unwrap(),
        }
    }
}

pub struct MigrationConnection<'a>(pub &'a Surreal<Any>);

pub struct SurrealError {
    inner: anyhow::Error,
}

impl Display for SurrealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SurrealDB error: {}", self.inner)
    }
}

impl Debug for SurrealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SurrealDB error: {}", self.inner)
    }
}

impl std::error::Error for SurrealError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.inner.source()
    }
}

impl From<surrealdb::Error> for SurrealError {
    fn from(inner: surrealdb::Error) -> Self {
        SurrealError {
            inner: anyhow::anyhow!(inner),
        }
    }
}

impl From<surrealdb_core::rpc::DbResultError> for SurrealError {
    fn from(inner: surrealdb_core::rpc::DbResultError) -> Self {
        SurrealError {
            inner: anyhow::anyhow!(inner.to_string()),
        }
    }
}

impl From<SurrealError> for refinery_core::Error {
    fn from(val: SurrealError) -> Self {
        let result: Result<(), SurrealError> = Err(val);
        result
            .migration_err("error getting last applied migration", None)
            .unwrap_err()
    }
}

impl From<HashMap<usize, surrealdb_core::rpc::DbResultError>> for SurrealError {
    fn from(inner: HashMap<usize, surrealdb_core::rpc::DbResultError>) -> Self {
        let errors = inner
            .into_values()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        SurrealError {
            inner: anyhow::anyhow!(errors),
        }
    }
}

impl From<HashMap<usize, surrealdb::Error>> for SurrealError {
    fn from(inner: HashMap<usize, surrealdb::Error>) -> Self {
        let errors = inner
            .into_values()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        SurrealError {
            inner: anyhow::anyhow!(errors),
        }
    }
}

#[async_trait]
impl AsyncTransaction for MigrationConnection<'_> {
    type Error = SurrealError;

    async fn execute<'a, T: Iterator<Item = &'a str> + Send>(
        &mut self,
        queries: T,
    ) -> Result<usize, Self::Error> {
        let mut count = 0;
        for query in queries {
            if query.is_empty() {
                continue;
            }
            let mut response = self.0.query(query).await?;
            let errors = response.take_errors();
            if !errors.is_empty() {
                let err: SurrealError = errors.into();
                return Err(err);
            }
            count += 1;
        }
        Ok(count)
    }
}

#[async_trait]
impl AsyncQuery<Vec<Migration>> for MigrationConnection<'_> {
    async fn query(
        &mut self,
        query: &str,
    ) -> Result<Vec<Migration>, <Self as AsyncTransaction>::Error> {
        let res = self.0.query(query).await?;
        let mut res = res.check()?;
        let m: Vec<MigrationInner> = res.take(0)?;
        Ok(m.into_iter().map(Migration::from).collect())
    }
}

#[async_trait]
impl AsyncMigrate for MigrationConnection<'_> {
    fn assert_migrations_table_query(migration_table_name: &str) -> String {
        format!(
            "
            BEGIN;
            DEFINE TABLE IF NOT EXISTS {migration_table_name} SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS state ON {migration_table_name} TYPE int DEFAULT 0;
            DEFINE FIELD IF NOT EXISTS name ON {migration_table_name} TYPE string;
            DEFINE FIELD IF NOT EXISTS checksum ON {migration_table_name} TYPE string;
            DEFINE FIELD IF NOT EXISTS version ON {migration_table_name} TYPE int;
            DEFINE FIELD IF NOT EXISTS sql ON {migration_table_name} TYPE option<string>;
            DEFINE FIELD IF NOT EXISTS applied_on ON {migration_table_name} TYPE option<string>;
            COMMIT;"
        )
    }

    fn get_last_applied_migration_query(migration_table_name: &str) -> String {
        format!("SELECT * FROM {migration_table_name} ORDER BY  DESC LIMIT 1;")
    }

    fn get_applied_migrations_query(migration_table_name: &str) -> String {
        format!("SELECT * FROM {migration_table_name} ORDER BY version DESC;")
    }

    async fn get_applied_migrations(
        &mut self,
        migration_table_name: &str,
    ) -> Result<Vec<Migration>, refinery_core::Error> {
        let query = Self::get_applied_migrations_query(migration_table_name);
        debug!("running applied migrations query {}", query);
        let mut response = self
            .0
            .query(query)
            .await
            .migration_err("error getting applied migrations", None)?;
        let errors = response.take_errors();
        if !errors.is_empty() {
            let err: SurrealError = errors.into();
            return Err(refinery_core::Error::from(err));
        }
        debug!("response from applied migrations query {:?}", response);
        let migrations: Vec<MigrationInner> = response.take(0).map_err(|err| {
            let err: SurrealError = err.into();
            refinery_core::Error::from(err)
        })?;

        let migrations = migrations.into_iter().map(|m| m.into()).collect::<Vec<_>>();
        Ok(migrations)
    }

    async fn get_last_applied_migration(
        &mut self,
        migration_table_name: &str,
    ) -> Result<Option<Migration>, refinery_core::Error> {
        let response = self
            .0
            .query(Self::get_last_applied_migration_query(migration_table_name).as_str())
            .await;
        match response {
            Ok(mut response) => {
                let errors = response.take_errors();
                if !errors.is_empty() {
                    let serr: SurrealError = errors.into();
                    return Err(refinery_core::Error::from(serr));
                };
                let mut migrations: Vec<MigrationInner> = response.take(0).map_err(|err| {
                    let serr: SurrealError = err.into();
                    refinery_core::Error::from(serr)
                })?;
                if migrations.is_empty() {
                    return Ok(None);
                }
                let m: Migration = migrations.pop().unwrap().into();
                Ok(Some(m))
            }
            Err(err) => {
                let serr: SurrealError = err.into();
                return Err(serr.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::any::{self, Any};

    #[tokio::test]
    async fn test_migration_connection() {
        let connection = any::connect("mem://").await.unwrap();
        connection.use_ns("test").await.unwrap();
        connection.use_db("test").await.unwrap();
        #[derive(SurrealValue, Debug)]
        struct Example {
            t: Option<DateTime<Utc>>,
        }
        let example = Example {
            t: Some(Utc::now()),
        };
        let _res: Option<Example> = connection.create("example").content(example).await.unwrap();
        let mut res = connection
            .query("SELECT * FROM example ORDER BY version DESC;")
            .await
            .unwrap();
        let errors = res.take_errors();
        if !errors.is_empty() {
            panic!("error getting examples from test db: {:?}", errors);
        }
        let examples: Vec<Example> = res.take(0).unwrap();
        assert_eq!(examples.len(), 1);
    }
}
