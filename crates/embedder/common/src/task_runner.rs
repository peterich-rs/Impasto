//! Named queues for thread-affine host work and owned cross-thread messages.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

pub type Task = Box<dyn FnOnce() + Send + 'static>;
pub type MainThreadPoster = Arc<dyn Fn(Task) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskQueueId(u64);

impl Default for TaskQueueId {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskQueueId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone)]
pub struct TaskHandle(Arc<AtomicBool>);

impl Default for TaskHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskHandle {
    pub fn new() -> Self {
        Self(Arc::default())
    }

    pub(crate) fn is_pending(&self) -> bool {
        !self.0.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn run(&self, task: Task) {
        if !self.0.swap(true, Ordering::AcqRel) {
            task();
        }
    }
}

pub trait TaskRunner: Send + Sync + 'static {
    /// Enqueues work for a later turn and never invokes it inline.
    /// Returns false when the queue has shut down.
    fn post(&self, task: Task) -> bool;
    fn post_delayed(&self, delay: Duration, task: Task) -> TaskHandle;
    fn runs_on_current_thread(&self) -> bool;
    fn queue_id(&self) -> TaskQueueId;

    fn monotonic_ms(&self) -> f64 {
        crate::timing::mono_ms()
    }
}

#[derive(Clone)]
pub struct TaskRunners {
    pub platform: Arc<dyn TaskRunner>,
    pub ui: Arc<dyn TaskRunner>,
    pub raster: Arc<dyn TaskRunner>,
    pub io: Arc<dyn TaskRunner>,
}

type Delayed = (TaskHandle, Task);

#[derive(Default)]
struct Tasks {
    ready: VecDeque<Task>,
    delayed: BTreeMap<(Duration, u64), Delayed>,
    next: u64,
    closed: bool,
}

impl Tasks {
    fn schedule(&mut self, when: Duration, handle: TaskHandle, task: Task) {
        if !self.closed {
            self.next = self.next.wrapping_add(1);
            self.delayed.insert((when, self.next), (handle, task));
        }
    }

    fn take(&mut self, now: Duration) -> Option<Task> {
        if self
            .delayed
            .first_key_value()
            .is_some_and(|(&(when, _), (handle, _))| !handle.is_pending() || when <= now)
        {
            let (_, (handle, task)) = self.delayed.pop_first()?;
            return Some(Box::new(move || handle.run(task)));
        }
        self.ready.pop_front()
    }
}

#[derive(Default)]
struct ThreadQueue {
    tasks: Mutex<Tasks>,
    changed: Condvar,
}

pub struct ThreadTaskRunner {
    id: TaskQueueId,
    owner: ThreadId,
    epoch: Instant,
    queue: Arc<ThreadQueue>,
}

impl ThreadTaskRunner {
    pub fn start(name: &str) -> std::io::Result<Arc<Self>> {
        let epoch = Instant::now();
        let queue = Arc::new(ThreadQueue::default());
        let pending = Arc::clone(&queue);
        let worker = thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                while let Some(task) = take_thread_task(&pending, epoch) {
                    // A task reports failures through its own reply channel. Keep
                    // the queue alive so it can still retire thread-local resources.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                }
            })?;
        Ok(Arc::new(Self {
            id: TaskQueueId::new(),
            owner: worker.thread().id(),
            epoch,
            queue,
        }))
    }
}

fn take_thread_task(queue: &ThreadQueue, epoch: Instant) -> Option<Task> {
    let mut tasks = queue
        .tasks
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    loop {
        let now = epoch.elapsed();
        if let Some(task) = tasks.take(now) {
            return Some(task);
        }
        if tasks.closed {
            return None;
        }
        tasks = match tasks.delayed.first_key_value() {
            Some((&(when, _), _)) => {
                queue
                    .changed
                    .wait_timeout(tasks, when.saturating_sub(now))
                    .unwrap_or_else(|error| error.into_inner())
                    .0
            }
            None => queue
                .changed
                .wait(tasks)
                .unwrap_or_else(|error| error.into_inner()),
        };
    }
}

impl TaskRunner for ThreadTaskRunner {
    fn post(&self, task: Task) -> bool {
        let mut tasks = self
            .queue
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if tasks.closed {
            return false;
        }
        tasks.ready.push_back(task);
        self.queue.changed.notify_one();
        true
    }

