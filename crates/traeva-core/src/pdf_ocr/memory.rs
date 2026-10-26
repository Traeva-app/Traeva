//! Process footprint includes Vision allocations outside the Rust allocator.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMemory {
    pub footprint_bytes: u64,
    /// Kernel-reported lifetime peak, retained without correction for evidence.
    pub kernel_lifetime_peak_bytes: u64,
    /// Maximum of this sample's current footprint and kernel lifetime peak.
    /// The fields can lag each other; this is an observed lower bound, not
    /// proof that an unsampled spike stayed below a hard memory limit.
    pub lifetime_peak_bytes: u64,
}

#[cfg(any(target_os = "macos", test))]
fn from_kernel_sample(footprint_bytes: u64, kernel_lifetime_peak_bytes: u64) -> ProcessMemory {
    ProcessMemory {
        footprint_bytes,
        kernel_lifetime_peak_bytes,
        lifetime_peak_bytes: footprint_bytes.max(kernel_lifetime_peak_bytes),
    }
}

/// Read before reaping the child. A missing sample is not zero usage;
/// the parent supervisor must reject an answer it cannot account for.
/// Compare current and normalized peak, retaining the maximum across samples.
pub fn process_memory(pid: u32) -> Option<ProcessMemory> {
    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
        let mut usage: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V4,
                (&mut usage as *mut libc::rusage_info_v4).cast(),
            )
        };
        (result == 0).then_some(from_kernel_sample(
            usage.ri_phys_footprint,
            usage.ri_lifetime_max_phys_footprint,
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// Call in the child before any OCR work. The parent passes its own PID
/// as a validated argument; do not infer it from a potentially orphaned child.
/// Parent exit is detected at startup and then every 50 ms.
pub fn child_parent_watchdog(parent_pid: u32) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let expected = i32::try_from(parent_pid)
            .ok()
            .filter(|pid| *pid > 1)
            .ok_or("Invalid OCR parent process.")?;
        if unsafe { libc::getppid() } != expected {
            return Err("The OCR parent process is no longer present.".into());
        }
        std::thread::Builder::new()
            .name("pdf-ocr-parent".into())
            .spawn(move || {
                loop {
                    // Reparenting changes getppid even if the old numeric PID
                    // is later reused by an unrelated process.
                    if unsafe { libc::getppid() } != expected {
                        unsafe { libc::_exit(125) };
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .map(|_| ())
            .map_err(|_| "The OCR parent watchdog could not start.".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = parent_pid;
        Err("Local PDF OCR is supported only on macOS.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lagged_kernel_peak_cannot_hide_the_observed_current_footprint() {
        let sample = from_kernel_sample(128, 96);
        assert_eq!(sample.footprint_bytes, 128);
        assert_eq!(sample.kernel_lifetime_peak_bytes, 96);
        assert_eq!(sample.lifetime_peak_bytes, 128);
        assert_eq!(from_kernel_sample(128, 256).lifetime_peak_bytes, 256);
    }

    #[test]
    fn invalid_process_ids_are_not_zero_memory_samples() {
        assert!(process_memory(0).is_none());
        assert!(process_memory(u32::MAX).is_none());
        assert!(process_memory(i32::MAX as u32).is_none());
    }

    #[test]
    fn invalid_or_wrong_parent_is_rejected_before_starting_a_watchdog() {
        assert!(child_parent_watchdog(0).is_err());
        assert!(child_parent_watchdog(1).is_err());
        assert!(child_parent_watchdog(u32::MAX).is_err());
        assert!(child_parent_watchdog(std::process::id()).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn current_process_has_a_real_footprint_sample() {
        let usage = process_memory(std::process::id()).unwrap();
        assert!(usage.footprint_bytes > 0);
        assert!(usage.lifetime_peak_bytes >= usage.footprint_bytes);
    }
}
