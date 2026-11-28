use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc::{self, Sender},
};
use std::thread;

use rusqlite::Connection;

use crate::SqlError;

type Work = Box<dyn FnOnce(&mut Connection) + Send>;
struct ConnectionWorker {
    sender: Sender<Work>,
    thread: thread::JoinHandle<()>,
    queued: Arc<AtomicUsize>,
}
struct State {
    writer: Option<ConnectionWorker>,
    readers: Vec<ConnectionWorker>,
    lease: Option<u64>,
    next: u64,
}

pub(crate) struct Worker {
    state: Mutex<State>,
    cursor: AtomicUsize,
}

fn start(
    connection: Connection,
    path_lease: Option<Arc<()>>,
) -> Result<ConnectionWorker, SqlError> {
    let (sender, receiver) = mpsc::channel::<Work>();
    let thread = thread::Builder::new()
        .name("impasto-sqlite".into())
        .spawn(move || {
            let _path_lease = path_lease;
            let mut connection = connection;
            while let Ok(work) = receiver.recv() {
                work(&mut connection);
            }
        })
        .map_err(|_| SqlError::io())?;
    Ok(ConnectionWorker {
        sender,
        thread,
        queued: Arc::default(),
    })
}

impl Worker {
    pub(crate) fn new(
        writer: Connection,
        readers: Vec<Connection>,
        path_lease: Option<Arc<()>>,
    ) -> Result<Arc<Self>, SqlError> {
        let readers = readers
            .into_iter()
            .map(|connection| start(connection, path_lease.clone()))
            .collect::<Result<_, _>>()?;
        let writer = start(writer, path_lease)?;
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                writer: Some(writer),
                readers,
                lease: None,
                next: 0,
            }),
            cursor: AtomicUsize::new(0),
        }))
    }
    pub(crate) fn read<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<R, SqlError> + Send + 'static,
    ) -> Result<R, SqlError> {
        let state = self.state.lock().map_err(|_| SqlError::closed())?;
        if state.writer.is_none() {
            return Err(SqlError::closed());
        }
        if state.readers.is_empty() {
            drop(state);
            return self.write(None, operation);
        }
        let index = self.cursor.fetch_add(1, Ordering::Relaxed) % state.readers.len();
        let reply = state.readers[index].submit(operation, false)?;
        drop(state);
        reply.recv().map_err(|_| SqlError::closed())?
    }
    pub(crate) fn write<R: Send + 'static>(
        &self,
        lease: Option<u64>,
        operation: impl FnOnce(&mut Connection) -> Result<R, SqlError> + Send + 'static,
    ) -> Result<R, SqlError> {
        let state = self.state.lock().map_err(|_| SqlError::closed())?;
        if state.lease != lease {
            return Err(if lease.is_none() {
                SqlError::busy()
            } else {
                SqlError::closed()
            });
        }
        let reply = state
            .writer
            .as_ref()
            .ok_or_else(SqlError::closed)?
            .submit(operation, false)?;
        drop(state);
        reply.recv().map_err(|_| SqlError::closed())?
    }
    pub(crate) fn begin(&self) -> Result<u64, SqlError> {
        let mut state = self.state.lock().map_err(|_| SqlError::closed())?;
        if state.lease.is_some() {
            return Err(SqlError::busy());
        }
        let writer = state.writer.as_ref().ok_or_else(SqlError::closed)?;
        let next = state.next.checked_add(1).ok_or_else(SqlError::closed)?;
        let reply = writer.submit(|conn| Ok(conn.execute_batch("BEGIN IMMEDIATE")?), false)?;
        state.next = next;
        state.lease = Some(next);
        drop(state);
        let result = reply
            .recv()
            .map_err(|_| SqlError::closed())
            .and_then(|result| result);
        let mut state = self.state.lock().map_err(|_| SqlError::closed())?;
        if state.lease != Some(next) {
            return Err(SqlError::closed());
        }
        if let Err(error) = result {
            state.lease = None;
            return Err(error);
        }
        Ok(next)
    }
    pub(crate) fn end(&self, lease: u64, commit: bool) -> Result<(), SqlError> {
        let mut state = self.state.lock().map_err(|_| SqlError::closed())?;
        if state.lease != Some(lease) {
            return Err(SqlError::closed());
        }
        let reply = state.writer.as_ref().ok_or_else(SqlError::closed)?.submit(
            move |conn| {
                if commit && let Err(error) = conn.execute_batch("COMMIT") {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(error.into());
                }
                if !commit {
                    conn.execute_batch("ROLLBACK")?;
                }
                Ok(())
            },
            true,
        );
        state.lease = None;
        drop(state);
        reply?.recv().map_err(|_| SqlError::closed())?
    }
    pub(crate) fn drop_lease(&self, lease: u64) {
        if let Ok(mut state) = self.state.lock()
            && state.lease == Some(lease)
        {
            if let Some(writer) = &state.writer {
                let _ = writer.sender.send(Box::new(|connection| {
                    let _ = connection.execute_batch("ROLLBACK");
                }));
            }
            state.lease = None;
        }
    }
    pub(crate) fn close(&self) -> Result<(), SqlError> {
        let mut state = self.state.lock().map_err(|_| SqlError::closed())?;
        let writer = state.writer.take();
        let readers = std::mem::take(&mut state.readers);
        state.lease = None;
        drop(state);
        for worker in writer.into_iter().chain(readers) {
            // SQLite closes each connection on its owning worker, rolling back any open transaction.
            drop(worker.sender);
            worker.thread.join().map_err(|_| SqlError::closed())?;
        }
        Ok(())
    }
}

struct QueueCredit(Arc<AtomicUsize>);

impl Drop for QueueCredit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl ConnectionWorker {
    fn submit<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<R, SqlError> + Send + 'static,
        control: bool,
    ) -> Result<mpsc::Receiver<Result<R, SqlError>>, SqlError> {
        let credit = if control {
            None
        } else {
            self.queued
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |length| {
                    (length < 32).then_some(length + 1)
                })
                .map_err(|_| SqlError::busy())?;
            Some(QueueCredit(Arc::clone(&self.queued)))
        };
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(Box::new(move |connection| {
                let _credit = credit;
                let _ = reply.send(operation(connection));
            }))
            .map_err(|_| SqlError::closed())?;
        Ok(receive)
    }
}
