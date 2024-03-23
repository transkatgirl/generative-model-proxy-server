use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};
use sled::{
    transaction::{ConflictableTransactionError, TransactionError, Transactional},
    Batch, Db, Mode,
};
// sled should probably be replaced with a proper database at some point. will need to write manual migrations when that time comes.

pub(super) trait RelatedToItem {
    type Key: Serialize;

    fn get_key(&self, table: &str) -> Self::Key;
}

pub(super) trait RelatedToItemSet {
    type Key: Serialize;

    fn get_keys(&self, table: &str) -> Vec<Self::Key>;
}

enum Either<A, B> {
    A(A),
    B(B),
}

#[derive(Clone)]
pub struct Database {
    database: Db,
}

pub(super) enum DatabaseActionResult {
    Success,
    NotFound,
    BackendError,
}

pub(super) enum DatabaseLinkedInsertionResult {
    Success,
    Duplicate,
    BackendError,
}

pub(super) enum DatabaseValueResult<T> {
    Success(T),
    NotFound,
    BackendError,
}

pub(super) enum DatabaseFunctionResult<T, E> {
    Success(T),
    FunctionError(E),
    BackendError,
}

// TODO: Implement a system for handling database migrations
//const PAST_DATABASE_STRING: &str = "version-0";
//const FUTURE_DATABASE_STRING: &str = "version-2"; // ? How will we handle version rollbacks?
const CURRENT_DATABASE_STRING: &str = "version-1";

impl Database {
    pub fn open(path: &Path) -> Result<Self, sled::Error> {
        let database_location = path.join(PathBuf::from(CURRENT_DATABASE_STRING));

        Ok(Database {
            database: sled::Config::default()
                .path(database_location)
                .mode(Mode::HighThroughput)
                .open()?,
        })
    }

