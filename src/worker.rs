//! Isolated rendering, shared wire protocol and process supervision.
use crate::{
    MAX_PIXELS, Raster, RasterOptions,
    mermaid::{self, Background, MAX_SOURCE},
    process::{RequestError, RequestGuard, kill_and_reap},
};
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
/// A worker exits after this many requests; applications can recycle it sooner.
pub const MAX_REQUESTS: usize = 32;
const MAX_RESIDENT_BYTES: u64 = 256 * 1024 * 1024;

/// Private child entry point; invoke before application initialization.
pub fn worker_main() -> io::Result<()> {
    restrict_worker()?;
    serve(&mut io::stdin().lock(), &mut io::stdout().lock())
}

fn restrict_worker() -> io::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: valid pointers to process-local resource limits. Fail closed if
        // the CPU/core-dump backstops cannot be applied to this dedicated child.
        for (resource, seconds) in [(libc::RLIMIT_CPU, 8), (libc::RLIMIT_CORE, 0)] {
            let limit = libc::rlimit {
                rlim_cur: seconds,
                rlim_max: seconds,
            };
            if unsafe { libc::setrlimit(resource, &limit) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // Lower priority is best effort; it does not enforce a resource limit.
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::{CStr, c_char};
        unsafe extern "C" {
            fn sandbox_init(profile: *const c_char, flags: u64, error: *mut *mut c_char) -> i32;
            fn sandbox_free_error(error: *mut c_char);
        }
        // All fonts are embedded; this worker needs no document paths, external
        // images, system fonts, writes, network, or child processes. Fail closed.
        let profile = c"(version 1)(allow default)(deny network*)(deny file-write*)(deny file-read-data)(deny process-exec)(deny process-fork)";
        let mut error = std::ptr::null_mut();
        // SAFETY: static NUL-terminated profile and valid out-pointer.
        if unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) } != 0 {
            let message = if error.is_null() {
                "Cannot restrict diagram worker".into()
            } else {
                unsafe { CStr::from_ptr(error) }
                    .to_string_lossy()
                    .into_owned()
            };
            if !error.is_null() {
                unsafe { sandbox_free_error(error) };
            }
            return Err(io::Error::other(message));
        }
    }
    Ok(())
}

fn serve(input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    for _ in 0..MAX_REQUESTS {
        let mut header = [0; 25];
        match input.read(&mut header[..1])? {
            0 => return Ok(()),
            _ => input.read_exact(&mut header[1..])?,
        }
        let number = |slice: &[u8]| u32::from_le_bytes(slice.try_into().unwrap());
        let length = number(&header[12..16]) as usize;
        let options = RasterOptions {
            bounds: (number(&header[4..8]), number(&header[8..12])),
            scale: f64::from_le_bytes(header[17..25].try_into().unwrap()),
        };
        let valid_flags = match &header[..4] {
            b"NPV1" => header[16] <= 1,
            b"NPV2" => header[16] <= 3,
            _ => false,
        };
        if !valid_flags || !options.valid() || length == 0 || length > MAX_SOURCE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid diagram request",
            ));
        }
        let mut source = vec![0; length];
        input.read_exact(&mut source)?;
        let background = if header[16] & 2 == 0 {
            Background::Theme
        } else {
            Background::Transparent
        };
        let pixels = std::str::from_utf8(&source)
            .ok()
            .and_then(|s| mermaid::render(s, header[16] & 1 != 0, background, options));
        if let Some(p) = pixels {
            for n in [
                p.width,
                p.height,
                p.rgba.len() as u32,
                p.original_width,
                p.original_height,
            ] {
                output.write_all(&n.to_le_bytes())?;
            }
            output.write_all(&p.rgba)?;
        } else {
            output.write_all(&[0; 20])?;
        }
        output.flush()?;
    }
    Ok(())
}

fn response(input: &mut impl Read, bounds: (u32, u32)) -> io::Result<Option<Raster>> {
    let mut header = [0; 20];
    input.read_exact(&mut header)?;
    let number = |slice: &[u8]| u32::from_le_bytes(slice.try_into().unwrap());
    let (width, height, length) = (
        number(&header[..4]),
        number(&header[4..8]),
        number(&header[8..12]) as usize,
    );
    let original_width = number(&header[12..16]);
    let original_height = number(&header[16..20]);
    if width == 0 && height == 0 && length == 0 && original_width == 0 && original_height == 0 {
        return Ok(None);
    }
    let pixels = u64::from(width) * u64::from(height);
    if original_width == 0
        || original_height == 0
        || original_width > crate::svg::MAX_DIAGRAM_EDGE
        || original_height > crate::svg::MAX_DIAGRAM_EDGE
        || u64::from(original_width) * u64::from(original_height) > crate::svg::MAX_DIAGRAM_AREA
        || width == 0
        || height == 0
        || width > bounds.0
        || height > bounds.1
        || pixels > MAX_PIXELS as u64
        || pixels * 4 != length as u64
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Mermaid pixels",
        ));
    }
    let mut rgba = vec![0; length];
    input.read_exact(&mut rgba)?;
    Ok(Some(Raster {
        original_width,
        original_height,
        width,
        height,
        rgba,
    }))
}

