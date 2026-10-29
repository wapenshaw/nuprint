//! Bounded scheduling for preview rendering.
//!
//! One render runs at a time and at most one more preview waits behind it. A newer request
//! replaces the waiting one, whose caller gets `StalePreview`. The execution permit moves into
//! the blocking closure, so capacity is released only when native work ends, even when the
//! async caller was dropped meanwhile. Only the newest accepted request may publish a result.

use std::sync::{Arc, Mutex};

use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::service::relock;

fn stale() -> AppError {
    AppError::new(ErrorCode::StalePreview, "A newer preview was requested")
}

struct State {
    /// Highest generation accepted so far; lower generations are refused outright.
    latest_generation: f64,
    /// Acceptance order. The request holding `latest_seq` is the only one allowed to publish.
    next_seq: u64,
    latest_seq: u64,
    /// The one request waiting for the execution slot; dropping the sender tells it so.
    pending: Option<(u64, oneshot::Sender<()>)>,
}

pub(crate) struct RenderQueue {
    slot: Arc<Semaphore>,
    state: Mutex<State>,
}

/// An accepted preview request, not yet running.
pub(crate) struct Ticket {
    seq: u64,
    replaced: oneshot::Receiver<()>,
}

/// Lets running work ask whether its request is still the newest.
pub(crate) struct Gate {
    queue: Arc<RenderQueue>,
    seq: u64,
}

impl Gate {
    pub(crate) fn is_current(&self) -> bool {
        self.queue.is_current(self.seq)
    }
}

