use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Instant;

pub fn process_uptime_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

pub fn elapsed_since(start_tick: u64) -> u64 {
    process_uptime_ms().saturating_sub(start_tick)
}

pub fn current_pid() -> u32 {
    std::process::id()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub working_set: u64,
}

pub fn snapshot_processes() -> Vec<ProcessEntry> {
    let Ok(output) = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,rss=,comm="])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent_pid = fields.next()?.parse().ok()?;
            let working_set = fields.next()?.parse::<u64>().ok()?.saturating_mul(1024);
            let name = fields.collect::<Vec<_>>().join(" ");
            let name = Path::new(&name).file_name()?.to_string_lossy().into_owned();
            Some(ProcessEntry {
                pid,
                parent_pid,
                name,
                working_set,
            })
        })
        .collect()
}

pub fn process_tree(root_pid: u32) -> Vec<ProcessEntry> {
    let mut processes = snapshot_processes();
    let mut seen = std::collections::HashSet::from([root_pid]);
    loop {
        let before = seen.len();
        for process in &processes {
            if seen.contains(&process.parent_pid) {
                seen.insert(process.pid);
            }
        }
        if before == seen.len() {
            break;
        }
    }
    processes.retain(|process| seen.contains(&process.pid));
    processes
}

pub fn working_set_bytes(pid: u32) -> u64 {
    snapshot_processes()
        .into_iter()
        .find(|p| p.pid == pid)
        .map_or(0, |p| p.working_set)
}

pub fn shell_open(path: &str) -> bool {
    Command::new("/usr/bin/open")
        .args(["--", path])
        .status()
        .is_ok_and(|s| s.success())
}

pub fn shell_reveal(path: &str) -> bool {
    Command::new("/usr/bin/open")
        .args(["-R", "--", path])
        .status()
        .is_ok_and(|s| s.success())
}

pub fn shell_open_folder(dir: &str) -> bool {
    shell_open(dir)
}

pub fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}
