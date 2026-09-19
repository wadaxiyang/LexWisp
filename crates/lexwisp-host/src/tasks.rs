use std::{
    future::Future,
    sync::{Arc, Mutex},
};

use lexwisp_core::TaskOwner;
use tokio::{runtime::Handle, task::JoinHandle};
use tokio_util::sync::CancellationToken;

struct ScopeState {
    cancellation: CancellationToken,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}

#[derive(Clone)]
pub struct TaskScope {
    owner: TaskOwner,
    runtime: Handle,
    state: Arc<ScopeState>,
}

impl TaskScope {
    pub const fn owner(&self) -> &TaskOwner {
        &self.owner
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.state.cancellation.clone()
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let task = self.runtime.spawn(future);
        self.retain(&task);
        task
    }

    pub fn spawn_blocking<F, T>(&self, operation: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let task = self.runtime.spawn_blocking(operation);
        self.retain(&task);
        task
    }

    pub fn cancel(&self) {
        self.state.cancellation.cancel();
        let tasks = self
            .state
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for task in tasks.iter() {
            task.abort();
        }
    }

    fn retain<T>(&self, task: &JoinHandle<T>) {
        let mut tasks = self
            .state
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks.retain(|handle| !handle.is_finished());
        tasks.push(task.abort_handle());
    }
}

#[derive(Clone)]
pub struct HostTaskPort {
    runtime: Handle,
    scopes: Arc<Mutex<Vec<TaskScope>>>,
}

impl HostTaskPort {
    pub fn new(runtime: Handle) -> Self {
        Self {
            runtime,
            scopes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn scope(&self, owner: TaskOwner) -> TaskScope {
        let scope = TaskScope {
            owner,
            runtime: self.runtime.clone(),
            state: Arc::new(ScopeState {
                cancellation: CancellationToken::new(),
                tasks: Mutex::new(Vec::new()),
            }),
        };
        self.scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(scope.clone());
        scope
    }

    pub fn cancel_all(&self) {
        let scopes = self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for scope in scopes.iter() {
            scope.cancel();
        }
    }
}
