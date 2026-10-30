//! Owns bounded task lanes and the shared async runtime for service work.

use std::sync::{
    Arc, Mutex, OnceLock,
    mpsc::{self, SyncSender, TrySendError},
};
use std::thread;
use tokio::sync::oneshot;

type Task = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send + 'static>;

const GENERAL_CAPACITY: usize = 256;
const BACKGROUND_CAPACITY: usize = 128;
const TITLE_CAPACITY: usize = 128;
const LONG_CAPACITY: usize = 64;
const LONG_WORKERS: usize = 3;
const GENERAL_WORKERS: usize = 3;
const BLOCKING_WORKERS: usize = GENERAL_WORKERS + 1;

struct TaskCoordinator {
    general: SyncSender<Task>,
    background: SyncSender<Task>,
    titles: SyncSender<Task>,
    long_running: SyncSender<Task>,
}

impl TaskCoordinator {
    fn global() -> &'static Self {
        static COORDINATOR: OnceLock<TaskCoordinator> = OnceLock::new();
        COORDINATOR.get_or_init(Self::start)
    }

    fn start() -> Self {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(BLOCKING_WORKERS)
                .enable_all()
                .build()
                .expect("failed to start shared service runtime"),
        );
        let (general, general_receiver) = mpsc::sync_channel::<Task>(GENERAL_CAPACITY);
        let general_receiver = Arc::new(Mutex::new(general_receiver));
        for index in 0..GENERAL_WORKERS {
            let receiver = general_receiver.clone();
            let runtime = runtime.clone();
            thread::Builder::new()
                .name(format!("panda-reader-task-{index}"))
                .spawn(move || {
                    loop {
                        let next = match receiver.lock() {
                            Ok(receiver) => receiver.recv(),
                            Err(poisoned) => poisoned.into_inner().recv(),
                        };
                        let Ok(task) = next else { break };
                        task(&runtime);
                    }
                })
                .expect("failed to start background task worker");
        }

        let (background, background_receiver) = mpsc::sync_channel::<Task>(BACKGROUND_CAPACITY);
        let background_runtime = runtime.clone();
        thread::Builder::new()
            .name("panda-reader-sync-lane".into())
            .spawn(move || {
                while let Ok(task) = background_receiver.recv() {
                    task(&background_runtime);
                }
            })
            .expect("failed to start background synchronization lane");

        let (long_running, long_receiver) = mpsc::sync_channel::<Task>(LONG_CAPACITY);
        let long_receiver = Arc::new(Mutex::new(long_receiver));
        for index in 0..LONG_WORKERS {
            let receiver = long_receiver.clone();
            let runtime = runtime.clone();
            thread::Builder::new()
                .name(format!("panda-reader-long-{index}"))
                .spawn(move || {
                    loop {
                        let next = receiver
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .recv();
                        let Ok(task) = next else { break };
                        task(&runtime);
                    }
                })
                .expect("failed to start long-running task worker");
        }
        let (titles, title_receiver) = mpsc::sync_channel::<Task>(TITLE_CAPACITY);
        let title_runtime = runtime;
        thread::Builder::new()
            .name("panda-reader-title-lane".into())
            .spawn(move || {
                while let Ok(task) = title_receiver.recv() {
                    task(&title_runtime);
                }
            })
            .expect("failed to start title task lane");

        Self {
            general,
            background,
            titles,
            long_running,
        }
    }
}

pub(super) fn try_long_job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) -> bool {
    submit(
        TaskCoordinator::global().long_running.clone(),
        reply,
        run,
        "Article",
    )
}

pub(super) fn long_job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) {
    let _ = try_long_job(reply, run);
}

pub(super) fn job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) {
    let _ = try_job(reply, run);
}

pub(super) fn try_job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) -> bool {
    submit(
        TaskCoordinator::global().general.clone(),
        reply,
        run,
        "Background",
    )
}

pub(super) fn background_job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) {
    let _ = try_background_job(reply, run);
}

pub(super) fn try_background_job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) -> bool {
    submit(
        TaskCoordinator::global().background.clone(),
        reply,
        run,
        "Synchronization",
    )
}

/// Title translation uses its own bounded lane so a slow title batch cannot
/// occupy a general worker or hold up article and synchronization work.
pub(super) fn title_job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) {
    submit(
        TaskCoordinator::global().titles.clone(),
        reply,
        run,
        "Title translation",
    );
}

fn submit<T: Send + 'static>(
    sender: SyncSender<Task>,
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
    lane: &'static str,
) -> bool {
    let reply_slot = Arc::new(Mutex::new(Some(reply)));
    let task_reply = reply_slot.clone();
    let task: Task = Box::new(move |runtime| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(runtime)))
            .unwrap_or_else(|panic| {
                Err(format!(
                    "Background operation panicked: {}",
                    panic_text(&*panic)
                ))
            });
        if let Some(reply) = take_reply(&task_reply) {
            let _ = reply.send(result);
        }
    });
    match sender.try_send(task) {
        Ok(()) => true,
        Err(error) => {
            let message = match error {
                TrySendError::Full(_) => format!("{lane} task queue is full; try again shortly"),
                TrySendError::Disconnected(_) => format!("{lane} task lane is unavailable"),
            };
            if let Some(reply) = take_reply(&reply_slot) {
                let _ = reply.send(Err(message));
            }
            false
        }
    }
}

fn take_reply<T>(
    slot: &Mutex<Option<oneshot::Sender<Result<T, String>>>>,
) -> Option<oneshot::Sender<Result<T, String>>> {
    match slot.lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn general_lane_runs_async_work_on_the_shared_runtime() {
        let (reply, result) = oneshot::channel();
        assert!(try_job(reply, |runtime| {
            let value = runtime.block_on(async {
                tokio::task::yield_now().await;
                42
            });
            Ok(value)
        }));
        assert_eq!(result.blocking_recv().unwrap().unwrap(), 42);
    }

    #[test]
    fn a_slow_background_task_does_not_block_reader_work() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (background_reply, _background_result) = oneshot::channel();
        assert!(try_background_job(background_reply, move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        }));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        let (reader_reply, reader_result) = oneshot::channel();
        assert!(try_job(reader_reply, |_| Ok("readable")));
        assert_eq!(reader_result.blocking_recv().unwrap().unwrap(), "readable");
        release_tx.send(()).unwrap();
    }

    #[test]
    fn saturated_article_lane_does_not_block_reader_work() {
        let (started_tx, started_rx) = mpsc::channel();
        let mut releases = Vec::new();
        for _ in 0..LONG_WORKERS {
            let (release_tx, release_rx) = mpsc::channel::<()>();
            releases.push(release_tx);
            let started = started_tx.clone();
            let (reply, _) = oneshot::channel();
            assert!(try_long_job(reply, move |_| {
                started.send(()).unwrap();
                let _ = release_rx.recv();
                Ok(())
            }));
        }
        for _ in 0..LONG_WORKERS {
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        let (completed_tx, completed_rx) = mpsc::channel();
        let (reply, _) = oneshot::channel();
        assert!(try_job(reply, move |_| {
            completed_tx.send(()).unwrap();
            Ok(())
        }));
        let completed = completed_rx.recv_timeout(Duration::from_secs(2));
        drop(releases);
        assert!(completed.is_ok(), "reader task was blocked by article jobs");
    }
}
