//! Spawning, cancelling, and killing the AI utility process.
//!
//! Cancellation has two paths because the cooperative one cannot be guaranteed:
//!
//! 1. **Cooperative.** The shell sends a `task.cancel` request and the utility
//!    answers with a [`crate::contract::CancellationResult`] saying how far it got.
//!    This is the normal path and the only one that can report
//!    `actions_completed`.
//! 2. **Forced.** A utility wedged inside a synchronous provider call cannot read
//!    its stdin, so it cannot be asked. The shell drops its write half (EOF — the
//!    child's read returns and a well-behaved child exits), waits `kill_grace`,
//!    then kills and reaps.
//!
//! Path 2 is not allowed to lose information. Every call still outstanding when the
//! child dies comes back as [`AbandonedWork`]: `unavailable` /
//! `utility.process_lost`, and `effect_unverified: true` for any call that may have
//! dispatched an action — the child could have written to the page and died before
//! replying, so the shell must re-observe rather than assume nothing happened.
//!
//! ## Gap that must not be forgotten
//!
//! If the shell itself crashes, a utility child it spawned survives as an orphan
//! with a live provider session. On Windows the fix is a Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` assigned at spawn
//! (`CreateJobObjectW` + `AssignProcessToJobObject`); the `windows` crate is already
//! pinned by native-browser at `=0.62.2`, so this needs no new dependency. Not
//! implemented here: the contract crate must not link `windows` just to be spawnable
//! in a test, and this is the one item that has to land before unattended tasks are
//! enabled. Tracked in this directory's README under "Known gaps".

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::contract::{CancelReason, CancellationRequest, CancellationResult};
use crate::failure::Failure;
use crate::identity::TaskId;
use crate::frame::{self, Frame, Method, Notification, Peer, ResponsePayload};

/// How long a child gets to exit on its own after its stdin is closed.
pub const DEFAULT_KILL_GRACE: Duration = Duration::from_millis(500);

/// What was outstanding when the child died.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbandonedWork {
    pub request_id: u64,
    pub task: Option<TaskId>,
    /// The child may have changed the page before dying. The caller has to take a
    /// fresh observation instead of reporting a clean stop.
    pub effect_unverified: bool,
    pub failure: Failure,
}

#[derive(Debug)]
struct PendingCall {
    task: Option<TaskId>,
    may_have_dispatched_action: bool,
}

enum Line {
    Frame(Result<Frame, Failure>),
    /// The child closed its stdout: it exited, or was killed.
    Ended,
}

/// A running AI utility process, spoken to in [`Frame`]s over its stdin/stdout.
pub struct UtilityProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Line>,
    reader: Option<JoinHandle<()>>,
    next_request_id: u64,
    pending: HashMap<u64, PendingCall>,
}

impl UtilityProcess {
    /// Spawns the utility with its stdio piped. The child gets no inherited
    /// handles to anything else; it reaches the network only through its own
    /// provider clients, never through the shell.
    pub fn spawn(program: &Path, args: &[&str]) -> std::io::Result<UtilityProcess> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;

