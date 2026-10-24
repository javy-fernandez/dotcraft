//! Child processes for build, run and test: output streamed line by line, cancellation that
//! kills the whole process tree (MSBuild nodes, test hosts, the app itself) and an
//! [`OpHandle`] that stops the work when cancelled or dropped.

use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

/// Which pipe a piece of output came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// A line of process output (ANSI sequences kept). `newline` is false for a partial line
/// flushed because the program is waiting (a prompt) or the stream ended without `\n`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: String,
    pub newline: bool,
}

impl OutputLine {
    pub fn stdout(text: impl Into<String>) -> Self {
        OutputLine { stream: Stream::Stdout, text: text.into(), newline: true }
    }
}

/// Shared cancellation flag between a handle and its background task.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<CancelInner>);

#[derive(Default)]
struct CancelInner {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }

    /// Resolves once [`CancelToken::cancel`] has been called.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CancelToken({})", self.is_cancelled())
    }
}

/// A background operation (build, test run…). `cancel()` or dropping the handle stops it and
/// kills its processes; the final event is still sent.
pub struct OpHandle {
    cancel: CancelToken,
    task: Option<JoinHandle<()>>,
}

impl OpHandle {
    pub(crate) fn new(cancel: CancelToken, task: JoinHandle<()>) -> Self {
        OpHandle { cancel, task: Some(task) }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(|t| t.is_finished())
    }

    pub fn token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Lets the operation run to its end even after the handle is dropped.
    pub fn detach(mut self) {
        self.task = None;
    }
}

impl Drop for OpHandle {
    fn drop(&mut self) {
        if self.task.is_some() {
            self.cancel.cancel();
        }
    }
}

/// How a streamed process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitOutcome {
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    pub cancelled: bool,
}

/// Options for [`run_streaming`].
pub(crate) struct StreamOpts {
    /// Flush partial lines after a short pause (interactive programs).
    pub partial_lines: bool,
    /// Time given to the process to exit after SIGTERM before it is killed.
    pub grace: Duration,
    /// Bytes to write to stdin; `None` closes stdin.
    pub stdin: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
}

impl Default for StreamOpts {
    fn default() -> Self {
        StreamOpts { partial_lines: false, grace: Duration::ZERO, stdin: None }
    }
}

/// Spawns `cmd` in its own process group, calls `on_start(pid)`, streams its output to
/// `on_line` and waits for it; cancelling kills the whole tree.
pub(crate) async fn run_streaming(
    mut cmd: Command,
    cancel: &CancelToken,
    mut opts: StreamOpts,
    on_start: &mut (dyn FnMut(u32) + Send),
    on_line: &mut (dyn FnMut(OutputLine) + Send),
) -> std::io::Result<ExitOutcome> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if opts.stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    if cancel.is_cancelled() {
        return Ok(ExitOutcome { code: None, cancelled: true });
    }
    let mut child = cmd.spawn()?;
    let pid = child.id().unwrap_or(0);
    on_start(pid);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut pumps = Vec::new();
    if let Some(out) = child.stdout.take() {
        pumps.push(tokio::spawn(pump(out, Stream::Stdout, tx.clone(), opts.partial_lines)));
    }
    if let Some(err) = child.stderr.take() {
        pumps.push(tokio::spawn(pump(err, Stream::Stderr, tx.clone(), opts.partial_lines)));
    }
    drop(tx);
    let writer = match (child.stdin.take(), opts.stdin.take()) {
        (Some(mut stdin), Some(mut input)) => Some(tokio::spawn(async move {
            while let Some(bytes) = input.recv().await {
                if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        })),
        _ => None,
    };

    let mut cancelled = false;
    let status = loop {
        tokio::select! {
            Some(line) = rx.recv() => on_line(line),
            status = child.wait() => break status?,
            _ = cancel.cancelled(), if !cancelled => {
                cancelled = true;
                terminate(&mut child, pid, opts.grace).await;
            }
        }
    };
    if cancelled {
        kill_tree(pid);
    }
    // Build servers and MSBuild nodes may keep the pipes open after the process exits:
    // read what is left for a moment, then stop listening.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(line) => on_line(line),
                None => break,
            },
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    for p in pumps {
        p.abort();
    }
    if let Some(w) = writer {
        w.abort();
    }
    Ok(ExitOutcome { code: status.code(), cancelled })
}

/// Asks the tree to stop (SIGTERM on Unix), waits up to `grace`, then kills it.
async fn terminate(child: &mut tokio::process::Child, pid: u32, grace: Duration) {
    #[cfg(unix)]
    if !grace.is_zero() && pid != 0 {
        // SAFETY: plain syscall on our own process group.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGTERM);
        }
        if tokio::time::timeout(grace, child.wait()).await.is_ok() {
            return;
        }
    }
    let _ = grace;
    kill_tree(pid);
    let _ = child.start_kill();
}

/// Kills a process and all its descendants (its process group on Unix, `taskkill /T` on
/// Windows). Safe to call on a process that already exited.
pub fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    // SAFETY: plain syscalls; the group was created by `run_streaming` for this child.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

