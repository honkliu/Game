//! AI utility process: providers, routing, context reduction, the task loop and
//! the NDJSON server. See `README`-level notes in `ai-browser/STATUS.md`.

pub mod act;
pub mod agent;
pub mod audit;
pub mod control;
pub mod prompt;
pub mod provider;
pub mod reduce;
pub mod reply;
pub mod router;
pub mod server;
pub mod secrets;
pub mod shell;
pub mod tabs;
pub mod task;
pub mod text;

use std::io::Write;
use std::path::PathBuf;

/// Environment variable naming the `ai.json` the utility process reads.
pub const CONFIG_ENV: &str = "AI_BROWSER_CONFIG";

/// Run the utility process to completion over this process's stdin/stdout.
/// Returns the exit code. Protocol frames only on stdout; audit lines on stderr.
pub fn run_process() -> i32 {
    let path = std::env::var_os(CONFIG_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ai.json"));
    let stdin = std::io::stdin();
    let stdout: Box<dyn Write + Send> = Box::new(std::io::stdout());
    match server::run(stdin.lock(), stdout, &path) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("ai-utility: stdio error: {}", error.kind());
            1
        }
    }
}
