//! A bounded printer session: selected profile + driver + transport link.
//!
//! The session executes a driver's [`ExchangePlan`] generically: chunking to the smaller
//! of the profile ceiling and the transport limit, pacing, flow-control pauses reported
//! by the driver's decoder, fault checks between chunks, cancellation and deadlines.
//! Callers must call [`PrinterSession::close`] in every outcome.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nuprint_protocol::{
    DeviceInfo, ExchangePlan, NotificationDecoder, PlanStep, ProtocolEvent, StatusFlag, StatusReport,
};
use nuprint_transport::{Advertisement, Endpoint, Link, Notification, ServiceInfo, Transport};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::api::{JobState, LogLevel};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::profile::{BoundEndpoints, ResolvedProfile};

pub type LogFn = Arc<dyn Fn(LogLevel, String) + Send + Sync>;

const MAX_RAW_NOTIFICATIONS: usize = 64;

#[derive(Debug, Clone)]
struct LiveState {
    status: Option<StatusReport>,
    status_seq: u64,
    writable: bool,
    device_info: Option<DeviceInfo>,
    stream_open: bool,
}

/// Session progress reported to the job service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    Phase(JobState),
    Progress { sent: usize, total: usize },
}

async fn cancellable<T>(cancel: &CancellationToken, fut: impl Future<Output = T>) -> AppResult<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AppError::cancelled()),
        value = fut => Ok(value),
    }
}

async fn sleep(cancel: &CancellationToken, duration: Duration) -> AppResult<()> {
    if !duration.is_zero() {
        cancellable(cancel, tokio::time::sleep(duration)).await?;
    }
    Ok(())
}

/// Scan until a device satisfying `matches` advertises, or `timeout` elapses. The scan is
/// always allowed to finish its own cleanup (the BLE backend stops scanning) before returning.
pub async fn locate(
    transport: &dyn Transport,
    timeout: Duration,
    cancel: &CancellationToken,
    matches: impl Fn(&Advertisement) -> bool,
) -> AppResult<Advertisement> {
    let (tx, mut rx) = mpsc::channel(64);
    let scan_cancel = cancel.child_token();
    let scan = transport.scan(timeout, tx, scan_cancel.clone());
    tokio::pin!(scan);
    let mut found = None;
    let mut scan_result = None;
    loop {
        tokio::select! {
            result = &mut scan, if scan_result.is_none() => scan_result = Some(result),
            adv = rx.recv() => match adv {
                Some(adv) => {
                    if found.is_none() && matches(&adv) {
                        found = Some(adv);
                        scan_cancel.cancel();
                    }
                }
                // The sender lives inside the scan future, so `None` means the scan finished.
                None => break,
            },
        }
    }
    if let Some(adv) = found {
        return Ok(adv);
    }
    if cancel.is_cancelled() {
        return Err(AppError::cancelled());
    }
    if let Some(Err(error)) = scan_result {
        return Err(error.into());
    }
    Err(AppError::new(
        ErrorCode::DeviceNotFound,
        "Printer was not found. Turn it on, keep it near the computer and close the phone app.",
    ))
}

pub struct PrinterSession {
    link: Box<dyn Link>,
    resolved: ResolvedProfile,
    bound: BoundEndpoints,
    state: watch::Receiver<LiveState>,
    pump: JoinHandle<()>,
    chunk: usize,
    raw: Arc<Mutex<Vec<String>>>,
    log: LogFn,
    print_bytes_submitted: usize,
    /// Length of a print write that was started but has not returned success.
    print_bytes_in_flight: usize,
}

impl PrinterSession {
    /// Connect, bind the profile's endpoints and subscribe to status notifications.
    pub async fn open(
        transport: &dyn Transport,
        resolved: &ResolvedProfile,
        backend_id: &str,
        log: LogFn,
        cancel: &CancellationToken,
    ) -> AppResult<Self> {
        let t = &resolved.profile.transport.timeouts;
        let link = cancellable(cancel, transport.connect(backend_id, Duration::from_millis(t.connect_ms))).await??;
        match Self::bind(link, resolved, log.clone(), cancel).await {
            Ok(session) => Ok(session),
            Err((link, error)) => {
                let _ = tokio::time::timeout(Duration::from_millis(t.disconnect_ms), link.disconnect()).await;
                Err(error)
            }
        }
    }