async fn pump(mut r: impl AsyncRead + Unpin, stream: Stream, tx: mpsc::UnboundedSender<OutputLine>, partial: bool) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 8192];
    loop {
        let read = if partial && !buf.is_empty() {
            match tokio::time::timeout(Duration::from_millis(60), r.read(&mut chunk)).await {
                Ok(n) => n,
                Err(_) => {
                    let n = utf8_prefix(&buf);
                    if n > 0 {
                        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                        buf.drain(..n);
                        if tx.send(OutputLine { stream, text, newline: false }).is_err() {
                            return;
                        }
                    }
                    continue;
                }
            }
        } else {
            r.read(&mut chunk).await
        };
        match read {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let mut line: Vec<u8> = buf.drain(..=pos).collect();
                    line.pop();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    let text = String::from_utf8_lossy(&line).into_owned();
                    if tx.send(OutputLine { stream, text, newline: true }).is_err() {
                        return;
                    }
                }
            }
        }
    }
    if !buf.is_empty() {
        let _ = tx.send(OutputLine { stream, text: String::from_utf8_lossy(&buf).into_owned(), newline: false });
    }
}

/// Length of the longest prefix that does not cut a UTF-8 sequence in half.
fn utf8_prefix(buf: &[u8]) -> usize {
    match std::str::from_utf8(buf) {
        Ok(_) => buf.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => buf.len(),
    }
}

/// Removes ANSI escape sequences (CSI colours, OSC titles/links, two-byte escapes).
pub fn strip_ansi(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                // CSI: parameters and intermediates, then a final byte in @..~
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC: until BEL or ESC \
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('(') | Some(')') => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

/// Splits a command line the way .NET does for `commandLineArgs`: whitespace separates,
/// double quotes group, `\"` is a literal quote.
pub fn split_command_line(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'"') => {
                chars.next();
                cur.push('"');
                in_arg = true;
            }
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut cur));
                    in_arg = false;
                }
            }
            c => {
                cur.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(cur);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colours_titles_and_links() {
        assert_eq!(strip_ansi("\x1b[1;31merror\x1b[0m: x"), "error: x");
        assert_eq!(strip_ansi("\x1b]0;title\x07ok"), "ok");
        assert_eq!(strip_ansi("\x1b]8;;http://a\x1b\\link\x1b]8;;\x1b\\"), "link");
        assert_eq!(strip_ansi("\x1b[?25lplain\x1b(B"), "plain");
        assert_eq!(strip_ansi("ñandú"), "ñandú");
    }

    #[test]
    fn splits_command_lines() {
        assert_eq!(split_command_line(r#"Grace --coupon "SAVE 10"  x"#), vec!["Grace", "--coupon", "SAVE 10", "x"]);
        assert_eq!(split_command_line(r#"a\"b "" c"#), vec!["a\"b", "", "c"]);
        assert!(split_command_line("   ").is_empty());
    }

    #[test]
    fn utf8_prefix_does_not_cut_characters() {
        let s = "añ".as_bytes();
        assert_eq!(utf8_prefix(&s[..2]), 1);
        assert_eq!(utf8_prefix(s), 3);
    }

    #[test]
    fn cancel_token_wakes_waiters() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let t = CancelToken::new();
            let t2 = t.clone();
            let h = tokio::spawn(async move { t2.cancelled().await });
            tokio::task::yield_now().await;
            t.cancel();
            tokio::time::timeout(Duration::from_secs(1), h).await.unwrap().unwrap();
            assert!(t.is_cancelled());
        });
    }

    #[cfg(unix)]
    #[test]
    fn streams_lines_and_partial_prompts_and_kills_trees() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "printf 'one\\r\\ntwo\\nask: '; echo err 1>&2; sleep 0.3; echo three"]);
            let mut lines = Vec::new();
            let opts = StreamOpts { partial_lines: true, ..Default::default() };
            let out = run_streaming(cmd, &CancelToken::new(), opts, &mut |_| {}, &mut |l| lines.push(l)).await.unwrap();
            assert_eq!(out, ExitOutcome { code: Some(0), cancelled: false });
            let texts: Vec<_> = lines.iter().filter(|l| l.stream == Stream::Stdout).map(|l| (l.text.as_str(), l.newline)).collect();
            assert_eq!(texts, vec![("one", true), ("two", true), ("ask: ", false), ("three", true)]);
            assert!(lines.iter().any(|l| l.stream == Stream::Stderr && l.text == "err"));

            // A grandchild that would outlive its parent dies with the tree.
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "sleep 30 & echo started; wait"]);
            let cancel = CancelToken::new();
            let c2 = cancel.clone();
            let started = std::time::Instant::now();
            let out = run_streaming(cmd, &cancel, StreamOpts::default(), &mut |_| {}, &mut |l| {
                if l.text == "started" {
                    c2.cancel();
                }
            })
            .await
            .unwrap();
            assert!(out.cancelled);
            assert!(started.elapsed() < Duration::from_secs(5));
        });
    }
}
