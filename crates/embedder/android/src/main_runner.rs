//! Android main-loop integration. Posts and timers enter the existing ALooper.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use embedder::{PlatformTaskRunner, Task, TaskRunner};
use ndk::looper::{FdEvent, ThreadLooper};

static RUNNER: OnceLock<Arc<PlatformTaskRunner>> = OnceLock::new();
static OWNER: OnceLock<std::thread::ThreadId> = OnceLock::new();

pub(crate) fn main_runner() -> Result<Arc<dyn TaskRunner>, String> {
    if let Some(runner) = RUNNER.get() {
        return Ok(Arc::clone(runner) as Arc<dyn TaskRunner>);
    }
    let looper = ThreadLooper::for_thread().ok_or("Android main Looper is unavailable")?;
    let event = owned_fd({
        // SAFETY: eventfd requires no pointers. The successful fd passes to OwnedFd.
        unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) }
    })?;
    let timer = owned_fd({
        // SAFETY: timerfd_create requires no pointers. OwnedFd owns the result.
        unsafe {
            libc::timerfd_create(
                libc::CLOCK_MONOTONIC,
                libc::TFD_CLOEXEC | libc::TFD_NONBLOCK,
            )
        }
    })?;
    let queue = Arc::new(MainQueue {
        event,
        timer,
        state: Mutex::default(),
    });
    // These registrations live on the process main Looper across View replacement.
    looper
        .add_fd_with_callback(queue.timer.as_fd(), FdEvent::INPUT, {
            let queue = Arc::clone(&queue);
            move |_, _| {
                acknowledge(&queue.timer);
                queue.drain();
                true
            }
        })
        .map_err(|error| error.to_string())?;
    looper
        .add_fd_with_callback(queue.event.as_fd(), FdEvent::INPUT, {
            let queue = Arc::clone(&queue);
            move |_, _| {
                acknowledge(&queue.event);
                queue.drain();
                true
            }
        })
        .map_err(|error| error.to_string())?;

    let _ = OWNER.set(std::thread::current().id());
    let runner = PlatformTaskRunner::new(
        Arc::new({
            let queue = Arc::clone(&queue);
            move |task| queue.post(task)
        }),
        move |delay, task| queue.delay(delay, task),
        || OWNER.get() == Some(&std::thread::current().id()),
    );
    let _ = RUNNER.set(Arc::clone(&runner));
    Ok(runner)
}

#[derive(Default)]
struct Pending {
    immediate: VecDeque<Task>,
    timers: BTreeMap<(Instant, u64), Task>,
    next: u64,
}

struct MainQueue {
    event: OwnedFd,
    timer: OwnedFd,
    state: Mutex<Pending>,
}

impl MainQueue {
    fn post(&self, task: Task) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .immediate
            .push_back(task);
        self.signal();
    }

    fn delay(&self, delay: Duration, task: Task) {
        let now = Instant::now();
        let Some(deadline) = now.checked_add(delay) else {
            return;
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let id = state.next;
        state.next = id.wrapping_add(1);
        state.timers.insert((deadline, id), task);
        self.arm(&state);
    }

    fn drain(&self) {
        for _ in 0..32 {
            let task = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if state
                    .timers
                    .first_key_value()
                    .is_some_and(|((when, _), _)| *when <= Instant::now())
                {
                    state.timers.pop_first().map(|(_, task)| task)
                } else {
                    state.immediate.pop_front()
                }
            };
            let Some(task) = task else { break };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
        }
        let pending = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !pending.immediate.is_empty() {
            self.signal();
        }
        self.arm(&pending);
    }

    fn arm(&self, pending: &Pending) {
        let after = pending
            .timers
            .first_key_value()
            .map_or(Duration::ZERO, |((when, _), _)| {
                when.saturating_duration_since(Instant::now())
                    .max(Duration::from_nanos(1))
            });
        let time = libc::itimerspec {
            it_interval: libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: libc::timespec {
                tv_sec: after.as_secs().try_into().unwrap_or(libc::time_t::MAX),
                tv_nsec: after.subsec_nanos().into(),
            },
        };
        // SAFETY: the timer fd is live and time is a fully initialized itimerspec.
        if unsafe { libc::timerfd_settime(self.timer.as_raw_fd(), 0, &time, std::ptr::null_mut()) }
            < 0
        {
            tracing::error!(target: "impasto::shell", error = %io::Error::last_os_error(), "cannot arm the Android main loop timer");
        }
    }

    fn signal(&self) {
        let signal = 1_u64;
        loop {
            // SAFETY: eventfd reads exactly eight bytes from this live value.
            if unsafe { libc::write(self.event.as_raw_fd(), (&raw const signal).cast(), 8) } >= 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                if error.kind() != io::ErrorKind::WouldBlock {
                    tracing::error!(target: "impasto::shell", %error, "cannot wake Android main loop");
                }
                break;
            }
        }
    }
}

fn owned_fd(fd: libc::c_int) -> Result<OwnedFd, String> {
    if fd < 0 {
        Err(io::Error::last_os_error().to_string())
    } else {
        // SAFETY: each creation returned a fresh descriptor and only OwnedFd closes it.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

fn acknowledge(fd: &OwnedFd) {
    let mut count = 0_u64;
    // SAFETY: both eventfd and timerfd write exactly eight bytes to this storage.
    unsafe { libc::read(fd.as_raw_fd(), (&raw mut count).cast(), 8) };
}