type Request = (Vec<u8>, (u32, u32));
/// One child process with bounded messages, cancellation and failure cleanup.
/// Applications own the scheduling queue and decide when to recycle or retire it.
pub struct Worker {
    child: Child,
    requests: Option<mpsc::SyncSender<Request>>,
    replies: mpsc::Receiver<io::Result<Option<Raster>>>,
    thread: Option<JoinHandle<()>>,
    completed: usize,
}
impl Worker {
    pub fn requests_completed(&self) -> usize {
        self.completed
    }
    pub fn spawn(program: &Path, worker_argument: &str) -> io::Result<Self> {
        let child = Command::new(program)
            .arg(worker_argument)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        Self::connect(child)
    }

    fn connect(mut child: Child) -> io::Result<Self> {
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        let (replies, outgoing) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("mermaid-io".into())
            .spawn(move || {
                while let Ok((message, bounds)) = incoming.recv() {
                    let result = input
                        .write_all(&message)
                        .and_then(|()| input.flush())
                        .and_then(|()| response(&mut output, bounds));
                    let failed = result.is_err();
                    if replies.send(result).is_err() || failed {
                        break;
                    }
                }
            });
        match thread {
            Ok(thread) => Ok(Self {
                child,
                requests: Some(requests),
                replies: outgoing,
                thread: Some(thread),
                completed: 0,
            }),
            Err(error) => {
                kill_and_reap(&mut child);
                Err(error)
            }
        }
    }