    fn post_delayed(&self, delay: Duration, task: Task) -> TaskHandle {
        let handle = TaskHandle::new();
        self.queue
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .schedule(
                self.epoch.elapsed().saturating_add(delay),
                handle.clone(),
                task,
            );
        self.queue.changed.notify_one();
        handle
    }

    fn runs_on_current_thread(&self) -> bool {
        self.owner == thread::current().id()
    }

    fn queue_id(&self) -> TaskQueueId {
        self.id
    }
}

impl Drop for ThreadTaskRunner {
    fn drop(&mut self) {
        let delayed = {
            let mut tasks = self
                .queue
                .tasks
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            tasks.closed = true;
            std::mem::take(&mut tasks.delayed)
        };
        self.queue.changed.notify_one();
        drop(delayed);
    }
}

/// Adapts a native main loop for immediate work and delayed tasks.
pub struct PlatformTaskRunner {
    id: TaskQueueId,
    poster: MainThreadPoster,
    is_current: fn() -> bool,
    delay: Box<dyn Fn(Duration, Task) + Send + Sync>,
}

impl PlatformTaskRunner {
    pub fn new(
        poster: MainThreadPoster,
        schedule_after: impl Fn(Duration, Task) + Send + Sync + 'static,
        is_current: fn() -> bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: TaskQueueId::new(),
            poster,
            is_current,
            delay: Box::new(schedule_after),
        })
    }
}

impl TaskRunner for PlatformTaskRunner {
    fn post(&self, task: Task) -> bool {
        (self.poster)(task);
        true
    }

    fn post_delayed(&self, delay: Duration, task: Task) -> TaskHandle {
        let handle = TaskHandle::new();
        let timer = handle.clone();
        (self.delay)(delay, Box::new(move || timer.run(task)));
        handle
    }

    fn runs_on_current_thread(&self) -> bool {
        (self.is_current)()
    }

    fn queue_id(&self) -> TaskQueueId {
        self.id
    }
}

pub(crate) struct WorkerTaskRunner {
    id: TaskQueueId,
    rt: tokio::runtime::Handle,
}

thread_local! {
    static WORKER: Cell<Option<TaskQueueId>> = const { Cell::new(None) };
    static SLOTS: RefCell<HashMap<TaskQueueId, Box<dyn Any>>> = RefCell::default();
}

impl WorkerTaskRunner {
    pub(crate) fn new(rt: tokio::runtime::Handle) -> Self {
        Self {
            id: TaskQueueId::new(),
            rt,
        }
    }
}

impl TaskRunner for WorkerTaskRunner {
    fn post(&self, task: Task) -> bool {
        let id = self.id;
        self.rt.spawn_blocking(move || {
            struct Reset;
            impl Drop for Reset {
                fn drop(&mut self) {
                    WORKER.set(None);
                }
            }
            WORKER.set(Some(id));
            let _reset = Reset;
            task();
        });
        true
    }

    fn post_delayed(&self, delay: Duration, task: Task) -> TaskHandle {
        let handle = TaskHandle::new();
        let timer = handle.clone();
        let runner = Self {
            id: self.id,
            rt: self.rt.clone(),
        };
        let deadline = tokio::time::Instant::now() + delay;
        self.rt.spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if !timer.0.load(Ordering::Acquire) {
                runner.post(Box::new(move || timer.run(task)));
            }
        });
        handle
    }

    fn runs_on_current_thread(&self) -> bool {
        WORKER.get() == Some(self.id)
    }

    fn queue_id(&self) -> TaskQueueId {
        self.id
    }
}

/// A deterministic single-thread event loop. Producers may post from any
/// thread, and only the creator can advance its clock or run tasks.
pub struct ManualTaskRunner {
    id: TaskQueueId,
    owner: ThreadId,
    state: Mutex<(Duration, Tasks)>,
}

impl Default for ManualTaskRunner {
    fn default() -> Self {
        Self {
            id: TaskQueueId::new(),
            owner: thread::current().id(),
            state: Mutex::default(),
        }
    }
}

impl ManualTaskRunner {
    pub fn advance(&self, delta: Duration) {
        assert!(self.runs_on_current_thread());
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.0 = state.0.saturating_add(delta);
    }

    pub fn run_next(&self) -> bool {
        assert!(self.runs_on_current_thread());
        let next = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let now = state.0;
            state.1.take(now)
        };
        if let Some(task) = next {
            task();
            true
        } else {
            false
        }
    }
}

