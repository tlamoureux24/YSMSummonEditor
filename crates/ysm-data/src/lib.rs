//! Data access for the YSM Summon Editor.
//!
//! Everything the editor reads or writes goes through the
//! [`SummonRepository`] trait, implemented for SQL Server and MariaDB ([`SqlRepository`]),
//! PostgreSQL ([`PgRepository`]) and an in-memory data set ([`InMemoryRepository`]) used by the demo
//! mode and the tests.

pub mod assets;
pub mod env_local;
pub mod error;
pub mod loader;
pub mod memory;
pub mod mssql;
pub mod pg;
pub mod repository;
pub mod settings;

pub use assets::AssetIndex;
pub use error::{DataError, Result};
pub use memory::InMemoryRepository;
pub use mssql::{MssqlRepository, SqlRepository};
pub use pg::PgRepository;
pub use repository::SummonRepository;
pub use settings::{AppSettings, DatabaseEngine, DbSettings, SettingsStore};