    async fn bind(
        mut link: Box<dyn Link>,
        resolved: &ResolvedProfile,
        log: LogFn,
        cancel: &CancellationToken,
    ) -> Result<Self, (Box<dyn Link>, AppError)> {
        let profile = &resolved.profile;
        let t = &profile.transport;
        let bound = match profile.bind(link.services()) {
            Ok(bound) => bound,
            Err(why) => {
                let error = AppError::new(
                    ErrorCode::ServiceMismatch,
                    format!("This device does not offer the {} print service.", profile.display_name),
                )
                .with_detail(why);
                return Err((link, error));
            }
        };
        let notify = profile.notify_endpoints(link.services());
        if notify.len() < t.min_notify_subscriptions.max(1) {
            let error =
                AppError::new(ErrorCode::ServiceMismatch, "Printer has no supported status notification channel");
            return Err((link, error));
        }
        for endpoint in &notify {
            let subscribed = cancellable(
                cancel,
                tokio::time::timeout(Duration::from_millis(t.timeouts.subscribe_ms), link.subscribe(*endpoint)),
            )
            .await;
            let error = match subscribed {
                Ok(Ok(Ok(()))) => continue,
                Ok(Ok(Err(e))) => AppError::from(e),
                Ok(Err(_)) => AppError::new(ErrorCode::ConnectionFailed, "Subscribing to printer status timed out"),
                Err(e) => e,
            };
            return Err((link, error));
        }
        let Some(notifications) = link.take_notifications() else {
            return Err((link, AppError::internal("notification stream already taken")));
        };
        let limit = link.max_write_len(bound.write_mode);
        let chunk = t.max_chunk_bytes.min(limit);
        if chunk == 0 {
            return Err((
                link,
                AppError::new(ErrorCode::ConnectionFailed, "Printer reported an invalid Bluetooth write size"),
            ));
        }
        let (state_tx, state) = watch::channel(LiveState {
            status: None,
            status_seq: 0,
            writable: true,
            device_info: None,
            stream_open: true,
        });
        let raw = Arc::new(Mutex::new(Vec::new()));
        let decoders: HashMap<Endpoint, Box<dyn NotificationDecoder>> =
            notify.iter().map(|e| (*e, resolved.driver.new_decoder())).collect();
        let pump = tokio::spawn(pump(notifications, decoders, state_tx, raw.clone(), log.clone()));
        log(
            LogLevel::Info,
            format!(
                "Connected to {} · write {} · chunk {chunk} bytes (link limit {limit})",
                profile.display_name, bound.write
            ),
        );
        Ok(Self {
            link,
            resolved: resolved.clone(),
            bound,
            state,
            pump,
            chunk,
            raw,
            log,
            print_bytes_submitted: 0,
            print_bytes_in_flight: 0,
        })
    }

    pub fn services(&self) -> &[ServiceInfo] {
        self.link.services()
    }

    pub fn max_write_len(&self) -> usize {
        self.link.max_write_len(self.bound.write_mode)
    }

    pub fn raw_notifications(&self) -> Vec<String> {
        self.raw.lock().unwrap().clone()
    }

    pub fn device_info(&self) -> Option<DeviceInfo> {
        self.state.borrow().device_info.clone()
    }

    pub fn last_status(&self) -> Option<StatusReport> {
        self.state.borrow().status.clone()
    }

    /// Bytes of print data written so far (excludes status queries). Non-zero means a
    /// failure may have left partial output.
    pub fn print_bytes_submitted(&self) -> usize {
        self.print_bytes_submitted
    }

    /// Print bytes that may have reached the printer: the confirmed writes plus a write that
    /// failed, timed out or was cancelled midway. A failed write does not prove nothing arrived,
    /// so this is what decides whether a failure is uncertain.
    pub fn print_bytes_at_risk(&self) -> usize {
        self.print_bytes_submitted + self.print_bytes_in_flight
    }

    async fn check_faults(&self) -> AppResult<()> {
        if !self.state.borrow().stream_open || !self.link.is_connected().await {
            return Err(AppError::from(nuprint_transport::TransportError::Disconnected));
        }
        if let Some(status) = &self.state.borrow().status
            && let Some(flag) = status.blocking_fault()
        {
            return Err(AppError::from_fault(flag));
        }
        Ok(())
    }

