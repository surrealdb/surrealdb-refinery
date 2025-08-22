use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::debug;
use refinery_core::Migration;
use refinery_core::error::WrapMigrationError;
use refinery_core::traits::r#async::{AsyncMigrate, AsyncQuery, AsyncTransaction};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::Debug;
use std::fmt::Display;
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use time::OffsetDateTime;

#[allow(dead_code)]
#[derive(Debug)]
enum State {
    Applied,
    Unapplied,
}

impl Serialize for State {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            State::Applied => serializer.serialize_i32(1),
            State::Unapplied => serializer.serialize_i32(0),
        }
    }
}

impl<'de> Deserialize<'de> for State {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        let value = i32::deserialize(deserializer)?;
        match value {
            1 => Ok(State::Applied),
            0 => Ok(State::Unapplied),
            _ => Err(D::Error::custom(format!("Invalid state value: {}", value))),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
struct MigrationInner {
    state: State,
    name: String,
    version: i32,
    #[serde(deserialize_with = "deserialize_checksum")]
    checksum: u64,
    sql: Option<String>,
    applied_on: Option<DateTime<Utc>>,
}

fn deserialize_checksum<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    if let Ok(s) = String::deserialize(deserializer) {
        s.parse::<u64>().map_err(D::Error::custom)
    } else {
        Err(D::Error::custom("invalid type for checksum"))
    }
}

impl From<MigrationInner> for Migration {
    fn from(inner: MigrationInner) -> Self {
        match inner.applied_on {
            Some(applied_on) => {
                let native_applied_on =
                    OffsetDateTime::from_unix_timestamp(applied_on.timestamp()).unwrap();
                Migration::applied(inner.version, inner.name, native_applied_on, inner.checksum)
            }
            None => Migration::unapplied(&inner.name, &inner.sql.unwrap()).unwrap(),
        }
    }
}

pub struct MigrationConnection(pub Surreal<Any>);

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

impl From<SurrealError> for refinery_core::Error {
    fn from(val: SurrealError) -> Self {
        let result: Result<(), SurrealError> = Err(val);
        result
            .migration_err("error getting last applied migration", None)
            .unwrap_err()
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
impl AsyncTransaction for MigrationConnection {
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
impl AsyncQuery<Vec<Migration>> for MigrationConnection {
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
impl AsyncMigrate for MigrationConnection {
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
            DEFINE FIELD IF NOT EXISTS applied_on ON {migration_table_name} TYPE string;
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
    use serde_json;
    use std::collections::HashMap;

    #[test]
    fn test_state_serialization() {
        let applied = State::Applied;
        let unapplied = State::Unapplied;

        let applied_json = serde_json::to_string(&applied).unwrap();
        let unapplied_json = serde_json::to_string(&unapplied).unwrap();

        assert_eq!(applied_json, "1");
        assert_eq!(unapplied_json, "0");
    }

    #[test]
    fn test_state_deserialization() {
        let applied_json = "1";
        let unapplied_json = "0";
        let invalid_json = "2";

        let applied: State = serde_json::from_str(applied_json).unwrap();
        let unapplied: State = serde_json::from_str(unapplied_json).unwrap();

        assert!(matches!(applied, State::Applied));
        assert!(matches!(unapplied, State::Unapplied));

        let invalid_result: Result<State, _> = serde_json::from_str(invalid_json);
        assert!(invalid_result.is_err());
    }

    #[test]
    fn test_deserialize_checksum_valid_string() {
        let json = r#"{"checksum": "12345"}"#;
        #[derive(Deserialize)]
        struct TestStruct {
            #[serde(deserialize_with = "deserialize_checksum")]
            checksum: u64,
        }

        let result: TestStruct = serde_json::from_str(json).unwrap();
        assert_eq!(result.checksum, 12345u64);
    }

    #[test]
    fn test_deserialize_checksum_invalid_string() {
        let json = r#"{"checksum": "not_a_number"}"#;
        #[derive(Deserialize)]
        struct TestStruct {
            #[serde(deserialize_with = "deserialize_checksum")]
            checksum: u64,
        }

        let result: Result<TestStruct, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_surreal_error_from_hashmap() {
        let mut errors = HashMap::new();
        errors.insert(0, surrealdb::Error::Db(surrealdb::error::Db::QueryTimedout));
        errors.insert(1, surrealdb::Error::Db(surrealdb::error::Db::TxFailure));

        let surreal_error: SurrealError = errors.into();
        let error_string = format!("{}", surreal_error);
        assert!(error_string.contains("SurrealDB error"));
    }
}
