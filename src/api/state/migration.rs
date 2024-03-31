use std::path::{Path, PathBuf};

use sled::{Config, Mode};

use super::Database;

impl Database {
    pub fn open(path: &Path) -> Result<Self, sled::Error> {
        let current_database_location = path.join(PathBuf::from("version-1"));
        let past_database_location = path.join(PathBuf::from("version-0"));

        if past_database_location.exists() && !current_database_location.exists() {
            todo!()
        }

        Ok(Database {
            database: Config::default()
                .path(current_database_location)
                .mode(Mode::HighThroughput)
                .open()?,
        })
    }
}

/*use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;
use sled::{
    transaction::{ConflictableTransactionError, TransactionError, Transactional},
    Batch, Db, Mode,
};


#[derive(Serialize, Deserialize, Debug, Clone)]
struct OldModelObject {
    #[serde(default)]
    label: String,

    #[serde(default)]
    uuid: Uuid,

    #[serde(default)]
    name: String,

    #[serde(default)]
    types: HashSet<RequestType>,

    api: ModelBackend,

    #[serde(default)]
    quotas: HashSet<Uuid>,
}
*/
