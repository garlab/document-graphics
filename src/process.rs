//! Private process mechanisms. Renderers keep their own limits, errors and sandboxes.
use std::{
    process::Child,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Stop a direct child and reap it, including when it has already exited.
/// Workers do not start descendants. Call before joining any blocked I/O thread.
pub(crate) fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RequestError {
    Cancelled,
    TimedOut,
    #[cfg(target_os = "macos")]
    MemoryUnavailable,
    #[cfg(target_os = "macos")]
    MemoryExceeded,
}

/// Checks a request without owning its I/O, error presentation or failure cleanup.
/// Memory sampling is rate-limited; completion can force one final sample.
pub(crate) struct RequestGuard<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
    #[cfg(target_os = "macos")]
    next_memory_check: Instant,
    #[cfg(target_os = "macos")]
    memory_limit: u64,
}
impl<'a> RequestGuard<'a> {
    pub(crate) fn new(cancel: &'a AtomicBool, deadline: Instant, _memory_limit: u64) -> Self {
        Self {
            cancel,
            deadline,
            #[cfg(target_os = "macos")]
            next_memory_check: Instant::now(),
            #[cfg(target_os = "macos")]
            memory_limit: _memory_limit,
        }
    }
    /// A ready reply must not bypass cancellation or expiration while waiting.
    pub(crate) fn check_completion(&self) -> Result<(), RequestError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(RequestError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(RequestError::TimedOut);
        }
        Ok(())
    }
    pub(crate) fn check(&mut self, _pid: u32, _force_memory: bool) -> Result<(), RequestError> {
        self.check_completion()?;
        #[cfg(target_os = "macos")]
        {
            let now = Instant::now();
            if _force_memory || now >= self.next_memory_check {
                self.next_memory_check = now + Duration::from_millis(20);
                let bytes = resident_bytes(_pid).ok_or(RequestError::MemoryUnavailable)?;
                if bytes > self.memory_limit {
                    return Err(RequestError::MemoryExceeded);
                }
            }
        }
        Ok(())
    }
    pub(crate) fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn resident_bytes(pid: u32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    // SAFETY: proc_pidinfo initializes this exact structure on a full-size result.
    let count = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if count == size {
        Some(unsafe { info.assume_init() }.pti_resident_size)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_expiry_apply_even_to_completed_requests() {
        let cancel = AtomicBool::new(false);
        let mut guard =
            RequestGuard::new(&cancel, Instant::now() + Duration::from_secs(60), u64::MAX);
        assert!(guard.check_completion().is_ok());
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(guard.check_completion(), Err(RequestError::Cancelled));
        guard.deadline = Instant::now();
        assert_eq!(guard.check_completion(), Err(RequestError::Cancelled));
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(guard.check_completion(), Err(RequestError::TimedOut));
        assert_eq!(
            guard.check(std::process::id(), true),
            Err(RequestError::TimedOut)
        );
        assert_eq!(guard.remaining(), Duration::ZERO);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn memory_sampling_is_bounded_and_completion_can_force_a_sample() {
        let cancel = AtomicBool::new(false);
        let mut guard =
            RequestGuard::new(&cancel, Instant::now() + Duration::from_secs(60), u64::MAX);
        let pid = std::process::id();
        assert_eq!(guard.check(pid, true), Ok(()));
        guard.memory_limit = 1;
        guard.next_memory_check = Instant::now() + Duration::from_secs(60);
        assert_eq!(guard.check(pid, false), Ok(()));
        assert_eq!(guard.check(pid, true), Err(RequestError::MemoryExceeded));
        // Callers decide whether a missing process sample is fatal.
        assert_eq!(
            guard.check(u32::MAX, true),
            Err(RequestError::MemoryUnavailable)
        );
    }
}
