//! Background writer for the diagnostic log file.
//!
//! Producers hand finished lines to a bounded in-memory queue and never wait for the disk. One
//! thread owns the file handle, tracks its size, writes batches and rotates. The queue is not a
//! durability mechanism: lines still buffered when the process is killed are lost, and under
//! overload lines are dropped (see [`LogWriter::submit`]). Queue state in the store, not this
//! log, is what keeps jobs safe.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::api::LogLevel;
use crate::service::relock;

/// Lines buffered for the writer thread. Beyond this the overload policy applies.
pub(crate) const QUEUE_LINES: usize = 1024;
/// A single file line is cut to this many bytes so the queue's memory stays bounded.
pub(crate) const MAX_LINE_BYTES: usize = 4096;
/// The file is replaced by a fresh one once it would grow past this.
pub(crate) const ROTATE_BYTES: u64 = 2_000_000;
/// After a write fails, lines are discarded (and counted) for this long before the file is retried.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// Where lines end up. Implemented by the file and, in tests, by controllable fakes.
pub(crate) trait LogSink: Send + 'static {
    /// Buffer or write one finished line (ending in a newline).
    fn write_line(&mut self, line: &str) -> io::Result<()>;
    /// Push buffered lines out.
    fn flush(&mut self) -> io::Result<()>;
}

/// Reports writer health to the in-memory log. Must not call back into the writer.
pub(crate) type Notifier = Arc<dyn Fn(LogLevel, String) + Send + Sync>;

struct Entry {
    /// Warnings and errors outlive information under overload.
    important: bool,
    text: String,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Entry>,
    /// Entries in `queue` that are not important.
    low: usize,
    /// Lines turned away or evicted since the writer last took the count.
    dropped: u64,
    dropped_total: u64,
    /// Lines accepted into the queue / finished with (written or discarded) / evicted again.
    accepted: u64,
    completed: u64,
    evicted: u64,
    closed: bool,
    finished: bool,
    healthy: bool,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    progress: Condvar,
    notifier: Mutex<Option<Notifier>>,
    capacity: usize,
    retry_after: Duration,
}

/// Counters for tests.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WriterStats {
    pub queued: usize,
    pub dropped_total: u64,
    pub healthy: bool,
}

pub(crate) struct LogWriter {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LogWriter {
    /// Write to `path`, rotating to `path.1`.
    pub(crate) fn to_file(path: PathBuf) -> Self {
        Self::with_sink(FileSink::new(path), QUEUE_LINES, RETRY_AFTER)
    }