impl TaskRunner for ManualTaskRunner {
    fn post(&self, task: Task) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .1
            .ready
            .push_back(task);
        true
    }

    fn post_delayed(&self, delay: Duration, task: Task) -> TaskHandle {
        let handle = TaskHandle::new();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let when = state.0.saturating_add(delay);
        state.1.schedule(when, handle.clone(), task);
        handle
    }

    fn runs_on_current_thread(&self) -> bool {
        self.owner == thread::current().id()
    }

    fn queue_id(&self) -> TaskQueueId {
        self.id
    }

    fn monotonic_ms(&self) -> f64 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .0
            .as_secs_f64()
            * 1000.0
    }
}

/// A serial queue with a stable identity during a one-way executor handoff.
pub struct SwitchableTaskRunner {
    id: TaskQueueId,
    state: Arc<Mutex<SwitchState>>,
}

struct SwitchState {
    target: Option<Arc<dyn TaskRunner>>,
    generation: u64,
    ready: VecDeque<Task>,
    delayed: BTreeMap<u64, (Instant, TaskHandle, Task)>,
    next: u64,
}

impl SwitchableTaskRunner {
    pub fn new(target: Arc<dyn TaskRunner>) -> Self {
        Self {
            id: TaskQueueId::new(),
            state: Arc::new(Mutex::new(SwitchState {
                target: Some(target),
                generation: 0,
                ready: VecDeque::new(),
                delayed: BTreeMap::new(),
                next: 0,
            })),
        }
    }

    /// Pauses dispatch at the current task boundary on the source queue.
    pub(crate) fn begin_migration(&self) -> Option<Arc<dyn TaskRunner>> {
        let mut state = runner_lock(&self.state);
        let target = state.target.as_ref()?;
        assert!(
            target.runs_on_current_thread(),
            "handoff must begin on its source queue"
        );
        state.generation = state.generation.wrapping_add(1);
        state.target.take()
    }

    /// Resumes dispatch on the destination after its thread-local state is installed.
    pub(crate) fn complete_migration(&self, target: Arc<dyn TaskRunner>) {
        assert!(
            target.runs_on_current_thread(),
            "handoff must complete on its destination queue"
        );
        let mut state = runner_lock(&self.state);
        assert!(state.target.is_none(), "handoff is already complete");
        let generation = state.generation;
        let pending = state.ready.len();
        for _ in 0..pending {
            if !self.post_ready(target.as_ref(), generation) {
                state.ready.clear();
                break;
            }
        }
        state.delayed.retain(|id, (when, handle, _)| {
            if !handle.is_pending() {
                return false;
            }
            self.post_timer(target.as_ref(), generation, *id, *when);
            true
        });
        state.target = Some(target);
    }

    fn post_ready(&self, target: &dyn TaskRunner, generation: u64) -> bool {
        let state = Arc::downgrade(&self.state);
        target.post(Box::new(move || {
            let Some(state) = state.upgrade() else { return };
            let task = {
                let mut state = runner_lock(&state);
                if generation != state.generation {
                    return;
                }
                state.ready.pop_front()
            };
            if let Some(task) = task {
                task();
            }
        }))
    }

    fn post_timer(&self, target: &dyn TaskRunner, generation: u64, id: u64, when: Instant) {
        let state = Arc::downgrade(&self.state);
        target.post_delayed(
            when.saturating_duration_since(Instant::now()),
            Box::new(move || {
                let Some(state) = state.upgrade() else { return };
                let delayed = {
                    let mut state = runner_lock(&state);
                    if generation != state.generation {
                        return;
                    }
                    state.delayed.remove(&id)
                };
                if let Some((_, handle, task)) = delayed {
                    handle.run(task);
                }
            }),
        );
    }
}

impl TaskRunner for SwitchableTaskRunner {
    fn post(&self, task: Task) -> bool {
        let mut state = runner_lock(&self.state);
        // TaskRunner::post queues work. The target cannot run this task inline
        // and any waiting callback observes the FIFO after the lock is released.
        if let Some(target) = &state.target
            && !self.post_ready(target.as_ref(), state.generation)
        {
            return false;
        }
        state.ready.push_back(task);
        true
    }