    /// Render one diagram off the UI thread. Any error stops and reaps the
    /// worker; subsequent calls fail until the application creates a new handle.
    pub fn render(
        &mut self,
        source: &str,
        light: bool,
        options: RasterOptions,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> io::Result<Option<Raster>> {
        self.render_with_background(source, light, Background::Theme, options, timeout, cancel)
    }

    /// Render with an explicit canvas treatment. Transparency is applied before
    /// rasterization, preserving antialiased edges and intentional diagram fills.
    /// Uses the same limits, cancellation and worker cleanup as [`Self::render`].
    pub fn render_with_background(
        &mut self,
        source: &str,
        light: bool,
        background: Background,
        options: RasterOptions,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> io::Result<Option<Raster>> {
        let result = self.render_inner(source, light, background, options, timeout, cancel);
        if result.is_err() {
            self.stop();
        }
        result
    }
    fn render_inner(
        &mut self,
        source: &str,
        light: bool,
        background: Background,
        options: RasterOptions,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> io::Result<Option<Raster>> {
        if self.requests.is_none() {
            return Err(io::Error::other("Diagram worker stopped"));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if !options.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid raster options",
            ));
        }
        if !mermaid::admissible(source) {
            return Ok(None);
        }
        if self.completed >= MAX_REQUESTS {
            return Err(io::Error::other("Diagram worker request limit reached"));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid render timeout"))?;
        let mut guard = RequestGuard::new(cancel, deadline, MAX_RESIDENT_BYTES);
        let bounds = options.bounds;
        let mut message = Vec::with_capacity(25 + source.len());
        let transparent = background == Background::Transparent;
        message.extend_from_slice(if transparent { b"NPV2" } else { b"NPV1" });
        message.extend_from_slice(&bounds.0.to_le_bytes());
        message.extend_from_slice(&bounds.1.to_le_bytes());
        message.extend_from_slice(&(source.len() as u32).to_le_bytes());
        message.push(u8::from(light) | (u8::from(transparent) << 1));
        message.extend_from_slice(&options.scale.to_le_bytes());
        message.extend_from_slice(source.as_bytes());
        self.requests
            .as_ref()
            .unwrap()
            .try_send((message, bounds))
            .map_err(|_| io::Error::other("Mermaid worker unavailable"))?;
        loop {
            match guard.check(self.child.id(), false) {
                // Preserve the diagram worker's best-effort RSS monitoring.
                #[cfg(target_os = "macos")]
                Err(RequestError::MemoryUnavailable) => {}
                result => result.map_err(request_error)?,
            }
            let remaining = guard.remaining();
            match self
                .replies
                .recv_timeout(remaining.min(Duration::from_millis(20)))
            {
                Ok(result) => {
                    // A queued reply does not override cancellation or a deadline
                    // reached while this thread was waiting to be scheduled.
                    guard.check_completion().map_err(request_error)?;
                    self.completed += 1;
                    return result;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(io::Error::other("Diagram worker stopped")),
            }
        }
    }
    fn stop(&mut self) {
        // The worker starts no descendants. Killing it closes both pipes and
        // unblocks the I/O thread even after a partial write or partial reply.
        kill_and_reap(&mut self.child);
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn request_error(error: RequestError) -> io::Error {
    match error {
        RequestError::Cancelled => io::Error::new(io::ErrorKind::Interrupted, "Diagram cancelled"),
        RequestError::TimedOut => io::Error::new(io::ErrorKind::TimedOut, "Diagram timed out"),
        #[cfg(target_os = "macos")]
        RequestError::MemoryExceeded => io::Error::other("Diagram exceeded its memory limit"),
        #[cfg(target_os = "macos")]
        RequestError::MemoryUnavailable => io::Error::other("Cannot monitor diagram worker memory"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_EDGE;
    #[cfg(unix)]
    use std::sync::Arc;
    #[test]
    fn worker_protocol_rejects_invalid_or_truncated_data_before_allocating() {
        let mut data = Vec::new();
        for n in [2u32, 1, 8, 1, 1] {
            data.extend_from_slice(&n.to_le_bytes());
        }
        data.extend_from_slice(&[255, 0, 0, 128, 0, 0, 255, 255]);
        let p = response(&mut &data[..], (MAX_EDGE, MAX_EDGE))
            .unwrap()
            .unwrap();
        assert_eq!((p.width, p.height), (2, 1));
        assert_eq!(p.rgba, data[20..]);
        // Large logical dimensions do not increase the pixel allocation.
        data[12..16].copy_from_slice(&10000_u32.to_le_bytes());
        data[16..20].copy_from_slice(&1500_u32.to_le_bytes());
        let p = response(&mut &data[..], (MAX_EDGE, MAX_EDGE))
            .unwrap()
            .unwrap();
        assert_eq!((p.original_width, p.original_height), (10000, 1500));
        assert_eq!(p.rgba.len(), 8);
        data[12..16].copy_from_slice(&(crate::svg::MAX_DIAGRAM_EDGE + 1).to_le_bytes());
        assert!(response(&mut &data[..], (MAX_EDGE, MAX_EDGE)).is_err());
        data[12..16].copy_from_slice(&32000_u32.to_le_bytes());
        data[16..20].copy_from_slice(&32000_u32.to_le_bytes());
        assert!(response(&mut &data[..], (MAX_EDGE, MAX_EDGE)).is_err());
        data[12..16].copy_from_slice(&1_u32.to_le_bytes());
        data[16..20].copy_from_slice(&1_u32.to_le_bytes());
        assert!(response(&mut &data[..27], (MAX_EDGE, MAX_EDGE)).is_err());
        assert!(response(&mut &data[..], (1, 1)).is_err());
        assert!(
            response(&mut &[0; 20][..], (MAX_EDGE, MAX_EDGE))
                .unwrap()
                .is_none()
        );
        data[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(response(&mut &data[..], (MAX_EDGE, MAX_EDGE)).is_err());
        let mut request = Vec::from(&b"NPV1"[..]);
        for n in [MAX_EDGE, MAX_EDGE, MAX_SOURCE as u32 + 1] {
            request.extend_from_slice(&n.to_le_bytes());
        }
        request.push(0);
        request.extend_from_slice(&1_f64.to_le_bytes());
        let mut output = Vec::new();
        assert!(serve(&mut &request[..], &mut output).is_err());
        assert!(output.is_empty());
        // The original protocol only accepts the light bit; the transparency
        // extension still rejects unknown flags before reading a source body.
        request[12..16].copy_from_slice(&1_u32.to_le_bytes());
        for (magic, flags) in [(b"NPV1", 2), (b"NPV2", 4), (b"NPV2", 255)] {
            request[..4].copy_from_slice(magic);
            request[16] = flags;
            let error = serve(&mut &request[..], &mut output).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(output.is_empty());
        }
    }
    #[cfg(unix)]
    #[test]
    fn a_reply_after_a_short_deadline_is_rejected() {
        let child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut worker = Worker::connect(child).unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        worker.replies = receive;
        let delayed = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            let _ = send.send(Ok(None));
        });
        let error = worker
            .render(
                "flowchart LR; A-->B",
                false,
                RasterOptions::scaled(1.),
                Duration::from_millis(1),
                &AtomicBool::new(false),
            )
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(worker.requests.is_none());
        assert_eq!(worker.requests_completed(), 0);
        delayed.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn timeouts_and_cancellation_kill_and_reap_partial_replies() {
        use std::os::unix::process::ExitStatusExt;

        const SOURCE: &str = "flowchart LR; A-->B";
        for cancel_in_flight in [false, true] {
            // The cancellation case reads the request before sending one reply
            // byte and acknowledging it on stderr. A sleep cannot establish this
            // ordering when the test runner is busy.
            let request_bytes = if cancel_in_flight {
                25 + SOURCE.len()
            } else {
                0
            };
            let mut child = Command::new("sh")
                .args([
                    "-c",
                    "dd bs=1 count=\"$1\" of=/dev/null 2>/dev/null; printf '\\001'; printf '\\001' >&2; exec sleep 60",
                    "partial-reply-worker",
                    &request_bytes.to_string(),
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut ready = child.stderr.take().unwrap();
            let mut worker = Worker::connect(child).unwrap();
            let pid = worker.child.id();
            let cancel = Arc::new(AtomicBool::new(false));
            let cancellation = if cancel_in_flight {
                let cancel = cancel.clone();
                Some(thread::spawn(move || {
                    let mut marker = [0];
                    ready.read_exact(&mut marker).unwrap();
                    assert_eq!(marker, [1]);
                    cancel.store(true, Ordering::Relaxed);
                }))
            } else {
                // Start the timeout only once the child has emitted its partial
                // reply, so process startup is not part of the deadline test.
                let mut marker = [0];
                ready.read_exact(&mut marker).unwrap();
                assert_eq!(marker, [1]);
                None
            };
            // Only the timeout case has a short deadline. The cancellation case
            // has a generous failure backstop, not a competing timing assertion.
            let timeout = if cancel_in_flight {
                Duration::from_secs(30)
            } else {
                Duration::from_millis(100)
            };
            let error = worker
                .render(SOURCE, false, RasterOptions::scaled(1.), timeout, &cancel)
                .err()
                .unwrap();
            if let Some(thread) = cancellation {
                thread.join().unwrap();
            }
            assert_eq!(
                error.kind(),
                if cancel_in_flight {
                    io::ErrorKind::Interrupted
                } else {
                    io::ErrorKind::TimedOut
                }
            );
            assert!(worker.requests.is_none());
            // SAFETY: signal zero only checks whether the reaped child exists.
            assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
            assert_eq!(
                worker.child.try_wait().unwrap().unwrap().signal(),
                Some(libc::SIGKILL)
            );
            assert!(
                worker
                    .render(
                        "flowchart LR; A-->B",
                        false,
                        RasterOptions::scaled(1.),
                        Duration::from_millis(100),
                        &AtomicBool::new(false)
                    )
                    .is_err()
            );
        }
    }
    /// The sandbox applies to a whole process, so `sandbox_probe` installs it in
    /// a child run of this test binary.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_sandbox_denies_file_reads_writes_network_and_processes() {
        let file = std::env::temp_dir().join(format!("document-graphics-{}", std::process::id()));
        std::fs::write(&file, b"private").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["worker::tests::sandbox_probe", "--exact", "--ignored"])
            .env("DOCUMENT_GRAPHICS_SANDBOX_PROBE", &file)
            .output()
            .unwrap();
        let contents = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        let report = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && report.contains("1 passed"),
            "{report}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(contents, b"private");
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "run in a child process by the macOS sandbox test"]
    fn sandbox_probe() {
        // Never restrict the test runner itself, only the child started above.
        let Some(file) = std::env::var_os("DOCUMENT_GRAPHICS_SANDBOX_PROBE") else {
            return;
        };
        // Each operation works before the sandbox, so failing after is its doing.
        let spawn = || Command::new("/usr/bin/true").status();
        assert!(std::fs::read(&file).is_ok() && spawn().is_ok());
        assert!(std::net::TcpListener::bind("127.0.0.1:0").is_ok());
        restrict_worker().unwrap();
        assert!(std::fs::read(&file).is_err(), "read a file");
        assert!(std::fs::write(&file, b"changed").is_err(), "wrote a file");
        assert!(
            std::net::TcpListener::bind("127.0.0.1:0").is_err(),
            "opened a socket"
        );
        assert!(spawn().is_err(), "started a process");
    }
}