impl RenderQueue {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            slot: Arc::new(Semaphore::new(1)),
            state: Mutex::new(State { latest_generation: f64::MIN, next_seq: 0, latest_seq: 0, pending: None }),
        })
    }

    /// Accept a request unless a higher generation was already seen. An equal generation is
    /// accepted and, being newer in arrival order, supersedes the earlier one. The previous
    /// waiting request, if any, is released as stale.
    pub(crate) fn admit(&self, generation: f64) -> AppResult<Ticket> {
        let mut state = relock(&self.state);
        if generation < state.latest_generation {
            return Err(stale());
        }
        state.latest_generation = generation;
        state.next_seq += 1;
        let seq = state.next_seq;
        state.latest_seq = seq;
        let (sender, replaced) = oneshot::channel();
        // Dropping the previous sender wakes its waiter with "replaced".
        state.pending = Some((seq, sender));
        Ok(Ticket { seq, replaced })
    }

    fn is_current(&self, seq: u64) -> bool {
        relock(&self.state).latest_seq == seq
    }

    /// Wait for the execution slot, recheck the request and shutdown, then run `work` on a
    /// blocking thread that owns the slot until it returns. `work` returns `None` when it saw
    /// its request go stale. Dropping this future never frees the slot early.
    pub(crate) async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        ticket: &mut Ticket,
        shutdown: &CancellationToken,
        work: impl FnOnce(&Gate) -> AppResult<Option<T>> + Send + 'static,
    ) -> AppResult<T> {
        let seq = ticket.seq;
        let permit = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Err(AppError::cancelled()),
            _ = &mut ticket.replaced => return Err(stale()),
            permit = self.slot.clone().acquire_owned() => permit.expect("the render slot is never closed"),
        };
        {
            let mut state = relock(&self.state);
            if state.pending.as_ref().is_some_and(|(pending, _)| *pending == seq) {
                state.pending = None;
            }
            if state.latest_seq != seq {
                return Err(stale());
            }
        }
        if shutdown.is_cancelled() {
            return Err(AppError::cancelled());
        }
        let gate = Gate { queue: self.clone(), seq };
        let joined = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work(&gate)
        })
        .await
        .map_err(AppError::internal)?;
        joined?.ok_or_else(stale)
    }

    /// Run non-preview work (such as reading a PDF's page count) through the same single slot
    /// so it cannot stack up behind PDFium's own lock. It is never replaced.
    pub(crate) async fn run_exclusive<T: Send + 'static>(
        &self,
        shutdown: &CancellationToken,
        work: impl FnOnce() -> AppResult<T> + Send + 'static,
    ) -> AppResult<T> {
        let permit = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Err(AppError::cancelled()),
            permit = self.slot.clone().acquire_owned() => permit.expect("the render slot is never closed"),
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(AppError::internal)?
    }

    /// Run `publish` only while `ticket` is still the newest accepted request. The check and
    /// the publication happen under one lock, so a newer request cannot be overtaken.
    pub(crate) fn publish<R>(&self, ticket: &Ticket, publish: impl FnOnce() -> R) -> AppResult<R> {
        let state = relock(&self.state);
        if state.latest_seq != ticket.seq {
            return Err(stale());
        }
        let result = publish();
        drop(state);
        Ok(result)
    }

    #[cfg(test)]
    pub(crate) fn free_slots(&self) -> usize {
        self.slot.available_permits()
    }

    #[cfg(test)]
    pub(crate) fn has_pending(&self) -> bool {
        relock(&self.state).pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use super::*;

    /// Work that signals it started, then blocks until released.
    fn held(
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        value: u32,
    ) -> impl FnOnce(&Gate) -> AppResult<Option<u32>> + Send + 'static {
        move |_| {
            started.send(()).unwrap();
            release.recv().unwrap();
            Ok(Some(value))
        }
    }

    async fn settle() {
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn newer_requests_replace_the_pending_one_and_the_latest_runs() {
        let queue = RenderQueue::new();
        let shutdown = CancellationToken::new();
        let runs = Arc::new(AtomicUsize::new(0));

        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut first = queue.admit(1.0).unwrap();
        let q = queue.clone();
        let s = shutdown.clone();
        let first_task = tokio::spawn(async move { q.run(&mut first, &s, held(started_tx, release_rx, 1)).await });
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap()).await.unwrap();

        // Ten more arrive while the first holds the slot; only the last may survive.
        let mut waiters = Vec::new();
        for generation in 2..12u32 {
            let mut ticket = queue.admit(f64::from(generation)).unwrap();
            let q = queue.clone();
            let s = shutdown.clone();
            let runs = runs.clone();
            waiters.push(tokio::spawn(async move {
                q.run(&mut ticket, &s, move |_| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(generation))
                })
                .await
            }));
            settle().await;
            assert_eq!(queue.free_slots(), 0, "one active render at most");
        }
        let (last, replaced) = waiters.split_last_mut().unwrap();
        for waiter in replaced.iter_mut() {
            assert_eq!(waiter.await.unwrap().unwrap_err().code, ErrorCode::StalePreview);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 0, "replaced requests never started");
        assert!(queue.has_pending(), "exactly the newest request is still waiting");

        release_tx.send(()).unwrap();
        // The first finished but is no longer current, so it cannot publish.
        assert_eq!(first_task.await.unwrap().unwrap(), 1);
        assert_eq!(last.await.unwrap().unwrap(), 11);
        assert_eq!(runs.load(Ordering::SeqCst), 1, "only the newest pending request ran");
        assert_eq!(queue.free_slots(), 1);
        assert!(!queue.has_pending());
    }

    #[tokio::test]
    async fn only_the_newest_accepted_request_may_publish() {
        let queue = RenderQueue::new();
        let old = queue.admit(1.0).unwrap();
        let equal = queue.admit(1.0).unwrap();
        assert!(queue.publish(&old, || ()).is_err(), "an older arrival is overtaken, even at an equal generation");
        assert_eq!(queue.publish(&equal, || 7).unwrap(), 7);
        let newer = queue.admit(2.0).unwrap();
        assert_eq!(queue.admit(1.5).err().unwrap().code, ErrorCode::StalePreview);
        assert!(queue.publish(&equal, || ()).is_err());
        assert!(queue.publish(&newer, || ()).is_ok());
    }

    #[tokio::test]
    async fn a_running_render_notices_it_was_superseded() {
        let queue = RenderQueue::new();
        let shutdown = CancellationToken::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let mut ticket = queue.admit(1.0).unwrap();
        let q = queue.clone();
        let s = shutdown.clone();
        let task = tokio::spawn(async move {
            q.run(&mut ticket, &s, move |gate| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(gate.is_current().then_some(()))
            })
            .await
        });
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap()).await.unwrap();
        let _newer = queue.admit(2.0).unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::StalePreview);
        assert_eq!(queue.free_slots(), 1);
    }

    #[tokio::test]
    async fn dropping_the_caller_keeps_the_permit_until_native_work_ends() {
        let queue = RenderQueue::new();
        let shutdown = CancellationToken::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut ticket = queue.admit(1.0).unwrap();
        let q = queue.clone();
        let s = shutdown.clone();
        let task = tokio::spawn(async move { q.run(&mut ticket, &s, held(started_tx, release_rx, 1)).await });
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap()).await.unwrap();

        task.abort();
        let _ = task.await;
        settle().await;
        assert_eq!(queue.free_slots(), 0, "the closure still owns the slot");

        // A later request waits instead of running beside the orphaned native call.
        let mut next = queue.admit(2.0).unwrap();
        let q = queue.clone();
        let s = shutdown.clone();
        let next_task = tokio::spawn(async move { q.run(&mut next, &s, |_| Ok(Some(2u32))).await });
        settle().await;
        assert!(!next_task.is_finished());

        release_tx.send(()).unwrap();
        assert_eq!(next_task.await.unwrap().unwrap(), 2);
        assert_eq!(queue.free_slots(), 1);
    }

    #[tokio::test]
    async fn errors_and_panics_release_the_slot() {
        let queue = RenderQueue::new();
        let shutdown = CancellationToken::new();

        let mut ticket = queue.admit(1.0).unwrap();
        let err = queue
            .run(&mut ticket, &shutdown, |_| -> AppResult<Option<()>> { Err(AppError::invalid("bad")) })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(queue.free_slots(), 1);

        let mut ticket = queue.admit(2.0).unwrap();
        let err = queue
            .run(&mut ticket, &shutdown, |_| -> AppResult<Option<()>> { panic!("native crash") })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
        assert_eq!(queue.free_slots(), 1);

        let mut ticket = queue.admit(3.0).unwrap();
        assert_eq!(queue.run(&mut ticket, &shutdown, |_| Ok(Some(5u8))).await.unwrap(), 5);
    }

    #[tokio::test]
    async fn shutdown_releases_waiters_without_pretending_native_work_stopped() {
        let queue = RenderQueue::new();
        let shutdown = CancellationToken::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut first = queue.admit(1.0).unwrap();
        let q = queue.clone();
        let s = shutdown.clone();
        let running = tokio::spawn(async move { q.run(&mut first, &s, held(started_tx, release_rx, 1)).await });
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap()).await.unwrap();

        let mut waiting = queue.admit(2.0).unwrap();
        let q = queue.clone();
        let s = shutdown.clone();
        let waiter = tokio::spawn(async move { q.run(&mut waiting, &s, |_| Ok(Some(2u32))).await });
        settle().await;
        shutdown.cancel();
        assert_eq!(waiter.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
        assert_eq!(queue.free_slots(), 0, "the in-flight native call still owns the slot");

        let mut late = queue.admit(3.0).unwrap();
        assert_eq!(queue.run(&mut late, &shutdown, |_| Ok(Some(3u32))).await.unwrap_err().code, ErrorCode::Cancelled);

        release_tx.send(()).unwrap();
        // The in-flight native call ran to completion; shutdown did not pretend to stop it.
        assert_eq!(running.await.unwrap().unwrap(), 1);
        assert_eq!(queue.free_slots(), 1);
    }

    #[tokio::test]
    async fn exclusive_work_shares_the_single_slot() {
        let queue = RenderQueue::new();
        let shutdown = CancellationToken::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let q = queue.clone();
        let s = shutdown.clone();
        let first = tokio::spawn(async move {
            q.run_exclusive(&s, move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(1u8)
            })
            .await
        });
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap()).await.unwrap();
        let mut ticket = queue.admit(1.0).unwrap();
        let q = queue.clone();
        let s = shutdown.clone();
        let preview = tokio::spawn(async move { q.run(&mut ticket, &s, |_| Ok(Some(2u8))).await });
        settle().await;
        assert!(!preview.is_finished(), "a preview waits for the exclusive work");
        release_tx.send(()).unwrap();
        assert_eq!(first.await.unwrap().unwrap(), 1);
        assert_eq!(preview.await.unwrap().unwrap(), 2);
    }
}
