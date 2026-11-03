//! Blocking image worker client. Applications own queues, snapshots and caches.
use super::{
    Frame, Info, Pixels, Region, check,
    protocol::{self, Operation, Response},
};
use crate::process::{RequestError, RequestGuard, kill_and_reap};
#[cfg(unix)]
use std::io::Write;
use std::{fs::File, path::Path, sync::atomic::AtomicBool};
use std::{
    io::{self, Read},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};
enum Answer {
    Info(Info),
    Frame(Frame),
}

const TIMEOUT: Duration = Duration::from_secs(10);
const RSS_LIMIT: u64 = 256 * 1024 * 1024;
pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    _source: File,
    failed: bool,
    info: Option<Info>,
}
impl Worker {
    /// `source` must be a stable, seekable regular file supplied by the caller.
    /// The worker never resolves a document path or snapshots a changing source.
    pub fn spawn(executable: &Path, argument: &str, source: &File) -> Result<Self, String> {
        if !cfg!(unix) {
            return Err("Isolated image previews are unavailable on this platform".into());
        }
        let mut command = Command::new(executable);
        command.arg(argument);
        Self::start(command, source.try_clone().map_err(|e| e.to_string())?)
    }
    fn start(mut command: Command, source_file: File) -> Result<Self, String> {
        let metadata = source_file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > super::MAX_FILE_BYTES {
            return Err("Image exceeds the 256 MiB file limit or is not a regular file".into());
        }
        #[cfg(unix)]
        {
            use std::os::{fd::AsRawFd, unix::process::CommandExt};
            let source = source_file.as_raw_fd();
            // SAFETY: only async-signal-safe descriptor operations before exec.
            unsafe {
                command.pre_exec(move || {
                    if libc::dup2(source, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let mut child = command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let worker = Self {
            child,
            input,
            output,
            _source: source_file,
            failed: false,
            info: None,
        };
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: the pipe is owned by worker; failure drops and reaps the child.
            let result = unsafe {
                let fd = worker.output.as_raw_fd();
                let flags = libc::fcntl(fd, libc::F_GETFL);
                if flags < 0 {
                    -1
                } else {
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK)
                }
            };
            if result < 0 {
                return Err(io::Error::last_os_error().to_string());
            }
        }
        Ok(worker)
    }
    pub fn info(&mut self, cancel: &AtomicBool) -> Result<Info, String> {
        match self.run(Operation::Info, cancel)? {
            Answer::Info(info) => Ok(info),
            _ => Err("Unexpected image response".into()),
        }
    }
    pub fn render(&mut self, regions: Vec<Region>, cancel: &AtomicBool) -> Result<Frame, String> {
        match self.run(Operation::Render(regions), cancel)? {
            Answer::Frame(frame) => Ok(frame),
            _ => Err("Unexpected image response".into()),
        }
    }
    fn run(&mut self, operation: Operation, cancel: &AtomicBool) -> Result<Answer, String> {
        if self.failed {
            return Err("Image worker stopped; start a new worker".into());
        }
        let result = self.request(operation, cancel);
        if result.is_err() {
            self.failed = true;
            kill_and_reap(&mut self.child);
        }
        result
    }
    fn request(&mut self, operation: Operation, cancel: &AtomicBool) -> Result<Answer, String> {
        check(cancel)?;
        if let Operation::Render(regions) = &operation {
            super::validate_regions(self.info.as_ref().ok_or("Image metadata missing")?, regions)?;
        }
        protocol::write(&mut self.input, &operation).map_err(|e| e.to_string())?;
        let mut reader = Interruptible {
            worker: self,
            guard: RequestGuard::new(cancel, Instant::now() + TIMEOUT, RSS_LIMIT),
        };
        let response: Response = protocol::read(&mut reader).map_err(|e| e.to_string())?;
        match response {
            Response::Info(info) => {
                if !matches!(operation, Operation::Info) {
                    return Err("Unexpected image metadata".into());
                }
                info.validate()?;
                reader.resources(true).map_err(|e| e.to_string())?;
                reader.worker.info = Some(info.clone());
                Ok(Answer::Info(info))
            }
            Response::Pixels { regions, delay_ms } => {
                let Operation::Render(wanted) = operation else {
                    return Err("Unexpected image pixels".into());
                };
                if regions != wanted || !(11..=60_000).contains(&delay_ms) {
                    return Err("Invalid image worker response".into());
                }
                let mut pixels = Vec::with_capacity(regions.len());
                for region in regions {
                    let mut rgba = vec![0; region.bytes()];
                    reader.read_exact(&mut rgba).map_err(|e| e.to_string())?;
                    pixels.push(Pixels { region, rgba });
                }
                reader.resources(true).map_err(|e| e.to_string())?;
                Ok(Answer::Frame(Frame { pixels, delay_ms }))
            }
            Response::Error(error) => Err(error),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        kill_and_reap(&mut self.child);
    }
}
struct Interruptible<'a> {
    worker: &'a mut Worker,
    guard: RequestGuard<'a>,
}
impl Interruptible<'_> {
    fn resources(&mut self, force: bool) -> io::Result<()> {
        self.guard
            .check(self.worker.child.id(), force)
            .map_err(|error| {
                io::Error::other(match error {
                    RequestError::Cancelled => super::CANCELLED,
                    RequestError::TimedOut => {
                        "Image decode exceeded 10 seconds; use Source/Binary or retry"
                    }
                    #[cfg(target_os = "macos")]
                    RequestError::MemoryUnavailable => "Cannot monitor image worker memory",
                    #[cfg(target_os = "macos")]
                    RequestError::MemoryExceeded => {
                        "Image decoder exceeded 256 MiB; overview or Binary remains available"
                    }
                })
            })
    }
}
impl Read for Interruptible<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            self.resources(false)?;
            match self.worker.output.read(bytes) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    #[cfg(unix)]
                    {
                        use std::os::fd::AsRawFd;
                        let mut poll = libc::pollfd {
                            fd: self.worker.output.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        // SAFETY: one valid pipe; wake regularly for cancellation and resource checks.
                        if unsafe { libc::poll(&mut poll, 1, 10) } < 0 {
                            let e = io::Error::last_os_error();
                            if e.kind() != io::ErrorKind::Interrupted {
                                return Err(e);
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
}
pub fn worker_main() -> Result<(), String> {
    #[cfg(unix)]
    {
        #[cfg(target_os = "macos")]
        restrict_worker()?;
        // SAFETY: child-only process resource settings and a terminating backstop.
        unsafe {
            libc::setrlimit(
                libc::RLIMIT_CORE,
                &libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                },
            );
            #[cfg(not(target_os = "macos"))]
            libc::setrlimit(
                libc::RLIMIT_AS,
                &libc::rlimit {
                    rlim_cur: 512 * 1024 * 1024,
                    rlim_max: 512 * 1024 * 1024,
                },
            );
            libc::alarm(12);
        }
        let document = super::backend::Decoder::open(std::path::Path::new("/dev/fd/3"));
        unsafe {
            libc::alarm(0);
        }
        let mut input = io::stdin().lock();
        let mut output = io::stdout().lock();
        loop {
            let operation: Operation = match protocol::read(&mut input) {
                Ok(op) => op,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e.to_string()),
            };
            unsafe {
                libc::alarm(12);
            }
            let result = (|| {
                let decoder = document.as_ref().map_err(Clone::clone)?;
                match operation {
                    Operation::Info => Ok(Answer::Info(decoder.info())),
                    Operation::Render(regions) => decoder.render(&regions).map(Answer::Frame),
                }
            })();
            unsafe {
                libc::alarm(0);
            }
            match result {
                Ok(Answer::Info(info)) => protocol::write(&mut output, &Response::Info(info)),
                Ok(Answer::Frame(frame)) => (|| {
                    protocol::write(
                        &mut output,
                        &Response::Pixels {
                            regions: frame.pixels.iter().map(|p| p.region).collect(),
                            delay_ms: frame.delay_ms,
                        },
                    )?;
                    for pixels in frame.pixels {
                        output.write_all(&pixels.rgba)?;
                    }
                    output.flush()
                })(),
                Err(error) => protocol::write(
                    &mut output,
                    &Response::Error(error.chars().take(2048).collect()),
                ),
            }
            .map_err(|e| e.to_string())?;
        }
    }
    #[cfg(not(unix))]
    Err("Isolated image previews are unavailable on this platform".into())
}
#[cfg(target_os = "macos")]
fn restrict_worker() -> Result<(), String> {
    unsafe extern "C" {
        fn sandbox_init(
            profile: *const std::ffi::c_char,
            flags: u64,
            error: *mut *mut std::ffi::c_char,
        ) -> i32;
        fn sandbox_free_error(error: *mut std::ffi::c_char);
    }
    // Fail closed if the native-codec sandbox cannot be installed. Reads remain available to
    // Core Graphics/font services; writes, networking and child execution do not.
    let profile = c"(version 1)(allow default)(deny network*)(deny file-write*)(deny process-exec)(deny process-fork)";
    let mut error = std::ptr::null_mut();
    // SAFETY: constant NUL-terminated profile; error is allocated by sandbox_init.
    let result = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
    if result != 0 {
        let message = if error.is_null() {
            "Cannot restrict image worker".into()
        } else {
            unsafe { std::ffi::CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        };
        unsafe {
            sandbox_free_error(error);
        }
        return Err(message);
    }
    // SAFETY: process-local priority and core-dump resource limits.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        libc::setrlimit(
            libc::RLIMIT_CORE,
            &libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            },
        );
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[test]
    fn cancellation_kills_and_reaps_a_stuck_decoder() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let source_file = file.reopen().unwrap();
        let mut command = Command::new("/bin/sleep");
        command.arg("60");
        let mut worker = Worker::start(command, source_file).unwrap();
        let pid = worker.child.id();
        let flag = cancel.clone();
        let trigger = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            flag.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        assert_eq!(
            worker.run(Operation::Info, &cancel).err().unwrap(),
            super::super::CANCELLED
        );
        assert!(worker.failed);
        trigger.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        // SAFETY: signal zero only checks that our child has been reaped.
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }
}
