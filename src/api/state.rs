use axum::http::StatusCode;

use serde::{de::DeserializeOwned, Serialize};
use sled::{
    transaction::{ConflictableTransactionError, TransactionError, Transactional},
    Batch,
};

use super::AppState;

// TODO: Review https://serde.rs/lifetimes.html and fix deserializer lifetimes (if applicable)

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

impl AppState {
    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) fn get_table<V>(&self, table: &str) -> Result<Vec<V>, StatusCode>
    where
        V: DeserializeOwned,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => Ok(tree
                .iter()
                .filter_map(|item| {
                    item.ok()
                        .and_then(|(_, value)| postcard::from_bytes(&value).ok())
                })
                .collect()),
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", table, error);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn get_item<K, V>(&self, table: &str, key: &K) -> Result<V, StatusCode>
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
                        Some(value) => Ok(Ok(postcard::from_bytes(&value)
                            .map_err(ConflictableTransactionError::Abort)?)),
                        None => Ok(Err(StatusCode::NOT_FOUND)),
                    }
                })
                .unwrap_or_else(|error| {
                    tracing::warn!("Unable to apply database transaction: {}", error);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                }),
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", table, error);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    #[tracing::instrument(skip(self, keys), level = "debug")]
    pub(super) fn get_items_skip_missing<K, V>(
        &self,
        table: &str,
        keys: &[K],
    ) -> Result<Vec<V>, StatusCode>
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

                    Ok(Ok(values))
                })
                .unwrap_or_else(|error| {
                    tracing::warn!("Unable to apply database transaction: {}", error);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                }),
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", table, error);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn get_related_item<K, V, W>(
        &self,
        tables: (&str, &str),
        key: &K,
    ) -> Result<W, StatusCode>
    where
        K: Serialize,
        V: DeserializeOwned + RelatedToItem,
        W: DeserializeOwned,
    {
        let table_main = match self.database.open_tree(tables.0.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", tables.0, error);
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

        let table_related = match self.database.open_tree(tables.1.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", tables.1, error);
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
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
                        return Ok(Ok(postcard::from_bytes(&value)
                            .map_err(ConflictableTransactionError::Abort)?));
                    }
                }

                Ok(Err(StatusCode::NOT_FOUND))
            })
            .unwrap_or_else(|error| {
                tracing::warn!("Unable to apply database transaction: {}", error);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            })
    }

    #[tracing::instrument(skip(self, key, value), level = "debug")]
    pub(super) fn insert_item<K, V>(&self, table: &str, key: &K, value: &V) -> StatusCode
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

                    Ok(StatusCode::OK)
                })
                .unwrap_or_else(|error| {
                    tracing::warn!("Unable to apply database transaction: {}", error);
                    StatusCode::INTERNAL_SERVER_ERROR
                }),
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", table, error);
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    #[tracing::instrument(skip(self, keys, filter_mapper), level = "debug")]
    pub(super) fn modify_items_skip_missing<K, V, F, T, E>(
        &self,
        table: &str,
        keys: &[K],
        filter_mapper: F,
    ) -> Result<Result<Vec<T>, E>, StatusCode>
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

                    Ok(Ok(Ok(outputs)))
                })
                .unwrap_or_else(|error| match error {
                    TransactionError::Abort(Either::A(error)) => {
                        tracing::warn!("Unable to apply database transaction: {}", error);
                        Err(StatusCode::INTERNAL_SERVER_ERROR)
                    }
                    TransactionError::Abort(Either::B(error)) => Ok(Err(error)),
                    TransactionError::Storage(error) => {
                        tracing::warn!("Unable to apply database transaction: {}", error);
                        Err(StatusCode::INTERNAL_SERVER_ERROR)
                    }
                }),
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", table, error);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    #[tracing::instrument(skip(self, main_item, related_items), level = "debug")]
    pub(super) fn insert_related_items<K, L, V, W>(
        &self,
        tables: (&str, &str),
        main_item: (&K, &V),
        related_items: &[(L, W)],
    ) -> StatusCode
    where
        K: Serialize,
        L: Serialize,
        V: Serialize + DeserializeOwned + RelatedToItemSet,
        W: Serialize,
    {
        let table_main = match self.database.open_tree(tables.0.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", tables.0, error);
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        };

        let table_related = match self.database.open_tree(tables.1.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", tables.1, error);
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        };

        (&table_main, &table_related)
            .transaction(|(table_main, table_related)| {
                let mut batch = Batch::default();
                if let Some(payload) = table_main.insert(
                    postcard::to_stdvec(main_item.0)
                        .map_err(ConflictableTransactionError::Abort)?,
                    postcard::to_stdvec(main_item.1)
                        .map_err(ConflictableTransactionError::Abort)?,
                )? {
                    let deserialized: V = postcard::from_bytes(&payload)
                        .map_err(ConflictableTransactionError::Abort)?;

                    for linked_key in deserialized.get_keys(tables.1) {
                        batch.remove(
                            postcard::to_stdvec(&linked_key)
                                .map_err(ConflictableTransactionError::Abort)?,
                        )
                    }
                }

                for (key, value) in related_items {
                    batch.insert(
                        postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                        postcard::to_stdvec(value).map_err(ConflictableTransactionError::Abort)?,
                    )
                }

                table_related.apply_batch(&batch)?;

                Ok(StatusCode::OK)
            })
            .unwrap_or_else(|error| {
                tracing::warn!("Unable to apply database transaction: {}", error);
                StatusCode::INTERNAL_SERVER_ERROR
            })
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn remove_item<K>(&self, table: &str, key: &K) -> StatusCode
    where
        K: Serialize,
    {
        match self.database.open_tree(table.as_bytes()) {
            Ok(tree) => tree
                .transaction(|tree| {
                    tree.remove(
                        postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                    )?;

                    Ok(StatusCode::OK)
                })
                .unwrap_or_else(|error| {
                    tracing::warn!("Unable to apply database transaction: {}", error);
                    StatusCode::INTERNAL_SERVER_ERROR
                }),
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", table, error);
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    pub(super) fn remove_related_items<K, V>(&self, tables: (&str, &str), key: &K) -> StatusCode
    where
        K: Serialize,
        V: Serialize + DeserializeOwned + RelatedToItemSet,
    {
        let table_main = match self.database.open_tree(tables.0.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", tables.0, error);
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        };

        let table_related = match self.database.open_tree(tables.1.as_bytes()) {
            Ok(tree) => tree,
            Err(error) => {
                tracing::warn!("Unable to open \"{}\" table: {}", tables.1, error);
                return StatusCode::INTERNAL_SERVER_ERROR;
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

                        Ok(StatusCode::OK)
                    }
                    None => Ok(StatusCode::NOT_FOUND),
                }
            })
            .unwrap_or_else(|error| {
                tracing::warn!("Unable to apply database transaction: {}", error);
                StatusCode::INTERNAL_SERVER_ERROR
            })
    }
}