    pub(crate) fn with_sink(sink: impl LogSink, capacity: usize, retry_after: Duration) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State { healthy: true, ..State::default() }),
            work: Condvar::new(),
            progress: Condvar::new(),
            notifier: Mutex::new(None),
            capacity: capacity.max(1),
            retry_after,
        });
        let worker = shared.clone();
        let thread = thread::Builder::new()
            .name("nuprint-log".into())
            .spawn(move || run(worker, sink))
            .expect("spawn the log writer thread");
        Self { shared, thread: Mutex::new(Some(thread)) }
    }

    pub(crate) fn set_notifier(&self, notifier: Notifier) {
        *relock(&self.shared.notifier) = Some(notifier);
    }

    /// Queue one finished line without waiting for the disk.
    ///
    /// A full queue drops work instead of blocking: an incoming information/debug line is turned
    /// away; an incoming warning or error evicts the oldest information/debug line, or the oldest
    /// line when everything queued is important. Every loss is added to a counter that the writer
    /// reports in the file and in the in-memory log.
    pub(crate) fn submit(&self, level: LogLevel, mut text: String) {
        let important = matches!(level, LogLevel::Warn | LogLevel::Error);
        if text.len() > MAX_LINE_BYTES {
            let mut end = MAX_LINE_BYTES - 4;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("...\n");
        }
        let mut s = lock(&self.shared.state);
        if s.closed {
            s.dropped_total += 1;
            return;
        }
        if s.queue.len() >= self.shared.capacity {
            if !important {
                s.dropped += 1;
                s.dropped_total += 1;
                return;
            }
            let victim = if s.low > 0 { s.queue.iter().position(|e| !e.important) } else { None };
            let victim = match victim {
                Some(index) => s.queue.remove(index),
                None => s.queue.pop_front(),
            };
            if let Some(victim) = victim {
                if !victim.important {
                    s.low -= 1;
                }
                s.dropped += 1;
                s.dropped_total += 1;
                s.evicted += 1;
            }
        }
        if !important {
            s.low += 1;
        }
        s.accepted += 1;
        s.queue.push_back(Entry { important, text });
        drop(s);
        self.shared.work.notify_one();
    }

    /// Wait until every line accepted before this call has been written (or discarded because the
    /// file is unavailable). Returns false on timeout.
    pub(crate) fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut s = lock(&self.shared.state);
        let target = s.accepted;
        while s.completed + s.evicted < target && !s.finished {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { return false };
            s = self.shared.progress.wait_timeout(s, left).unwrap().0;
        }
        true
    }

    /// Stop accepting lines, write what is queued and end the thread. Returns false if the writer
    /// did not finish in time (it is then left to finish on its own).
    pub(crate) fn close(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut s = lock(&self.shared.state);
        s.closed = true;
        self.shared.work.notify_all();
        while !s.finished {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { return false };
            s = self.shared.progress.wait_timeout(s, left).unwrap().0;
        }
        drop(s);
        self.join();
        true
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> WriterStats {
        let s = lock(&self.shared.state);
        WriterStats { queued: s.queue.len(), dropped_total: s.dropped_total, healthy: s.healthy }
    }

    fn join(&self) {
        let thread = relock(&self.thread).take();
        if let Some(thread) = thread
            && thread.thread().id() != thread::current().id()
        {
            let _ = thread.join();
        }
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        {
            let mut s = lock(&self.shared.state);
            s.closed = true;
        }
        self.shared.work.notify_all();
        self.join();
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    // The state is only counters and a queue; a panic elsewhere must not stop logging.
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn notify(shared: &Shared, level: LogLevel, message: String) {
    let notifier = relock(&shared.notifier).clone();
    if let Some(notifier) = notifier {
        notifier(level, message);
    }
}

fn run(shared: Arc<Shared>, mut sink: impl LogSink) {
    let mut unreported: u64 = 0;
    let mut retry_at: Option<Instant> = None;
    loop {
        let (batch, dropped) = {
            let mut s = lock(&shared.state);
            while s.queue.is_empty() && s.dropped == 0 && !s.closed {
                s = shared.work.wait(s).unwrap_or_else(|p| p.into_inner());
            }
            if s.queue.is_empty() && s.dropped == 0 && s.closed {
                s.finished = true;
                drop(s);
                shared.progress.notify_all();
                return;
            }
            s.low = 0;
            (std::mem::take(&mut s.queue), std::mem::take(&mut s.dropped))
        };
        unreported += dropped;
        let count = batch.len() as u64;

        let backing_off = retry_at.is_some_and(|at| Instant::now() < at);
        let result = if backing_off {
            // Do not touch the failing file on every line.
            unreported += count;
            Ok(())
        } else {
            write_batch(&mut sink, &batch, unreported)
        };
        let mut transition = None;
        match &result {
            Ok(()) if !backing_off => {
                unreported = 0;
                if retry_at.take().is_some() {
                    transition = Some((LogLevel::Info, "The log file is writable again.".to_string()));
                }
            }
            Ok(()) => {}
            Err(e) => {
                unreported += count;
                let first = retry_at.is_none();
                retry_at = Some(Instant::now() + shared.retry_after);
                if first {
                    transition = Some((
                        LogLevel::Warn,
                        format!("The log file cannot be written ({e}); file logging is paused and lines are dropped."),
                    ));
                }
            }
        }
        // Report before marking the batch complete so a flush sees the notice.
        if let Some((level, message)) = transition {
            notify(&shared, level, message);
        }
        {
            let mut s = lock(&shared.state);
            s.completed += count;
            s.healthy = retry_at.is_none();
        }
        shared.progress.notify_all();
    }
}

fn write_batch(sink: &mut impl LogSink, batch: &VecDeque<Entry>, unreported: u64) -> io::Result<()> {
    if unreported > 0 {
        let now = crate::store::now_ms();
        sink.write_line(&format!("{now} Warn {unreported} log line(s) were not written to this file\n"))?;
    }
    for entry in batch {
        sink.write_line(&entry.text)?;
    }
    sink.flush()
}

/// The log file: one handle kept open, its size tracked, one backup (`<name>.1`).
pub(crate) struct FileSink {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    buffer: Vec<u8>,
}

impl FileSink {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, file: None, size: 0, buffer: Vec::new() }
    }

    fn open(&mut self) -> io::Result<&mut File> {
        if self.file.is_none() {
            let file = OpenOptions::new().create(true).append(true).open(&self.path)?;
            self.size = file.metadata()?.len();
            self.file = Some(file);
        }
        Ok(self.file.as_mut().expect("opened above"))
    }

    /// Forget the handle and anything buffered after a failure; the next write reopens.
    fn reset(&mut self) {
        self.file = None;
        self.buffer.clear();
    }

    fn drain(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let buffer = std::mem::take(&mut self.buffer);
        let result = self.open().and_then(|file| file.write_all(&buffer));
        match result {
            Ok(()) => {
                self.size += buffer.len() as u64;
                self.buffer = buffer;
                self.buffer.clear();
                Ok(())
            }
            Err(e) => {
                self.reset();
                Err(e)
            }
        }
    }

    /// Close the current file and move it over the previous backup.
    fn rotate(&mut self) -> io::Result<()> {
        self.drain()?;
        // The handle must be closed first: Windows cannot rename an open file.
        self.file = None;
        let backup = backup_path(&self.path);
        if fs::rename(&self.path, &backup).is_err() {
            // An existing backup that cannot be replaced in place (read-only, or a platform that
            // refuses to rename over it) is removed first; if even that fails the current file
            // is truncated so the size stays bounded.
            let moved = fs::remove_file(&backup).and_then(|()| fs::rename(&self.path, &backup));
            if moved.is_err() {
                File::create(&self.path)?;
            }
        }
        self.size = 0;
        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_added_extension("1")
}

impl LogSink for FileSink {
    fn write_line(&mut self, line: &str) -> io::Result<()> {
        if self.file.is_none()
            && let Err(e) = self.open()
        {
            self.reset();
            return Err(e);
        }
        if self.size + (self.buffer.len() + line.len()) as u64 > ROTATE_BYTES
            && (self.size > 0 || !self.buffer.is_empty())
            && let Err(e) = self.rotate()
        {
            self.reset();
            return Err(e);
        }
        self.buffer.extend_from_slice(line.as_bytes());
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;

    /// Records lines; `gate` blocks the first write until released, `fail` makes writes error.
    #[derive(Clone, Default)]
    struct Probe {
        lines: Arc<Mutex<Vec<String>>>,
        fail: Arc<AtomicBool>,
        attempts: Arc<AtomicUsize>,
        entered: Arc<Mutex<Option<mpsc::Sender<()>>>>,
        gate: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
    }

    impl LogSink for Probe {
        fn write_line(&mut self, line: &str) -> io::Result<()> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if let Some(tx) = self.entered.lock().unwrap().take() {
                let _ = tx.send(());
            }
            let gate = self.gate.lock().unwrap().take();
            if let Some(rx) = gate {
                let _ = rx.recv();
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(io::Error::other("disk full"));
            }
            self.lines.lock().unwrap().push(line.to_string());
            Ok(())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn line(text: &str) -> String {
        format!("{text}\n")
    }

    fn written(probe: &Probe) -> Vec<String> {
        probe.lines.lock().unwrap().iter().map(|l| l.trim_end().to_string()).collect()
    }

    #[test]
    fn concurrent_producers_keep_per_producer_order() {
        let probe = Probe::default();
        let writer = Arc::new(LogWriter::with_sink(probe.clone(), 100_000, Duration::from_secs(60)));
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let writer = writer.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    for i in 0..500 {
                        writer.submit(LogLevel::Info, line(&format!("{t}:{i}")));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(writer.flush(Duration::from_secs(30)));
        let lines = written(&probe);
        assert_eq!(lines.len(), 2000);
        for t in 0..4 {
            let mine: Vec<usize> =
                lines.iter().filter_map(|l| l.strip_prefix(&format!("{t}:")).map(|n| n.parse().unwrap())).collect();
            assert_eq!(mine, (0..500).collect::<Vec<_>>(), "producer {t}");
        }
    }

    #[test]
    fn saturation_drops_information_first_and_keeps_errors() {
        let probe = Probe::default();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *probe.entered.lock().unwrap() = Some(entered_tx);
        *probe.gate.lock().unwrap() = Some(release_rx);
        let writer = LogWriter::with_sink(probe.clone(), 4, Duration::from_secs(60));

        // The writer takes this line and blocks inside the sink, so the queue is ours to fill.
        writer.submit(LogLevel::Info, line("first"));
        entered_rx.recv().unwrap();
        for i in 0..4 {
            writer.submit(LogLevel::Info, line(&format!("info{i}")));
        }
        // Full of information: more information is turned away, producers do not block.
        writer.submit(LogLevel::Info, line("info-extra"));
        writer.submit(LogLevel::Debug, line("debug-extra"));
        // Errors evict the oldest information lines.
        writer.submit(LogLevel::Error, line("error0"));
        writer.submit(LogLevel::Warn, line("warn0"));
        assert_eq!(writer.stats().queued, 4);
        // Fill with important lines only: the oldest important line is evicted.
        writer.submit(LogLevel::Error, line("error1"));
        writer.submit(LogLevel::Error, line("error2"));
        writer.submit(LogLevel::Error, line("error3"));
        assert_eq!(writer.stats().dropped_total, 2 + 2 + 3);

        release_tx.send(()).unwrap();
        assert!(writer.flush(Duration::from_secs(30)));
        let lines = written(&probe);
        assert_eq!(lines[0], "first");
        assert!(lines[1].contains("7 log line(s) were not written"), "{lines:?}");
        assert_eq!(lines[2..], ["warn0", "error1", "error2", "error3"]);
    }

    #[test]
    fn write_failure_pauses_without_retrying_and_recovers() {
        let probe = Probe::default();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let writer = LogWriter::with_sink(probe.clone(), 100, Duration::from_secs(3600));
        let sink = notices.clone();
        writer.set_notifier(Arc::new(move |level, message| sink.lock().unwrap().push((level, message))));
        probe.fail.store(true, Ordering::SeqCst);

        writer.submit(LogLevel::Info, line("a"));
        assert!(writer.flush(Duration::from_secs(30)));
        let attempts = probe.attempts.load(Ordering::SeqCst);
        assert!(!writer.stats().healthy);
        // Further lines are discarded without touching the sink.
        for i in 0..10 {
            writer.submit(LogLevel::Error, line(&format!("b{i}")));
            assert!(writer.flush(Duration::from_secs(30)));
        }
        assert_eq!(probe.attempts.load(Ordering::SeqCst), attempts);
        let notices = notices.lock().unwrap();
        assert_eq!(notices.len(), 1, "one notice, not one per line: {notices:?}");
        assert_eq!(notices[0].0, LogLevel::Warn);
    }

    #[test]
    fn writer_recovers_after_the_retry_interval_and_reports_the_loss() {
        let probe = Probe::default();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let writer = LogWriter::with_sink(probe.clone(), 100, Duration::ZERO);
        let sink = notices.clone();
        writer.set_notifier(Arc::new(move |level, message| sink.lock().unwrap().push((level, message))));
        probe.fail.store(true, Ordering::SeqCst);
        writer.submit(LogLevel::Info, line("lost"));
        assert!(writer.flush(Duration::from_secs(30)));
        probe.fail.store(false, Ordering::SeqCst);
        writer.submit(LogLevel::Info, line("kept"));
        assert!(writer.flush(Duration::from_secs(30)));
        let lines = written(&probe);
        assert!(lines[0].contains("1 log line(s) were not written"), "{lines:?}");
        assert_eq!(lines[1], "kept");
        assert!(writer.stats().healthy);
        let levels: Vec<_> = notices.lock().unwrap().iter().map(|n| n.0).collect();
        assert_eq!(levels, [LogLevel::Warn, LogLevel::Info]);
    }

    #[test]
    fn close_writes_pending_lines_and_later_lines_are_counted() {
        let probe = Probe::default();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *probe.entered.lock().unwrap() = Some(entered_tx);
        *probe.gate.lock().unwrap() = Some(release_rx);
        let writer = LogWriter::with_sink(probe.clone(), 100, Duration::from_secs(60));
        writer.submit(LogLevel::Info, line("one"));
        entered_rx.recv().unwrap();
        writer.submit(LogLevel::Info, line("two"));
        writer.submit(LogLevel::Info, line("three"));
        // The writer is stuck: a bounded close reports that instead of hanging.
        assert!(!writer.close(Duration::from_millis(20)));
        release_tx.send(()).unwrap();
        assert!(writer.close(Duration::from_secs(30)));
        assert_eq!(written(&probe), ["one", "two", "three"]);
        writer.submit(LogLevel::Info, line("late"));
        assert_eq!(writer.stats().dropped_total, 1);
    }

    #[test]
    fn long_lines_are_cut_on_a_character_boundary() {
        let probe = Probe::default();
        let writer = LogWriter::with_sink(probe.clone(), 10, Duration::from_secs(60));
        writer.submit(LogLevel::Info, format!("{}\n", "é".repeat(MAX_LINE_BYTES)));
        assert!(writer.flush(Duration::from_secs(30)));
        let lines = probe.lines.lock().unwrap();
        assert!(lines[0].len() <= MAX_LINE_BYTES && lines[0].ends_with("...\n"));
    }

    #[test]
    fn file_sink_rotates_over_an_existing_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nuprint.log");
        let backup = backup_path(&path);
        fs::write(&backup, "old backup\n").unwrap();
        fs::write(&path, vec![b'x'; (ROTATE_BYTES - 10) as usize]).unwrap();
        let mut sink = FileSink::new(path.clone());
        sink.write_line("0123456789 first after reopen\n").unwrap();
        sink.write_line("second\n").unwrap();
        sink.flush().unwrap();
        // The previous current file replaced the old backup; the new file holds only new lines.
        let backed_up = fs::read(&backup).unwrap();
        assert_eq!(backed_up.len() as u64, ROTATE_BYTES - 10);
        assert_eq!(fs::read_to_string(&path).unwrap(), "0123456789 first after reopen\nsecond\n");
        // The handle stays open: more lines append without reopening.
        sink.write_line("third\n").unwrap();
        sink.flush().unwrap();
        assert!(fs::read_to_string(&path).unwrap().ends_with("second\nthird\n"));
    }

    #[test]
    fn file_sink_reports_an_unwritable_path_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be makes opening fail on every platform.
        let path = dir.path().join("nuprint.log");
        fs::create_dir(&path).unwrap();
        let mut sink = FileSink::new(path.clone());
        assert!(sink.write_line("x\n").is_err());
        fs::remove_dir(&path).unwrap();
        sink.write_line("y\n").unwrap();
        sink.flush().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "y\n");
    }
}
