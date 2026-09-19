use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
};

use lexwisp_core::{FavoriteFuture, FavoriteUiPort, InvocationId};
use lexwisp_storage::ContentStore;

use crate::ExecutionStore;

pub struct FavoriteService {
    storage: ContentStore,
    known: Arc<RwLock<HashSet<InvocationId>>>,
    executions: Arc<ExecutionStore>,
}

impl FavoriteService {
    pub fn new(storage: ContentStore, executions: Arc<ExecutionStore>) -> Result<Self, String> {
        let known = storage
            .favorite_ids()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(InvocationId::parse)
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(Self {
            storage,
            known: Arc::new(RwLock::new(known)),
            executions,
        })
    }

    pub fn set_known(&self, invocation_id: InvocationId, favorite: bool) {
        let mut known = self
            .known
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if favorite {
            known.insert(invocation_id);
        } else {
            known.remove(&invocation_id);
        }
    }

    pub fn clear_known(&self) {
        self.known
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

impl FavoriteUiPort for FavoriteService {
    fn toggle(&self, invocation_id: InvocationId) -> FavoriteFuture<'_> {
        let storage = self.storage.clone();
        let known = self.known.clone();
        let executions = self.executions.clone();
        Box::pin(async move {
            let already_favorite = known
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&invocation_id);
            if !already_favorite
                && !storage
                    .contains_favorite(invocation_id.as_str())
                    .map_err(|error| error.to_string())?
            {
                executions.persist_for_favorite(&invocation_id).await?;
            }
            let id = invocation_id.clone();
            let favorite = tokio::task::spawn_blocking(move || {
                storage
                    .toggle_favorite(id.as_str())
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())??;
            let mut known = known
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if favorite {
                known.insert(invocation_id);
            } else {
                known.remove(&invocation_id);
            }
            Ok(favorite)
        })
    }

    fn contains(&self, invocation_id: &InvocationId) -> bool {
        self.known
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(invocation_id)
    }
}