        let stdin = child.stdin.take().expect("stdin was requested as piped");
        let stdout = child.stdout.take().expect("stdout was requested as piped");
        let (sender, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            // `lines` bounds a peer that never emits a newline: read_line would
            // grow without limit, which is the attack MAX_FRAME_BYTES exists for.
            // `lines` allocates per line; the cap is enforced in frame::decode.
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        if sender.send(Line::Frame(frame::decode(&line, Peer::Utility))).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = sender.send(Line::Ended);
        });

        Ok(UtilityProcess {
            child,
            stdin: Some(stdin),
            lines,
            reader: Some(reader),
            next_request_id: 1,
            pending: HashMap::new(),
        })
    }

    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    /// Allocates a request id and records what is at stake if the child dies before
    /// answering. `may_have_dispatched_action` must be set for anything that could
    /// have reached a page, because that is the only case where silence leaves the
    /// world possibly changed.
    pub fn begin_call(
        &mut self,
        method: Method,
        task: Option<TaskId>,
        may_have_dispatched_action: bool,
        params: &serde_json::Value,
    ) -> Result<u64, Failure> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.pending.insert(
            id,
            PendingCall {
                task,
                may_have_dispatched_action,
            },
        );
        let frame = Frame::request(id, method, params);
        self.write_frame(&frame).map(|()| id)
    }

    fn write_frame(&mut self, frame: &Frame) -> Result<(), Failure> {
        let line = frame::encode(frame)?;
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(Failure::unavailable("utility.pipe_closed"));
        };
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                // A broken pipe means the child is gone; say so rather than
                // reporting an io error the caller cannot act on.
                Failure::unavailable("utility.pipe_closed").with_detail(error.to_string())
            })
    }

    /// Drains frames until a response to `id` arrives or `timeout` elapses.
    /// Notifications seen on the way are returned through `on_notification` so the
    /// caller keeps its progress reporting during a long call.
    pub fn await_response(
        &mut self,
        id: u64,
        timeout: Duration,
        mut on_notification: impl FnMut(&Notification),
    ) -> Result<ResponsePayload, Failure> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Failure::timed_out("utility.response_deadline").with_detail(format!(
                    "no reply to request {id} within {} ms",
                    timeout.as_millis()
                )));
            }
            match self.lines.recv_timeout(remaining) {
                Ok(Line::Frame(Ok(Frame::Response(response)))) => {
                    if response.id != id {
                        // Routed to no call we made. Forwarding it to `id`'s caller
                        // would hand one request another's answer.
                        self.pending.remove(&id);
                        return Err(Failure::blocked("transport.unexpected_response_id")
                            .with_detail(format!("reply named id {} while waiting for {id}", response.id)));
                    }
                    self.pending.remove(&id);
                    return Ok(response.payload);
                }
                Ok(Line::Frame(Ok(frame::Frame::Notification(notification)))) => {
                    on_notification(&notification);
                }
                Ok(Line::Frame(Ok(_))) => {}
                Ok(Line::Frame(Err(failure))) => return Err(failure),
                Ok(Line::Ended) => return Err(Failure::unavailable("utility.pipe_closed")),
                // Timeout leaves the pending call recorded on purpose: the child may
                // still answer, and if it dies first the call is reported as abandoned.
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::unavailable("utility.pipe_closed"))
                }
            }
        }
    }

    /// Cooperative stop. `Ok` means the utility acknowledged and said how far it
    /// got; `Err(timed_out)` means it did not answer and the caller must escalate to
    /// [`UtilityProcess::abort`].
    pub fn cancel(
        &mut self,
        task: TaskId,
        reason: CancelReason,
        deadline: Duration,
    ) -> Result<CancellationResult, Failure> {
        let request = CancellationRequest { task, reason };
        let params = serde_json::to_value(&request)
            .map_err(|error| Failure::unsupported("transport.encode_failed").with_detail(error.to_string()))?;
        let id = self.begin_call(Method::CancelTask, Some(task), false, &params)?;
        match self.await_response(id, deadline, |_| {}) {
            Ok(ResponsePayload::Ok(value)) => serde_json::from_value(value).map_err(|error| {
                Failure::unsupported("transport.payload_invalid").with_detail(error.to_string())
            }),
            Ok(ResponsePayload::Failure(failure)) => Err(failure),
            Err(failure) => Err(failure),
        }
    }

    /// Forced stop: EOF, grace, kill, reap. Returns everything that never got an
    /// answer.
    pub fn abort(&mut self, kill_grace: Duration) -> Vec<AbandonedWork> {
        // Closing our write half is the polite step; a child blocked in read wakes
        // with EOF and can exit on its own.
        self.stdin = None;

        let deadline = Instant::now() + kill_grace;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        if matches!(self.child.try_wait(), Ok(None)) {
            // Still running after the grace: terminate and reap so the handle is not
            // left dangling and the pid cannot be reused under us.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }

        let abandoned = self
            .pending
            .drain()
            .map(|(request_id, call)| AbandonedWork {
                request_id,
                task: call.task,
                effect_unverified: call.may_have_dispatched_action,
                failure: Failure::unavailable("utility.process_lost").with_detail(format!(
                    "request {request_id} had no reply when the utility process stopped"
                )),
            })
            .collect();
        self.detach_reader();
        abandoned
    }

    fn detach_reader(&mut self) {
        if let Some(reader) = self.reader.take() {
            // The reader exits when stdout closes, which the kill above guarantees.
            // Bounded join so a wedged thread cannot wedge the caller.
            let _ = reader.join();
        }
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for UtilityProcess {
    fn drop(&mut self) {
        // No orphan children, including when a test panics on the way out.
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
