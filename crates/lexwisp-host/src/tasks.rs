use std::{
    future::Future,
    sync::{Arc, Mutex, Weak},
};

use lexwisp_core::TaskOwner;
use tokio::{runtime::Handle, task::JoinHandle};
use tokio_util::sync::CancellationToken;

struct ScopeState {
    cancellation: CancellationToken,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}

impl ScopeState {
    fn cancel(&self) {
        self.cancellation.cancel();
        let tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for task in tasks.iter() {
            task.abort();
        }
    }
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
        // Keep the scope alive until its final task finishes. HostTaskPort stores only a Weak
        // reference so completed Invocation scopes do not accumulate for the process lifetime.
        let state = self.state.clone();
        let task = self.runtime.spawn(async move {
            let _scope_lifetime = state;
            future.await
        });
        self.retain(&task);
        task
    }

    pub fn spawn_blocking<F, T>(&self, operation: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let state = self.state.clone();
        let task = self.runtime.spawn_blocking(move || {
            let _scope_lifetime = state;
            operation()
        });
        self.retain(&task);
        task
    }

    pub fn cancel(&self) {
        self.state.cancel();
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
    scopes: Arc<Mutex<Vec<Weak<ScopeState>>>>,
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
        let mut scopes = self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        scopes.retain(|scope| scope.strong_count() > 0);
        scopes.push(Arc::downgrade(&scope.state));
        drop(scopes);
        scope
    }

    pub fn cancel_all(&self) {
        let mut scopes = self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        scopes.retain(|scope| {
            let Some(scope) = scope.upgrade() else {
                return false;
            };
            scope.cancel();
            true
        });
    }

    #[cfg(test)]
    fn live_scope_count(&self) -> usize {
        let mut scopes = self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        scopes.retain(|scope| scope.strong_count() > 0);
        scopes.len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completed_invocation_scopes_are_reclaimed() {
        let tasks = HostTaskPort::new(Handle::current());
        for ix in 0..100 {
            let scope = tasks.scope(TaskOwner::Invocation(format!("stress-{ix}")));
            scope.spawn(async {}).await.expect("task completes");
        }
        tokio::task::yield_now().await;
        assert_eq!(tasks.live_scope_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_scope_stays_cancellable_while_its_task_runs() {
        let tasks = HostTaskPort::new(Handle::current());
        let scope = tasks.scope(TaskOwner::Invocation("cancel-me".into()));
        let cancellation = scope.cancellation();
        let task = scope.spawn({
            let cancellation = cancellation.clone();
            async move {
                cancellation.cancelled().await;
            }
        });
        drop(scope);

        tasks.cancel_all();
        let _ = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("cancelled task stops promptly");
        assert!(cancellation.is_cancelled());
        assert_eq!(tasks.live_scope_count(), 0);
    }
}