    fn post_delayed(&self, delay: Duration, task: Task) -> TaskHandle {
        let handle = TaskHandle::new();
        let Some(when) = Instant::now().checked_add(delay) else {
            handle.cancel();
            return handle;
        };
        let mut state = runner_lock(&self.state);
        state
            .delayed
            .retain(|_, (_, handle, _)| handle.is_pending());
        let id = state.next;
        state.next = state.next.wrapping_add(1);
        if let Some(target) = &state.target {
            self.post_timer(target.as_ref(), state.generation, id, when);
        }
        state.delayed.insert(id, (when, handle.clone(), task));
        handle
    }

    fn runs_on_current_thread(&self) -> bool {
        runner_lock(&self.state)
            .target
            .as_ref()
            .is_some_and(|target| target.runs_on_current_thread())
    }

    fn queue_id(&self) -> TaskQueueId {
        self.id
    }
}

fn runner_lock<T>(state: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

/// The queue carries only an identifier. The state and all its borrows stay
/// on the owner thread, including construction and destruction.
pub(crate) struct RunnerSlot<T: 'static> {
    runner: Arc<dyn TaskRunner>,
    id: TaskQueueId,
    state: PhantomData<fn() -> T>,
}

impl<T> RunnerSlot<T> {
    pub(crate) fn new(runner: Arc<dyn TaskRunner>) -> Self {
        Self {
            runner,
            id: TaskQueueId::new(),
            state: PhantomData,
        }
    }

    pub(crate) fn runner(&self) -> &Arc<dyn TaskRunner> {
        &self.runner
    }

    pub(crate) fn clear(&self) {
        assert!(self.runner.runs_on_current_thread());
        let value = SLOTS.with_borrow_mut(|slots| slots.remove(&self.id));
        drop(value);
    }

    pub(crate) fn install(&self, value: T) {
        self.install_migrated(self.runner.as_ref(), value);
    }

    /// Removes the uniquely owned value on its source queue.
    pub(crate) fn extract(&self, source: &dyn TaskRunner) -> Option<T> {
        assert!(
            source.runs_on_current_thread(),
            "slot extraction must run on the source queue"
        );
        SLOTS.with_borrow_mut(|slots| {
            let value = slots.get(&self.id)?.downcast_ref::<Rc<RefCell<T>>>()?;
            if Rc::strong_count(value) != 1 {
                return None;
            }
            let value = slots.remove(&self.id)?.downcast::<Rc<RefCell<T>>>().ok()?;
            Rc::try_unwrap(*value).ok().map(RefCell::into_inner)
        })
    }

    /// Installs a value on the destination before logical dispatch resumes.
    pub(crate) fn install_migrated(&self, target: &dyn TaskRunner, value: T) {
        assert!(
            target.runs_on_current_thread(),
            "slot installation must run on the destination queue"
        );
        let previous = SLOTS
            .with_borrow_mut(|slots| slots.insert(self.id, Box::new(Rc::new(RefCell::new(value)))));
        assert!(previous.is_none(), "runner slot was already installed");
    }

    pub(crate) fn with<R>(&self, work: impl FnOnce(&mut T) -> R) -> Option<R> {
        assert!(self.runner.runs_on_current_thread());
        // Clone a thread-local reference and release the map borrow before
        // running work, so independent slots can be accessed inline.
        let state = SLOTS.with_borrow(|slots| {
            slots
                .get(&self.id)?
                .downcast_ref::<Rc<RefCell<T>>>()
                .map(Rc::clone)
        })?;
        let mut state = state.try_borrow_mut().ok()?;
        Some(work(&mut state))
    }
}

