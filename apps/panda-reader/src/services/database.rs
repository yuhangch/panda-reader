//! Serialized SQLite mutation queue.

use panda_store::Store;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TryRecvError};
use std::thread;
use std::time::Duration;

type WriteJob = Box<dyn FnOnce(&mut Store) + Send + 'static>;

#[derive(Clone)]
pub(super) struct DbWriter {
    sender: SyncSender<WriteJob>,
    low_priority_sender: SyncSender<WriteJob>,
}

impl DbWriter {
    pub fn start(path: PathBuf) -> anyhow::Result<Self> {
        Store::migrate(&path)?;
        let (sender, receiver) = mpsc::sync_channel::<WriteJob>(256);
        let (low_priority_sender, low_priority_receiver) = mpsc::sync_channel::<WriteJob>(16);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("panda-reader-db-writer".into())
            .spawn(move || {
                let mut store = match Store::open_writer(&path, "local") {
                    Ok(store) => {
                        let _ = ready_sender.send(Ok(()));
                        store
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(format!("{error:#}")));
                        return;
                    }
                };
                loop {
                    match receiver.recv_timeout(Duration::from_millis(20)) {
                        Ok(job) => job(&mut store),
                        Err(RecvTimeoutError::Timeout) => match low_priority_receiver.try_recv() {
                            Ok(job) => job(&mut store),
                            Err(TryRecvError::Empty) => {}
                            Err(TryRecvError::Disconnected) => break,
                        },
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;
        ready_receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("database writer stopped during startup"))?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            sender,
            low_priority_sender,
        })
    }

    pub fn write<T: Send + 'static>(
        &self,
        workspace: String,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let (reply, response) = mpsc::sync_channel(1);
        let job: WriteJob = Box::new(move |store| {
            store.set_workspace(&workspace);
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| operation(store)))
                .map_err(|panic| {
                    let message = panic
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown panic");
                    anyhow::anyhow!("database write operation panicked: {message}")
                })
                .and_then(|result| result);
            let _ = reply.send(result.map_err(|error| format!("{error:#}")));
        });
        self.sender
            .send(job)
            .map_err(|_| "database writer is unavailable".to_owned())?;
        response
            .recv()
            .map_err(|_| "database writer dropped the operation result".to_owned())?
    }

    pub fn enqueue(
        &self,
        workspace: String,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<()> + Send + 'static,
    ) -> Result<(), String> {
        let job: WriteJob = Box::new(move |store| {
            store.set_workspace(&workspace);
            match std::panic::catch_unwind(AssertUnwindSafe(|| operation(store))) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("queued database write failed: {error:#}"),
                Err(_) => eprintln!("queued database write panicked"),
            }
        });
        self.sender
            .send(job)
            .map_err(|_| "database writer is unavailable".to_owned())
    }

    pub fn write_low_priority<T: Send + 'static>(
        &self,
        workspace: String,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let (reply, response) = mpsc::sync_channel(1);
        let job: WriteJob = Box::new(move |store| {
            store.set_workspace(&workspace);
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| operation(store)))
                .map_err(|_| anyhow::anyhow!("low-priority database operation panicked"))
                .and_then(|result| result);
            let _ = reply.send(result.map_err(|error| format!("{error:#}")));
        });
        self.low_priority_sender
            .send(job)
            .map_err(|_| "database writer is unavailable".to_owned())?;
        response
            .recv()
            .map_err(|_| "database writer dropped the operation result".to_owned())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn serializes_concurrent_mutations_on_its_single_connection() {
        let directory = tempfile::tempdir().unwrap();
        let writer = DbWriter::start(directory.path().join("library.sqlite3")).unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let writer = writer.clone();
            let active = active.clone();
            let peak = peak.clone();
            tasks.push(std::thread::spawn(move || {
                writer
                    .write("local".into(), move |_| {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(current, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(5));
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .unwrap();
            }));
        }
        for task in tasks {
            task.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }
}
