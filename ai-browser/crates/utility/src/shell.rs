//! The utility's view of the pipe back to the shell, as a trait so the task and
//! act logic can be driven by an in-memory fake in tests.

use ai_platform_contract::failure::Failure;
use ai_platform_contract::frame::{Method, ResponsePayload};

use crate::control::TaskControl;

pub trait ShellLink: Send + Sync {
    /// Fire-and-forget (`task.progress`, `task.finish`).
    fn notify(&self, method: Method, params: &serde_json::Value);

    /// Sends a request and waits for its response, giving up with the task's
    /// stop failure as soon as `control` says stop. `Err` means no response
    /// arrived (stopped, or the pipe is gone); a shell-side refusal is
    /// `Ok(ResponsePayload::Failure(..))`.
    fn call(
        &self,
        method: Method,
        params: &serde_json::Value,
        control: &TaskControl,
    ) -> Result<ResponsePayload, Failure>;
}