impl<T> Drop for RunnerSlot<T> {
    fn drop(&mut self) {
        let id = self.id;
        let remove = move || {
            let value = SLOTS.try_with(|slots| slots.borrow_mut().remove(&id));
            drop(value);
        };
        if self.runner.runs_on_current_thread() {
            remove();
        } else {
            self.runner.post(Box::new(remove));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn manual_queue_orders_work_and_advances_only_on_request() {
        let runner = ManualTaskRunner::default();
        let (send, recv) = mpsc::channel();
        for event in ["first", "second"] {
            let send = send.clone();
            runner.post(Box::new(move || send.send(event).unwrap()));
        }
        let canceled = runner.post_delayed(
            Duration::from_millis(2),
            Box::new(|| {
                panic!("canceled task ran");
            }),
        );
        canceled.cancel();
        let task = runner.post_delayed(
            Duration::from_millis(10),
            Box::new(move || {
                send.send("later").unwrap();
            }),
        );

        while runner.run_next() {}
        assert_eq!(recv.try_iter().collect::<Vec<_>>(), ["first", "second"]);
        runner.advance(Duration::from_millis(9));
        assert!(!runner.run_next());
        runner.advance(Duration::from_millis(1));
        assert!(runner.run_next());
        assert!(!task.is_pending());
        assert_eq!(recv.try_recv().unwrap(), "later");
    }

    #[test]
    fn handoff_preserves_ready_work_and_timer_handles() {
        let source = ThreadTaskRunner::start("test-ui-boot").unwrap();
        let platform = Arc::new(ManualTaskRunner::default());
        let queue = Arc::new(SwitchableTaskRunner::new(source));
        let slot = Arc::new(RunnerSlot::new(Arc::clone(&queue) as Arc<dyn TaskRunner>));
        let id = queue.queue_id();
        let timeout = Duration::from_secs(5);
        let long_delay = Duration::from_secs(30);
        let (entered, boot_started) = mpsc::channel();
        let (proceed, resume) = mpsc::channel();
        let (result, extracted) = mpsc::channel();
        let (installed, log) = mpsc::channel();
        let boot_slot = Arc::clone(&slot);
        let boot_queue = Arc::clone(&queue);
        assert!(queue.post(Box::new(move || {
            boot_slot.install(Vec::new());
            entered.send(()).unwrap();
            resume.recv_timeout(timeout).unwrap();
            let old = boot_queue.begin_migration().unwrap();
            let value = boot_slot.extract(old.as_ref()).unwrap();
            result.send(value).unwrap();
        })));
        boot_started.recv_timeout(timeout).unwrap();

        let push = |n| -> Task {
            let slot = Arc::clone(&slot);
            Box::new(move || slot.with(|values: &mut Vec<_>| values.push(n)).unwrap())
        };
        assert!(queue.post(push(0)));
        let before = queue.post_delayed(long_delay, push(1025));
        let cancelled = queue.post_delayed(long_delay, push(2000));
        proceed.send(()).unwrap();
        let value = extracted.recv_timeout(timeout).unwrap();
        assert_eq!(queue.queue_id(), id);
        assert!(!queue.runs_on_current_thread());
        thread::scope(|scope| {
            scope.spawn(|| {
                for n in 1..=1024 {
                    assert!(queue.post(push(n)));
                }
            });
        });
        let during = queue.post_delayed(long_delay * 2, push(1026));
        cancelled.cancel();
        slot.install_migrated(platform.as_ref(), value);
        queue.complete_migration(Arc::clone(&platform) as Arc<dyn TaskRunner>);
        assert_eq!(queue.queue_id(), id);
        assert!(queue.runs_on_current_thread());

        while platform.run_next() {}
        assert!(before.is_pending() && during.is_pending());
        assert_eq!(slot.with(|values| values.len()), Some(1025));
        platform.advance(long_delay * 2);
        assert!(queue.post(Box::new(move || {
            installed.send(slot.with(std::mem::take).unwrap()).unwrap();
        })));
        while platform.run_next() {}
        assert_eq!(
            log.recv_timeout(timeout).unwrap(),
            (0..=1026).collect::<Vec<_>>()
        );
        assert!(!before.is_pending() && !during.is_pending());
    }

    #[test]
    fn a_thread_affine_slot_is_created_and_dropped_on_the_runner() {
        struct Local {
            _thread_affine: Rc<()>,
            owner: ThreadId,
            done: mpsc::Sender<ThreadId>,
        }

        impl Drop for Local {
            fn drop(&mut self) {
                assert_eq!(thread::current().id(), self.owner);
                self.done.send(self.owner).unwrap();
            }
        }

        let runner = ThreadTaskRunner::start("slot-test").unwrap();
        let slot = Arc::new(RunnerSlot::new(Arc::clone(&runner) as Arc<dyn TaskRunner>));
        let (done, ended) = mpsc::channel();
        let (ready, initialized) = mpsc::channel();
        runner.post(Box::new({
            let slot = Arc::clone(&slot);
            move || {
                let owner = thread::current().id();
                slot.install(Local {
                    _thread_affine: Rc::new(()),
                    owner,
                    done,
                });
                assert_eq!(slot.with(|local| local.owner), Some(owner));
                ready.send(owner).unwrap();
            }
        }));
        let owner = initialized.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(slot);
        assert_eq!(ended.recv_timeout(Duration::from_secs(2)).unwrap(), owner);
    }
}
