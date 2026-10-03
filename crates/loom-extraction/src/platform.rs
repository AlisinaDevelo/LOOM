//! The reviewed OS-only unsafe boundary. No source, database, or protocol parsing here.

use crate::{ExtractionBudget, ExtractionError, Result};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProcessSample {
    pub resident_bytes: u64,
    pub footprint_bytes: u64,
}

#[cfg(unix)]
pub(crate) fn effective_uid() -> u32 {
    // SAFETY: geteuid has no pointer arguments or memory effects visible to Rust.
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
pub(crate) fn parent_pid() -> u32 {
    // SAFETY: getppid has no pointer arguments; its positive PID fits u32.
    unsafe { libc::getppid() as u32 }
}

#[cfg(not(unix))]
pub(crate) fn parent_pid() -> u32 {
    0
}

#[cfg(target_os = "linux")]
type LimitResource = libc::__rlimit_resource_t;
#[cfg(all(unix, not(target_os = "linux")))]
type LimitResource = libc::c_int;

#[cfg(unix)]
fn lower_limit(resource: LimitResource, requested: u64) -> Result<()> {
    let mut old = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: old is a live correctly aligned rlimit output object, unique to this call.
    if unsafe { libc::getrlimit(resource, &mut old) } != 0 {
        return Err(ExtractionError::GuardUnavailable);
    }
    let requested =
        libc::rlim_t::try_from(requested).map_err(|_| ExtractionError::GuardUnavailable)?;
    // Equal CPU soft/hard limits can kill with SIGKILL before SIGXCPU can identify
    // exhaustion. Keep one second of hard-limit slack; stricter inherited CPU
    // limits that cannot provide that distinction fail closed.
    let hard_requested = if resource == libc::RLIMIT_CPU {
        requested
            .checked_add(1)
            .ok_or(ExtractionError::GuardUnavailable)?
    } else {
        requested
    };
    let soft = requested.min(old.rlim_cur).min(old.rlim_max);
    let hard = hard_requested.min(old.rlim_max);
    if resource == libc::RLIMIT_CPU && hard <= soft {
        return Err(ExtractionError::GuardUnavailable);
    }
    let new = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    // SAFETY: new is a fully initialized immutable rlimit. This runs after exec,
    // only in the one-shot helper, never in a parent pre_exec/fork callback.
    if unsafe { libc::setrlimit(resource, &new) } != 0 {
        return Err(ExtractionError::GuardUnavailable);
    }
    let mut actual = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: same owned output-object contract as the getrlimit above.
    if unsafe { libc::getrlimit(resource, &mut actual) } != 0
        || actual.rlim_cur > requested
        || actual.rlim_cur > old.rlim_cur
        || actual.rlim_max > hard_requested
        || (resource == libc::RLIMIT_CPU && actual.rlim_max <= actual.rlim_cur)
    {
        return Err(ExtractionError::GuardUnavailable);
    }
    Ok(())
}

#[cfg(all(test, unix))]
pub(crate) fn cpu_limits_for_test() -> (u64, u64) {
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: live uniquely borrowed correctly sized output object.
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CPU, &mut limits) }, 0);
    (limits.rlim_cur, limits.rlim_max)
}

#[cfg(all(test, unix))]
pub(crate) fn impose_inherited_limits_for_test() -> u64 {
    let address_soft = if cfg!(target_os = "macos") {
        511 * 1024 * 1024 * 1024
    } else {
        512 * 1024 * 1024
    };
    for (resource, soft, hard) in [
        (libc::RLIMIT_CPU, 1, 240),
        (
            libc::RLIMIT_AS,
            address_soft,
            if cfg!(target_os = "macos") {
                512 * 1024 * 1024 * 1024
            } else {
                2 * 1024 * 1024 * 1024
            },
        ),
    ] {
        let limits = libc::rlimit {
            rlim_cur: soft,
            rlim_max: hard,
        };
        // SAFETY: initialized immutable numeric object, only in an owned child fixture.
        assert_eq!(
            unsafe { libc::setrlimit(resource, &limits) },
            0,
            "resource={resource}: {}",
            std::io::Error::last_os_error()
        );
    }
    address_soft
}

#[cfg(all(test, unix))]
pub(crate) fn address_soft_limit_for_test() -> u64 {
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: live uniquely borrowed correctly sized output object.
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limits) }, 0);
    limits.rlim_cur
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn install_limits(budget: ExtractionBudget) -> Result<()> {
    budget.validate()?;
    lower_limit(libc::RLIMIT_CORE, 0)?;
    lower_limit(libc::RLIMIT_CPU, u64::from(budget.cpu_seconds))?;
    lower_limit(libc::RLIMIT_AS, budget.address_space_bytes)?;
    // Reduce scheduler priority without changing the parent's priority or using
    // a post-fork callback. A stricter inherited priority remains acceptable.
    // SAFETY: PRIO_PROCESS with PID zero queries/affects this helper only; no pointers.
    let inherited = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
    if inherited < 10 && unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 10) } != 0 {
        return Err(ExtractionError::GuardUnavailable);
    }
    sample_process(std::process::id())?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn install_limits(_budget: ExtractionBudget) -> Result<()> {
    Err(ExtractionError::GuardUnavailable)
}

