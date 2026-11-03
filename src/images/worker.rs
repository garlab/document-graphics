//! Blocking image worker client. Applications own queues, snapshots and caches.
use super::{
    Frame, Info, Pixels, Region, check,
    protocol::{self, Operation, Response},
};
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
#[cfg(target_os = "macos")]
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
            let _ = self.child.kill();
            let _ = self.child.wait();
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
            cancel,
            deadline: Instant::now() + TIMEOUT,
            memory_check: Instant::now(),
            #[cfg(target_os = "macos")]
            memory_limit: RSS_LIMIT,
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
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Interruptible<'a> {
    worker: &'a mut Worker,
    cancel: &'a AtomicBool,
    deadline: Instant,
    memory_check: Instant,
    #[cfg(target_os = "macos")]
    memory_limit: u64,
}
impl Interruptible<'_> {
    fn resources(&mut self, force: bool) -> io::Result<()> {
        check(self.cancel).map_err(io::Error::other)?;
        let now = Instant::now();
        if now >= self.deadline {
            return Err(io::Error::other(
                "Image decode exceeded 10 seconds; use Source/Binary or retry",
            ));
        }
        if force || now >= self.memory_check {
            self.memory_check = now + Duration::from_millis(20);
            #[cfg(target_os = "macos")]
            {
                let rss = crate::process::resident_bytes(self.worker.child.id())
                    .ok_or_else(|| io::Error::other("Cannot monitor image worker memory"))?;
                if rss > self.memory_limit {
                    return Err(io::Error::other(
                        "Image decoder exceeded 256 MiB; overview or Binary remains available",
                    ));
                }
            }
        }
        Ok(())
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
        crate::process::restrict_image_worker()?;
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
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[cfg(target_os = "macos")]
    #[test]
    fn memory_and_deadline_guards_interrupt_a_stuck_decoder() {
        let source = tempfile::NamedTempFile::new().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let source_file = source.reopen().unwrap();
        let mut command = Command::new("/bin/sleep");
        command.arg("60");
        let mut worker = Worker::start(command, source_file).unwrap();
        let mut reader = Interruptible {
            worker: &mut worker,
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(1),
            memory_check: Instant::now(),
            #[cfg(target_os = "macos")]
            memory_limit: RSS_LIMIT,
        };
        reader.resources(true).unwrap();
        reader.memory_limit = 1;
        assert!(
            reader
                .resources(true)
                .unwrap_err()
                .to_string()
                .contains("256 MiB")
        );
        reader.deadline = Instant::now();
        assert!(
            reader
                .resources(true)
                .unwrap_err()
                .to_string()
                .contains("10 seconds")
        );
    }
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