    pub async fn close(self) -> Result<(), sled::Error> {
        self.database.flush_async().await?;

        Ok(())
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub fn is_table_empty(&self, table: &str) -> bool {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => tree.is_empty(),
            Err(_) => false,
        }
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) fn get_table<V>(&self, table: &str) -> DatabaseValueResult<Vec<V>>
    where
        V: DeserializeOwned,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => DatabaseValueResult::Success(
                tree.iter()
                    .filter_map(|item| {
                        item.ok()
                            .and_then(|(_, value)| postcard::from_bytes(&value).ok())
                    })
                    .collect(),
            ),
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", table, error);
                DatabaseValueResult::BackendError
            }
        }
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn get_item<K, V>(&self, table: &str, key: &K) -> DatabaseValueResult<V>
    where
        K: Serialize,
        V: DeserializeOwned,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => tree
                .transaction(|tree| {
                    match tree.get(
                        postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                    )? {
                        Some(value) => Ok(DatabaseValueResult::Success(
                            postcard::from_bytes(&value)
                                .map_err(ConflictableTransactionError::Abort)?,
                        )),
                        None => Ok(DatabaseValueResult::NotFound),
                    }
                })
                .unwrap_or_else(|error| {
                    tracing::error!("Unable to apply database transaction: {}", error);
                    DatabaseValueResult::BackendError
                }),
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", table, error);
                DatabaseValueResult::BackendError
            }
        }
    }

    #[tracing::instrument(skip(self, keys), level = "debug")]
    pub(super) fn get_items_skip_missing<K, V>(
        &self,
        table: &str,
        keys: &[K],
    ) -> DatabaseValueResult<Vec<V>>
    where
        K: Serialize,
        V: DeserializeOwned,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => tree
                .transaction(move |tree| {
                    let mut values = Vec::with_capacity(keys.len());

                    for key in keys {
                        if let Some(value) = tree.get(
                            postcard::to_stdvec(&key)
                                .map_err(ConflictableTransactionError::Abort)?,
                        )? {
                            values.push(
                                postcard::from_bytes(&value)
                                    .map_err(ConflictableTransactionError::Abort)?,
                            );
                        }
                    }

                    Ok(DatabaseValueResult::Success(values))
                })
                .unwrap_or_else(|error| {
                    tracing::error!("Unable to apply database transaction: {}", error);
                    DatabaseValueResult::BackendError
                }),
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", table, error);
                DatabaseValueResult::BackendError
            }
        }
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn get_related_item<K, V, W>(
        &self,
        tables: (&str, &str),
        key: &K,
    ) -> DatabaseValueResult<W>
    where
        K: Serialize,
        V: DeserializeOwned + RelatedToItem,
        W: DeserializeOwned,
    {
        let table_main = match self.database.open_tree(tables.0.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", tables.0, error);
                return DatabaseValueResult::BackendError;
            }
        };

        let table_related = match self.database.open_tree(tables.1.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", tables.1, error);
                return DatabaseValueResult::BackendError;
            }
        };

        (&table_main, &table_related)
            .transaction(|(table_main, table_related)| {
                if let Some(value) = table_main
                    .get(postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?)?
                {
                    let deserialized: V = postcard::from_bytes(&value)
                        .map_err(ConflictableTransactionError::Abort)?;

                    if let Some(value) = table_related.get(
                        postcard::to_stdvec(&deserialized.get_key(tables.1))
                            .map_err(ConflictableTransactionError::Abort)?,
                    )? {
                        return Ok(DatabaseValueResult::Success(
                            postcard::from_bytes(&value)
                                .map_err(ConflictableTransactionError::Abort)?,
                        ));
                    }
                }

                Ok(DatabaseValueResult::NotFound)
            })
            .unwrap_or_else(|error| {
                tracing::error!("Unable to apply database transaction: {}", error);
                DatabaseValueResult::BackendError
            })
    }

    #[tracing::instrument(skip(self, key, value), level = "debug")]
    pub(super) fn insert_item<K, V>(&self, table: &str, key: &K, value: &V) -> DatabaseActionResult
    where
        K: Serialize,
        V: Serialize,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => tree
                .transaction(|tree| {
                    tree.insert(
                        postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                        postcard::to_stdvec(value).map_err(ConflictableTransactionError::Abort)?,
                    )?;

                    Ok(DatabaseActionResult::Success)
                })
                .unwrap_or_else(|error| {
                    tracing::error!("Unable to apply database transaction: {}", error);
                    DatabaseActionResult::BackendError
                }),
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", table, error);
                DatabaseActionResult::BackendError
            }
        }
    }

    #[tracing::instrument(skip(self, keys, filter_mapper), level = "debug")]
    pub(super) fn modify_items_skip_missing<K, V, F, T, E>(
        &self,
        table: &str,
        keys: &[K],
        filter_mapper: F,
    ) -> DatabaseFunctionResult<Vec<T>, E>
    where
        K: Serialize,
        V: Serialize + DeserializeOwned,
        F: Fn(&mut V) -> Result<T, E>,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => tree
                .transaction(|tree| {
                    let mut outputs = Vec::with_capacity(keys.len());

                    for key in keys {
                        let key = postcard::to_stdvec(key)
                            .map_err(Either::A)
                            .map_err(ConflictableTransactionError::Abort)?;

                        if let Some(value) = tree.get(&key)? {
                            let mut value = postcard::from_bytes(&value)
                                .map_err(Either::A)
                                .map_err(ConflictableTransactionError::Abort)?;

                            outputs.push(
                                filter_mapper(&mut value)
                                    .map_err(Either::B)
                                    .map_err(ConflictableTransactionError::Abort)?,
                            );

                            tree.insert(
                                key,
                                postcard::to_stdvec(&value)
                                    .map_err(Either::A)
                                    .map_err(ConflictableTransactionError::Abort)?,
                            )?;
                        }
                    }

                    Ok(DatabaseFunctionResult::Success(outputs))
                })
                .unwrap_or_else(|error| match error {
                    TransactionError::Abort(Either::A(error)) => {
                        tracing::error!("Unable to apply database transaction: {}", error);
                        DatabaseFunctionResult::BackendError
                    }
                    TransactionError::Abort(Either::B(error)) => {
                        DatabaseFunctionResult::FunctionError(error)
                    }
                    TransactionError::Storage(error) => {
                        tracing::error!("Unable to apply database transaction: {}", error);
                        DatabaseFunctionResult::BackendError
                    }
                }),
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", table, error);
                DatabaseFunctionResult::BackendError
            }
        }
    }

    #[tracing::instrument(skip(self, main_item, related_items), level = "debug")]
    pub(super) fn insert_related_items<K, L, V, W>(
        &self,
        tables: (&str, &str),
        main_item: (&K, &V),
        related_items: &[(L, W)],
    ) -> DatabaseLinkedInsertionResult
    where
        K: Serialize,
        L: Serialize,
        V: Serialize + DeserializeOwned + RelatedToItemSet,
        W: Serialize,
    {
        let table_main = match self.database.open_tree(tables.0.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", tables.0, error);
                return DatabaseLinkedInsertionResult::BackendError;
            }
        };

        let table_related = match self.database.open_tree(tables.1.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", tables.1, error);
                return DatabaseLinkedInsertionResult::BackendError;
            }
        };

        (&table_main, &table_related)
            .transaction(|(table_main, table_related)| {
                if let Some(payload) = table_main.insert(
                    postcard::to_stdvec(main_item.0)
                        .map_err(Either::A)
                        .map_err(ConflictableTransactionError::Abort)?,
                    postcard::to_stdvec(main_item.1)
                        .map_err(Either::A)
                        .map_err(ConflictableTransactionError::Abort)?,
                )? {
                    let deserialized: V = postcard::from_bytes(&payload)
                        .map_err(Either::A)
                        .map_err(ConflictableTransactionError::Abort)?;

                    let mut batch = Batch::default();
                    for linked_key in deserialized.get_keys(tables.1) {
                        batch.remove(
                            postcard::to_stdvec(&linked_key)
                                .map_err(Either::A)
                                .map_err(ConflictableTransactionError::Abort)?,
                        )
                    }
                    table_related.apply_batch(&batch)?;
                }

                let mut batch = Batch::default();
                for (key, value) in related_items {
                    let key = postcard::to_stdvec(key)
                        .map_err(Either::A)
                        .map_err(ConflictableTransactionError::Abort)?;

                    if let Ok(Some(_)) = table_related.get(key.clone()) {
                        return Err(ConflictableTransactionError::Abort(Either::B(
                            DatabaseLinkedInsertionResult::Duplicate,
                        )));
                    }

                    batch.insert(
                        key,
                        postcard::to_stdvec(value)
                            .map_err(Either::A)
                            .map_err(ConflictableTransactionError::Abort)?,
                    )
                }

                table_related.apply_batch(&batch)?;

                Ok(DatabaseLinkedInsertionResult::Success)
            })
            .unwrap_or_else(|error| match error {
                TransactionError::Abort(Either::A(error)) => {
                    tracing::error!("Unable to apply database transaction: {}", error);
                    DatabaseLinkedInsertionResult::BackendError
                }
                TransactionError::Abort(Either::B(error)) => error,
                TransactionError::Storage(error) => {
                    tracing::error!("Unable to apply database transaction: {}", error);
                    DatabaseLinkedInsertionResult::BackendError
                }
            })
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn remove_item<K>(&self, table: &str, key: &K) -> DatabaseActionResult
    where
        K: Serialize,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => tree
                .transaction(|tree| {
                    match tree
                        .remove(
                            postcard::to_stdvec(key)
                                .map_err(ConflictableTransactionError::Abort)?,
                        )?
                        .is_some()
                    {
                        true => Ok(DatabaseActionResult::Success),
                        false => Ok(DatabaseActionResult::NotFound),
                    }
                })
                .unwrap_or_else(|error| {
                    tracing::error!("Unable to apply database transaction: {}", error);
                    DatabaseActionResult::BackendError
                }),
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", table, error);
                DatabaseActionResult::BackendError
            }
        }
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn remove_related_items<K, V>(
        &self,
        tables: (&str, &str),
        key: &K,
    ) -> DatabaseActionResult
    where
        K: Serialize,
        V: Serialize + DeserializeOwned + RelatedToItemSet,
    {
        let table_main = match self.database.open_tree(tables.0.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", tables.0, error);
                return DatabaseActionResult::BackendError;
            }
        };

        let table_related = match self.database.open_tree(tables.1.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::error!("Unable to open \"{}\" table: {}", tables.1, error);
                return DatabaseActionResult::BackendError;
            }
        };

        (&table_main, &table_related)
            .transaction(|(table_main, table_related)| {
                match table_main.remove(
                    postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                )? {
                    Some(payload) => {
                        let deserialized: V = postcard::from_bytes(&payload)
                            .map_err(ConflictableTransactionError::Abort)?;

                        let mut batch = Batch::default();
                        for linked_key in deserialized.get_keys(tables.1) {
                            batch.remove(
                                postcard::to_stdvec(&linked_key)
                                    .map_err(ConflictableTransactionError::Abort)?,
                            )
                        }
                        table_related.apply_batch(&batch)?;

                        Ok(DatabaseActionResult::Success)
                    }
                    None => Ok(DatabaseActionResult::NotFound),
                }
            })
            .unwrap_or_else(|error| {
                tracing::error!("Unable to apply database transaction: {}", error);
                DatabaseActionResult::BackendError
            })
    }
}