#[cfg(target_os = "macos")]
pub(crate) fn sample_process(pid: u32) -> Result<ProcessSample> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| ExtractionError::GuardUnavailable)?;
    if pid <= 1 {
        return Err(ExtractionError::GuardUnavailable);
    }
    // SAFETY: this C repr consists solely of integer fields, so all-zero is valid.
    // It is initialized even if the OS call fails; only successful calls are read.
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    // SAFETY: flavour V2 matches the correctly sized/aligned, uniquely borrowed
    // output object. The libproc ABI uses rusage_info_t* for this output buffer.
    let result = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            std::ptr::addr_of_mut!(info).cast::<libc::rusage_info_t>(),
        )
    };
    if result != 0 {
        return Err(ExtractionError::GuardUnavailable);
    }
    Ok(ProcessSample {
        resident_bytes: info.ri_resident_size,
        footprint_bytes: info.ri_phys_footprint,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn sample_process(pid: u32) -> Result<ProcessSample> {
    use std::io::Read;
    if pid <= 1 {
        return Err(ExtractionError::GuardUnavailable);
    }
    let mut bytes = String::new();
    std::fs::File::open(format!("/proc/{pid}/statm"))
        .map_err(|_| ExtractionError::GuardUnavailable)?
        .take(4096)
        .read_to_string(&mut bytes)
        .map_err(|_| ExtractionError::GuardUnavailable)?;
    if bytes.len() == 4096 {
        return Err(ExtractionError::GuardUnavailable);
    }
    let pages = bytes
        .split_whitespace()
        .nth(1)
        .and_then(|n| n.parse::<u64>().ok())
        .ok_or(ExtractionError::GuardUnavailable)?;
    // SAFETY: sysconf(_SC_PAGESIZE) has no pointer arguments or Rust aliasing effects.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = u64::try_from(page_size)
        .ok()
        .filter(|n| *n > 0)
        .ok_or(ExtractionError::GuardUnavailable)?;
    let resident_bytes = pages
        .checked_mul(page_size)
        .ok_or(ExtractionError::GuardUnavailable)?;
    Ok(ProcessSample {
        resident_bytes,
        footprint_bytes: resident_bytes,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn sample_process(_pid: u32) -> Result<ProcessSample> {
    Err(ExtractionError::GuardUnavailable)
}

#[cfg(unix)]
pub(crate) fn own_usage() -> Result<(u64, u64)> {
    // SAFETY: rusage is a C repr of numeric fields and timeval structs; zero is valid.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: the uniquely borrowed object is correctly sized and initialized.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return Err(ExtractionError::GuardUnavailable);
    }
    let millis = |time: libc::timeval| -> Result<u64> {
        let secs = u64::try_from(time.tv_sec).map_err(|_| ExtractionError::GuardUnavailable)?;
        let micros = u64::try_from(time.tv_usec).map_err(|_| ExtractionError::GuardUnavailable)?;
        secs.checked_mul(1000)
            .and_then(|n| n.checked_add(micros / 1000))
            .ok_or(ExtractionError::GuardUnavailable)
    };
    let cpu_ms = millis(usage.ru_utime)?
        .checked_add(millis(usage.ru_stime)?)
        .ok_or(ExtractionError::GuardUnavailable)?;
    let peak = u64::try_from(usage.ru_maxrss).map_err(|_| ExtractionError::GuardUnavailable)?;
    let peak = if cfg!(target_os = "macos") {
        peak
    } else {
        peak.checked_mul(1024)
            .ok_or(ExtractionError::GuardUnavailable)?
    };
    Ok((cpu_ms, peak))
}

#[cfg(not(unix))]
pub(crate) fn own_usage() -> Result<(u64, u64)> {
    Err(ExtractionError::GuardUnavailable)
}

/// Watchdog exit bypasses atexit/Objective-C cleanup locks. Only the helper calls this.
#[cfg(unix)]
pub(crate) fn exit_immediately(code: i32) -> ! {
    // SAFETY: _exit terminates the calling helper process and never returns. No
    // Rust-owned pointer crosses the ABI; there is no unwinding or shared parent heap.
    unsafe { libc::_exit(code) }
}

#[cfg(not(unix))]
pub(crate) fn exit_immediately(code: i32) -> ! {
    std::process::exit(code)
}
