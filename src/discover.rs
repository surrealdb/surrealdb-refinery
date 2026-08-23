//! Discovery of SurrealQL migration files.
//!
//! This is the in-crate replacement for refinery's `.sql`-only file discovery,
//! which is why this driver does not need a patched refinery. The version, name
//! and checksum of each migration are still parsed by refinery itself, so the
//! naming rules are exactly those of [`refinery::embed_migrations!`].

use std::path::{Path, PathBuf};

use refinery_core::Migration;

/// File extensions recognised as SurrealQL migrations.
pub const MIGRATION_EXTENSIONS: [&str; 2] = ["surql", "sql"];

/// An error encountered while discovering migration files on disk.
#[derive(Debug)]
#[non_exhaustive]
pub enum DiscoverError {
    /// A directory or file under the migrations directory could not be read.
    Io {
        /// The path being read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A file carried a migration extension but its stem is not a valid
    /// migration name.
    InvalidName {
        /// The offending file.
        path: PathBuf,
        /// The parse error reported by refinery.
        source: refinery_core::Error,
    },
}

impl std::fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, .. } => write!(f, "failed to read {}", path.display()),
            Self::InvalidName { path, .. } => write!(
                f,
                "{} is not a valid migration name; expected V{{version}}__{{name}}.surql",
                path.display()
            ),
        }
    }
}

impl std::error::Error for DiscoverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidName { source, .. } => Some(source),
        }
    }
}

/// Load every SurrealQL migration under `dir`, recursing into subdirectories.
///
/// The returned migrations are ordered by version, ready for
/// [`refinery::Runner::new`].
///
/// Unlike refinery's own loader, a file that carries a migration extension but
/// whose stem does not parse is an **error**, not a warning: a silently skipped
/// migration is how a database ends up half-migrated.
pub fn load_migrations(dir: impl AsRef<Path>) -> Result<Vec<Migration>, DiscoverError> {
    let mut paths = Vec::new();
    collect(dir.as_ref(), &mut paths)?;
    paths.sort();

    let mut migrations = Vec::with_capacity(paths.len());
    for path in paths {
        let sql = std::fs::read_to_string(&path).map_err(|source| DiscoverError::Io {
            path: path.clone(),
            source,
        })?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let migration = Migration::unapplied(stem, &sql)
            .map_err(|source| DiscoverError::InvalidName { path, source })?;
        migrations.push(migration);
    }

    migrations.sort();
    Ok(migrations)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), DiscoverError> {
    let io_err = |source| DiscoverError::Io {
        path: dir.to_path_buf(),
        source,
    };
    for entry in std::fs::read_dir(dir).map_err(io_err)? {
        let path = entry.map_err(io_err)?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| MIGRATION_EXTENSIONS.contains(&e))
        {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_surql_migrations_in_version_order() {
        let migrations = load_migrations("tests/migrations").unwrap();
        assert_eq!(migrations.len(), 1);
        assert_eq!(migrations[0].version(), 1);
        assert_eq!(migrations[0].name(), "first");
    }

    #[test]
    fn missing_directory_is_an_error() {
        let err = load_migrations("tests/does-not-exist").unwrap_err();
        assert!(matches!(err, DiscoverError::Io { .. }));
    }
}