    async fn wait_writable(&mut self, cancel: &CancellationToken) -> AppResult<()> {
        let timeout = Duration::from_millis(self.resolved.profile.transport.timeouts.flow_pause_ms);
        let mut state = self.state.clone();
        let wait = tokio::time::timeout(timeout, state.wait_for(|s| s.writable || !s.stream_open));
        match cancellable(cancel, wait).await? {
            Ok(_) => Ok(()),
            Err(_) => Err(AppError::new(
                ErrorCode::BufferTimeout,
                format!("Printer buffer stayed full for {} seconds; transfer stopped.", timeout.as_secs()),
            )),
        }
    }

    /// Write `data` in paced chunks. `print_data` enables fault gating and byte accounting.
    async fn send(
        &mut self,
        data: &[u8],
        print_data: bool,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(SessionEvent) + Send),
    ) -> AppResult<()> {
        let pacing = self.resolved.profile.transport.pacing.clone();
        let write_timeout = Duration::from_millis(self.resolved.profile.transport.timeouts.write_ms);
        let mut offset = 0;
        while offset < data.len() {
            if cancel.is_cancelled() {
                return Err(AppError::cancelled());
            }
            if print_data {
                self.check_faults().await?;
            }
            self.wait_writable(cancel).await?;
            if print_data {
                self.check_faults().await?;
            }
            let end = (offset + self.chunk).min(data.len());
            if print_data {
                self.print_bytes_in_flight = end - offset;
            }
            let write = self.link.write(self.bound.write, &data[offset..end], self.bound.write_mode);
            match cancellable(cancel, tokio::time::timeout(write_timeout, write)).await? {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => return Err(AppError::new(ErrorCode::ConnectionFailed, "Bluetooth write timed out")),
            }
            if print_data {
                self.print_bytes_in_flight = 0;
                self.print_bytes_submitted += end - offset;
                on_event(SessionEvent::Progress { sent: end, total: data.len() });
            }
            // Conservative pacing; never stream at full BLE speed.
            sleep(cancel, Duration::from_millis(pacing.chunk_delay_ms)).await?;
            if pacing.boundary_bytes > 0 && offset / pacing.boundary_bytes != end / pacing.boundary_bytes {
                sleep(cancel, Duration::from_millis(pacing.boundary_delay_ms)).await?;
            }
            offset = end;
        }
        Ok(())
    }

    /// Send the profile's status query and require a fresh, valid reply.
    pub async fn query_status(&mut self, timeout: Duration, cancel: &CancellationToken) -> AppResult<StatusReport> {
        let Some(query) = self.resolved.driver.status_query() else {
            return Err(AppError::new(ErrorCode::StatusTimeout, "This printer model does not report status"));
        };
        let seq = self.state.borrow().status_seq;
        self.send(&query, false, cancel, &mut |_| {}).await?;
        let mut state = self.state.clone();
        let fresh = tokio::time::timeout(timeout, state.wait_for(|s| s.status_seq > seq || !s.stream_open));
        let report = match cancellable(cancel, fresh).await? {
            Ok(Ok(s)) if s.status_seq > seq => s.status.clone(),
            _ => None,
        };
        report.ok_or_else(|| {
            AppError::new(
                ErrorCode::StatusTimeout,
                "Connected, but no valid printer status response arrived. Switch the printer off/on and close other printer apps.",
            )
        })
    }

    /// Execute a driver plan. On error, [`Self::print_bytes_submitted`] tells the caller
    /// whether output may be partial.
    pub async fn execute(
        &mut self,
        plan: &ExchangePlan,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(SessionEvent) + Send),
    ) -> AppResult<()> {
        for step in &plan.steps {
            match step {
                PlanStep::RequireReady { timeout } => {
                    let report = self.query_status(*timeout, cancel).await?;
                    if let Some(flag) = report.blocking_fault() {
                        return Err(AppError::from_fault(flag));
                    }
                    if report.is_busy() {
                        return Err(AppError::from_fault(StatusFlag::Printing));
                    }
                }
                PlanStep::SendPrintData { bytes } => {
                    on_event(SessionEvent::Phase(JobState::Sending));
                    (self.log)(LogLevel::Info, format!("Sending {} bytes of print data", bytes.len()));
                    self.send(bytes, true, cancel, on_event).await?;
                }
                PlanStep::Settle(duration) => {
                    on_event(SessionEvent::Phase(JobState::WaitingForReady));
                    sleep(cancel, *duration).await?;
                }
                PlanStep::PollUntilReady { interval, deadline, query_timeout } => {
                    on_event(SessionEvent::Phase(JobState::WaitingForReady));
                    let until = tokio::time::Instant::now() + *deadline;
                    loop {
                        let report = self.query_status(*query_timeout, cancel).await?;
                        if let Some(flag) = report.blocking_fault() {
                            return Err(AppError::from_fault(flag));
                        }
                        if report.is_ready() && self.state.borrow().writable {
                            break;
                        }
                        if tokio::time::Instant::now() >= until {
                            return Err(AppError::new(
                                ErrorCode::OutcomeUncertain,
                                "Data was sent, but the printer did not become ready. Check the paper before retrying.",
                            ));
                        }
                        sleep(cancel, *interval).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Read-only diagnostics: driver queries (best effort) plus a required fresh status.
    pub async fn diagnose(&mut self, cancel: &CancellationToken) -> AppResult<StatusReport> {
        for query in self.resolved.driver.diagnostic_queries() {
            if let Err(error) = self.send(&query.bytes, false, cancel, &mut |_| {}).await
                && query.required
            {
                return Err(error);
            }
        }
        let timeout = self.resolved.driver.status_timeout();
        let status = self.query_status(timeout, cancel).await?;
        // Device info is best-effort; give a late reply a moment to arrive.
        sleep(cancel, Duration::from_millis(500)).await?;
        Ok(status)
    }

    /// Disconnect, bounded by the profile's cleanup deadline. Always call this.
    pub async fn close(self) {
        self.pump.abort();
        let timeout = Duration::from_millis(self.resolved.profile.transport.timeouts.disconnect_ms);
        match tokio::time::timeout(timeout, self.link.disconnect()).await {
            Ok(Ok(())) => (self.log)(LogLevel::Info, "Disconnected".into()),
            Ok(Err(e)) => (self.log)(LogLevel::Warn, format!("Bluetooth disconnect cleanup: {e}")),
            Err(_) => (self.log)(LogLevel::Warn, "Bluetooth disconnect cleanup timed out".into()),
        }
    }
}

async fn pump(
    mut notifications: mpsc::Receiver<Notification>,
    mut decoders: HashMap<Endpoint, Box<dyn NotificationDecoder>>,
    state: watch::Sender<LiveState>,
    raw: Arc<Mutex<Vec<String>>>,
    log: LogFn,
) {
    while let Some(notification) = notifications.recv().await {
        let hex = hex::encode(&notification.data);
        {
            let mut raw = raw.lock().unwrap();
            if raw.len() >= MAX_RAW_NOTIFICATIONS {
                raw.remove(0);
            }
            raw.push(format!("{}: {hex}", notification.endpoint.characteristic));
        }
        log(LogLevel::Debug, format!("RX {}: {hex}", &notification.endpoint.characteristic.to_string()[4..8]));
        let Some(decoder) = decoders.get_mut(&notification.endpoint) else { continue };
        for event in decoder.feed(&notification.data) {
            match event {
                ProtocolEvent::FlowPause => {
                    log(LogLevel::Info, "Printer buffer full; pausing transfer".into());
                    state.send_modify(|s| s.writable = false);
                }
                ProtocolEvent::FlowResume => state.send_modify(|s| s.writable = true),
                ProtocolEvent::Status(report) => {
                    log(LogLevel::Info, format!("Printer: {}", report.text()));
                    state.send_modify(|s| {
                        s.status = Some(report);
                        s.status_seq += 1;
                    });
                }
                ProtocolEvent::DeviceInfo(info) => {
                    if let Some(fw) = &info.firmware {
                        log(LogLevel::Info, format!("Firmware {fw}"));
                    }
                    state.send_modify(|s| s.device_info = Some(info));
                }
                ProtocolEvent::Unhandled { opcode, .. } => {
                    log(LogLevel::Debug, format!("Unhandled reply opcode {opcode:02X}"));
                }
            }
        }
    }
    state.send_modify(|s| s.stream_open = false);
}
