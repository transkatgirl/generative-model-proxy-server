use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};
use sled::{
    transaction::{ConflictableTransactionError, TransactionError, Transactional},
    Batch, Db, Mode,
};

use super::Database;

impl Database {
    pub fn open(path: &Path) -> Result<Self, sled::Error> {
        let current_database_location = path.join(PathBuf::from("version-1"));
        let past_database_location = path.join(PathBuf::from("version-0"));

        if past_database_location.exists() && !current_database_location.exists() {
            todo!()
        }

        Ok(Database {
            database: sled::Config::default()
                .path(current_database_location)
                .mode(Mode::HighThroughput)
                .open()?,
        })
    }
}
