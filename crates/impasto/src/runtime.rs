use std::collections::BTreeMap;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};

use futures_channel::oneshot;
use javascript::host::CancelToken;

pub type FutureTask = Pin<Box<dyn Future<Output = ()> + Send>>;
pub type BlockingTask = Box<dyn FnOnce() + Send>;

pub trait RuntimeBackend: Send + Sync + 'static {
    fn spawn(&self, task: FutureTask);
    fn spawn_blocking(&self, task: BlockingTask);
    fn spawn_main(&self, task: BlockingTask);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskError {
    Cancelled,
    Unavailable,
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => out.write_str("task cancelled"),
            Self::Unavailable => out.write_str("task could not complete"),
        }
    }
}

impl std::error::Error for TaskError {}

#[must_use]
pub struct Task<T> {
    result: oneshot::Receiver<T>,
    cancel: CancelToken,
    cancelled: FutureTask,
}

impl<T> Task<T> {
    pub fn abort(&self) {
        self.cancel.cancel();
    }
}

impl<T> Future for Task<T> {
    type Output = Result<T, TaskError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.cancelled.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(TaskError::Cancelled));
        }
        Pin::new(&mut self.result)
            .poll(context)
            .map(|result| result.map_err(|_| TaskError::Unavailable))
    }
}

#[derive(Default)]
struct Scope {
    next: u64,
    tasks: BTreeMap<u64, CancelToken>,
    closed: bool,
}

struct Lease {
    scope: Weak<Mutex<Scope>>,
    id: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(scope) = self.scope.upgrade() {
            scope
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .tasks
                .remove(&self.id);
        }
    }
}

#[derive(Clone, Default)]
pub struct Runtime {
    backend: Option<Arc<dyn RuntimeBackend>>,
    scope: Arc<Mutex<Scope>>,
}

impl Runtime {
    pub fn new(backend: impl RuntimeBackend) -> Self {
        Self {
            backend: Some(Arc::new(backend)),
            scope: Arc::default(),
        }
    }

    pub(crate) fn has_backend(&self) -> bool {
        self.backend.is_some()
    }

    fn task<T>(&self) -> (Task<T>, oneshot::Sender<T>, Option<Lease>) {
        let (sender, result) = oneshot::channel();
        let cancel = CancelToken::new();
        let cancelled = cancel.clone();
        let task = Task {
            result,
            cancel: cancel.clone(),
            cancelled: Box::pin(async move { cancelled.cancelled().await }),
        };
        let mut scope = self.scope.lock().unwrap_or_else(|error| error.into_inner());
        let id = scope.next;
        let lease = if let Some(next) = id.checked_add(1).filter(|_| !scope.closed) {
            scope.next = next;
            scope.tasks.insert(id, cancel);
            Some(Lease {
                scope: Arc::downgrade(&self.scope),
                id,
            })
        } else {
            cancel.cancel();
            None
        };
        (task, sender, lease)
    }

    pub fn spawn<F: Future + Send + 'static>(&self, work: F) -> Task<F::Output>
    where
        F::Output: Send + 'static,
    {
        let (task, sender, lease) = self.task();
        let cancel = task.cancel.clone();
        if let (Some(backend), Some(lease)) = (&self.backend, lease) {
            backend.spawn(Box::pin(async move {
                let _lease = lease;
                if let Some(result) = until_cancelled(work, cancel).await {
                    let _ = sender.send(result);
                }
            }));
        }
        task
    }

    pub fn spawn_blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Task<T> {
        self.spawn_blocking_with_cancel(move |_| work())
    }

    pub fn spawn_blocking_with_cancel<T: Send + 'static>(
        &self,
        work: impl FnOnce(CancelToken) -> T + Send + 'static,
    ) -> Task<T> {
        self.dispatch(work, RuntimeBackend::spawn_blocking)
    }

    pub fn spawn_main<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Task<T> {
        self.dispatch(move |_| work(), RuntimeBackend::spawn_main)
    }

    fn dispatch<T: Send + 'static>(
        &self,
        work: impl FnOnce(CancelToken) -> T + Send + 'static,
        dispatch: fn(&dyn RuntimeBackend, BlockingTask),
    ) -> Task<T> {
        let (task, sender, lease) = self.task();
        let cancel = task.cancel.clone();
        if let (Some(backend), Some(lease)) = (&self.backend, lease) {
            dispatch(
                backend.as_ref(),
                Box::new(move || {
                    let _lease = lease;
                    if !cancel.is_cancelled() {
                        let _ = sender.send(work(cancel));
                    }
                }),
            );
        }
        task
    }

    pub fn close(&self) {
        let tasks = {
            let mut scope = self.scope.lock().unwrap_or_else(|error| error.into_inner());
            scope.closed = true;
            std::mem::take(&mut scope.tasks)
        };
        for task in tasks.into_values() {
            task.cancel();
        }
    }
}

async fn until_cancelled<F: Future>(work: F, cancel: CancelToken) -> Option<F::Output> {
    let mut work = pin!(work);
    let mut cancellation = pin!(cancel.cancelled());
    poll_fn(|context| {
        if cancellation.as_mut().poll(context).is_ready() {
            Poll::Ready(None)
        } else {
            work.as_mut().poll(context).map(Some)
        }
    })
    .await
}

pub async fn scoped<F: Future>(
    work: F,
    request: CancelToken,
    scope: Option<CancelToken>,
) -> Option<F::Output> {
    if let Some(scope) = scope {
        until_cancelled(until_cancelled(work, scope), request)
            .await
            .flatten()
    } else {
        until_cancelled(work, request).await
    }
}
