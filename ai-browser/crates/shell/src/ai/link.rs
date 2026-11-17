//! The shell's end of the link to the AI engine.
//!
//! The engine (`ai-utility`: providers, prompts, routing, the task and agent loops)
//! runs inside the browser process on its own thread, not in a child process.
//! Browser navigation and the model loop never share a thread, and a panic in the
//! engine is caught and reported as a closed link, so a failed model call cannot
//! take the browser down.
//!
//! The engine still speaks the contract's NDJSON frames. That keeps one wire format
//! for the engine's tests, and keeps the shell's decode gate (which side may send
//! which method) in force, but the bytes travel over in-memory channels instead of
//! a pipe. A reader thread decodes what the engine sends and hands it to a callback
//! that posts into the UI event loop; sending never blocks the UI thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use ai_platform_contract::failure::Failure;
use ai_platform_contract::frame::{decode, encode, Frame, Peer};

/// Byte stream over an in-memory channel: one end of an in-process "pipe".
struct ChannelReader {
    rx: Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    pos: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.chunk.len() {
            match self.rx.recv() {
                Ok(next) => {
                    self.chunk = next;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.chunk.len() - self.pos);
        out[..n].copy_from_slice(&self.chunk[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct ChannelWriter(Sender<Vec<u8>>);

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(buf.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "link closed"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct Link {
    /// Dropping this ends the engine's input, which stops its tasks and thread.
    outbox: Sender<Vec<u8>>,
    next_id: u64,
    running: Arc<AtomicBool>,
}

impl Link {
    /// Start the engine thread and begin delivering its frames. `on_frame` runs on
    /// the reader thread, so it must only forward (post to the event loop).
    /// Credentials belong only to this engine instance, never the process environment.
    pub fn spawn(
        config: &Path,
        envs: &[(String, String)],
        on_frame: impl Fn(Frame) + Send + 'static,
        on_closed: impl FnOnce(String) + Send + 'static,
    ) -> Result<Link, Failure> {
        let credentials = envs.iter().cloned().collect();
        let (to_engine, engine_in) = channel::<Vec<u8>>();
        let (engine_out, from_engine) = channel::<Vec<u8>>();
        let running = Arc::new(AtomicBool::new(true));

        let config = config.to_path_buf();
        let engine_flag = running.clone();
        std::thread::Builder::new()
            .name("ai-engine".into())
            .spawn(move || {
                let input = BufReader::new(ChannelReader { rx: engine_in, chunk: Vec::new(), pos: 0 });
                let output: Box<dyn Write + Send> = Box::new(ChannelWriter(engine_out));
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ai_utility::server::run_with_credentials(input, output, &config, credentials)
                }));
                if outcome.is_err() {
                    eprintln!("ai engine: panicked; the browser is unaffected");
                }
                engine_flag.store(false, Ordering::SeqCst);
            })
            .map_err(|e| Failure::unavailable("engine.spawn_failed").with_detail(e.to_string()))?;

        let reader_flag = running.clone();
        std::thread::Builder::new()
            .name("ai-link-reader".into())
            .spawn(move || {
                let reason = read_loop(BufReader::new(ChannelReader { rx: from_engine, chunk: Vec::new(), pos: 0 }), &on_frame);
                reader_flag.store(false, Ordering::SeqCst);
                on_closed(reason);
            })
            .map_err(|e| Failure::unavailable("engine.spawn_failed").with_detail(e.to_string()))?;

        Ok(Link { outbox: to_engine, next_id: 1, running })
    }

    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn send(&self, frame: &Frame) -> Result<(), Failure> {
        let mut line = encode(frame)?;
        line.push('\n');
        self.outbox
            .send(line.into_bytes())
            .map_err(|_| Failure::unavailable("engine.link_closed"))
    }

    /// False once the engine thread has stopped.
    pub fn alive(&mut self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

fn read_loop(reader: impl BufRead, on_frame: &impl Fn(Frame)) -> String {
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => return format!("read error: {e}"),
        };
        if line.trim().is_empty() {
            continue;
        }
        match decode(&line, Peer::Utility) {
            Ok(frame) => on_frame(frame),
            // A malformed or wrong-side frame is dropped and reported; it never
            // reaches a handler, and one bad line does not end the session.
            Err(failure) => eprintln!("ai link: dropped frame: {}", failure.code),
        }
    }
    "engine closed its output".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_platform_contract::frame::Method;
    use std::time::Duration;

    #[test]
    fn engine_answers_ping_in_process_and_stops_when_dropped() {
        let dir = std::env::temp_dir().join(format!("link-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (frames, seen) = channel::<Frame>();
        let (closed_tx, closed) = channel::<String>();
        let mut link = Link::spawn(
            &dir.join("ai.json"),
            &[],
            move |f| {
                let _ = frames.send(f);
            },
            move |r| {
                let _ = closed_tx.send(r);
            },
        )
        .unwrap();
        let id = link.next_id();
        link.send(&Frame::request(id, Method::Ping, &serde_json::json!({}))).unwrap();
        match seen.recv_timeout(Duration::from_secs(5)).unwrap() {
            Frame::Response(r) => assert_eq!(r.id, id),
            other => panic!("unexpected frame {other:?}"),
        }
        assert!(link.alive());
        drop(link);
        assert!(closed.recv_timeout(Duration::from_secs(5)).is_ok(), "engine must stop when the link is dropped");
        let _ = std::fs::remove_dir_all(dir);
    }
}
