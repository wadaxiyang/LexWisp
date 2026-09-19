use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
};

use lexwisp_core::{FavoriteFuture, FavoriteUiPort, InvocationId};
use lexwisp_storage::ContentStore;

pub struct FavoriteService {
    storage: ContentStore,
    known: Arc<RwLock<HashSet<InvocationId>>>,
}

impl FavoriteService {
    pub fn new(storage: ContentStore) -> Self {
        Self {
            storage,
            known: Arc::new(RwLock::new(HashSet::new())),
        }
    }
}

impl FavoriteUiPort for FavoriteService {
    fn toggle(&self, invocation_id: InvocationId) -> FavoriteFuture<'_> {
        let storage = self.storage.clone();
        let known = self.known.clone();
        Box::pin(async move {
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
