use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    extract_bytes, platform, protocol, ExtractionBudget, ExtractionError, ExtractionMetrics,
    FailureCode, HelperResponse, MediaKind, Result, WATCH_INTERVAL_MS,
};

pub(crate) const EXIT_WALL: i32 = 124;
pub(crate) const EXIT_MEMORY: i32 = 125;
pub(crate) const EXIT_ORPHAN: i32 = 126;
pub(crate) const EXIT_GUARD: i32 = 127;

/// Owns only this helper's lifetime/resource guards, never canonical authority.
struct HelperGuard {
    start: Instant,
    wall_ms: Arc<AtomicU64>,
    memory_bytes: Arc<AtomicU64>,
    sampled_peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    watchdog: Option<JoinHandle<()>>,
}

impl HelperGuard {
    fn start(expected_parent: u32) -> Result<Self> {
        if expected_parent <= 1 || platform::parent_pid() != expected_parent {
            return Err(ExtractionError::GuardUnavailable);
        }
        let bootstrap = ExtractionBudget::for_media(MediaKind::Png);
        // Installed after exec, before reading metadata/source bytes or calling providers.
        platform::install_limits(bootstrap)?;
        let start = Instant::now();
        let wall_ms = Arc::new(AtomicU64::new(u64::from(bootstrap.wall_ms)));
        let memory_bytes = Arc::new(AtomicU64::new(bootstrap.memory_bytes));
        let sampled_peak = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let guard_wall = Arc::clone(&wall_ms);
        let guard_memory = Arc::clone(&memory_bytes);
        let guard_peak = Arc::clone(&sampled_peak);
        let guard_stop = Arc::clone(&stop);
        let watchdog = thread::Builder::new()
            .name("extractor-watchdog".into())
            .spawn(move || {
                while !guard_stop.load(Ordering::Acquire) {
                    if platform::parent_pid() != expected_parent {
                        platform::exit_immediately(EXIT_ORPHAN);
                    }
                    if start.elapsed().as_millis() >= u128::from(guard_wall.load(Ordering::Acquire))
                    {
                        platform::exit_immediately(EXIT_WALL);
                    }
                    let sample = match platform::sample_process(std::process::id()) {
                        Ok(sample) => sample,
                        Err(_) => platform::exit_immediately(EXIT_GUARD),
                    };
                    guard_peak.fetch_max(sample.resident_bytes, Ordering::Relaxed);
                    if sample.resident_bytes.max(sample.footprint_bytes)
                        > guard_memory.load(Ordering::Acquire)
                    {
                        platform::exit_immediately(EXIT_MEMORY);
                    }
                    thread::sleep(Duration::from_millis(u64::from(WATCH_INTERVAL_MS)));
                }
            })
            .map_err(|_| ExtractionError::GuardUnavailable)?;
        Ok(Self {
            start,
            wall_ms,
            memory_bytes,
            sampled_peak,
            stop,
            watchdog: Some(watchdog),
        })
    }

    fn configure(&self, budget: ExtractionBudget) -> Result<()> {
        budget.validate()?;
        platform::install_limits(budget)?;
        self.memory_bytes
            .store(budget.memory_bytes, Ordering::Release);
        // This is always measured from process startup; metadata cannot extend it.
        self.wall_ms
            .store(u64::from(budget.wall_ms), Ordering::Release);
        Ok(())
    }

    fn metrics(&self) -> Result<ExtractionMetrics> {
        let (cpu_ms, peak) = platform::own_usage()?;
        Ok(ExtractionMetrics {
            wall_ms: u64::try_from(self.start.elapsed().as_millis())
                .map_err(|_| ExtractionError::GuardUnavailable)?,
            cpu_ms,
            peak_resident_bytes: peak.max(self.sampled_peak.load(Ordering::Relaxed)),
            sample_interval_ms: WATCH_INTERVAL_MS,
            address_space_limit_installed: true,
        })
    }
}

impl Drop for HelperGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.watchdog.take() {
            let _ = thread.join();
        }
    }
}

/// Dedicated one-shot entry point. The caller supplies only its expected parent PID.
/// stdout is framing only; the binary never accepts a path, URL, database or command.
pub fn serve_stdio(expected_parent: u32) -> Result<()> {
    let guard = match HelperGuard::start(expected_parent) {
        Ok(guard) => guard,
        Err(error) => {
            protocol::write_response(
                std::io::stdout().lock(),
                &HelperResponse::Failure {
                    code: FailureCode::from_error(&error),
                },
            )?;
            return Ok(());
        }
    };
    let mut input = std::io::stdin().lock();
    let response = (|| {
        let metadata = protocol::read_metadata(&mut input)?;
        guard.configure(metadata.budget)?;
        let bytes = protocol::read_input(&mut input)?;
        let output = extract_bytes(
            metadata.media,
            &bytes,
            metadata.budget.max_pdf_pages as usize,
            metadata.budget.max_image_pixels,
        )?;
        output.validate(metadata.media, metadata.budget)?;
        let metrics = guard.metrics()?;
        if metrics.peak_resident_bytes > metadata.budget.memory_bytes {
            return Err(ExtractionError::Memory);
        }
        Ok(HelperResponse::Success { output, metrics })
    })();
    let response = match response {
        Ok(response) => response,
        Err(error) => HelperResponse::Failure {
            code: FailureCode::from_error(&error),
        },
    };
    protocol::write_response(std::io::stdout().lock(), &response)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Read,
        path::Path,
        process::{Child, Command, Stdio},
    };

    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn guarded(mode: &str, ready: &Path) -> Owned {
        Owned(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "helper::tests::guarded_process_fixture",
                    "--nocapture",
                ])
                .env("LOOM_TEST_GUARD_MODE", mode)
                .env("LOOM_TEST_GUARD_READY", ready)
                .env("LOOM_TEST_GUARD_PARENT", std::process::id().to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    }

    fn wait_exit(child: &mut Owned) -> std::process::ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                start.elapsed() < Duration::from_secs(4),
                "guarded process did not exit"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn guarded_process_fixture() {
        let Ok(mode) = std::env::var("LOOM_TEST_GUARD_MODE") else {
            return;
        };
        let parent = std::env::var("LOOM_TEST_GUARD_PARENT")
            .unwrap()
            .parse()
            .unwrap();
        let inherited_address = if mode == "inherit" {
            Some(platform::impose_inherited_limits_for_test())
        } else {
            None
        };
        let guard = HelperGuard::start(parent).unwrap();
        if let Some(address) = inherited_address {
            assert_eq!(
                platform::cpu_limits_for_test().0,
                1,
                "bootstrap loosened inherited CPU limit"
            );
            assert_eq!(
                platform::address_soft_limit_for_test(),
                address,
                "bootstrap loosened inherited address-space limit"
            );
            guard
                .configure(ExtractionBudget::for_media(MediaKind::Text))
                .unwrap();
            assert_eq!(
                platform::cpu_limits_for_test().0,
                1,
                "media configuration loosened inherited CPU limit"
            );
            assert_eq!(platform::address_soft_limit_for_test(), address);
            return;
        }
        let mut budget = ExtractionBudget::for_media(MediaKind::Text);
        budget.cpu_seconds = 1;
        budget.memory_bytes = 64 * 1024 * 1024;
        guard.configure(budget).unwrap();
        let (soft, hard) = platform::cpu_limits_for_test();
        assert_eq!(soft, 1);
        assert!(
            hard > soft,
            "hard CPU limit must leave time for SIGXCPU classification"
        );
        fs::write(
            std::env::var_os("LOOM_TEST_GUARD_READY").unwrap(),
            std::process::id().to_string(),
        )
        .unwrap();
        if mode == "cpu" {
            let mut n = 0u64;
            loop {
                n = std::hint::black_box(n).wrapping_add(1);
            }
        }
        if mode == "memory" {
            let mut memory = vec![0u8; 96 * 1024 * 1024];
            memory.fill(42);
            std::hint::black_box(&memory);
            thread::sleep(Duration::from_secs(10));
            return;
        }
        loop {
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn real_post_exec_cpu_guard_has_a_classifiable_exhaustion_exit() {
        use std::os::unix::process::ExitStatusExt;
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("ready");
        let mut child = guarded("cpu", &ready);
        let status = wait_exit(&mut child);
        let mut diagnostics = String::new();
        child
            .0
            .stdout
            .take()
            .unwrap()
            .take(4096)
            .read_to_string(&mut diagnostics)
            .unwrap();
        child
            .0
            .stderr
            .take()
            .unwrap()
            .take(4096)
            .read_to_string(&mut diagnostics)
            .unwrap();
        assert!(
            ready.exists(),
            "resource fixture failed before provider entry: {status:?}: {diagnostics}"
        );
        assert_eq!(status.signal(), Some(libc::SIGXCPU), "{status:?}");
    }

    #[test]
    fn process_guards_never_loosen_inherited_soft_cpu_or_address_space_limits() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = guarded("inherit", &directory.path().join("ready"));
        let status = wait_exit(&mut child);
        let mut diagnostics = String::new();
        child
            .0
            .stdout
            .take()
            .unwrap()
            .take(4096)
            .read_to_string(&mut diagnostics)
            .unwrap();
        child
            .0
            .stderr
            .take()
            .unwrap()
            .take(4096)
            .read_to_string(&mut diagnostics)
            .unwrap();
        assert!(status.success(), "{status:?}: {diagnostics}");
    }

    #[test]
    fn real_helper_watchdog_terminates_allocation_overrun() {
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("ready");
        let mut child = guarded("memory", &ready);
        let status = wait_exit(&mut child);
        assert!(
            ready.exists(),
            "resource fixture failed before provider entry: {status:?}"
        );
        assert_eq!(status.code(), Some(EXIT_MEMORY), "{status:?}");
    }

    #[test]
    fn orphan_parent_fixture() {
        let Some(ready) = std::env::var_os("LOOM_TEST_ORPHAN_READY") else {
            return;
        };
        let mut child = guarded("orphan", Path::new(&ready));
        let start = Instant::now();
        while !Path::new(&ready).exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "orphan child failed at startup"
            );
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(10));
        }
        // Only this fault fixture deliberately abandons a child. Production supervisor
        // cleanup is separately tested; killing this parent must trigger the helper guard.
        std::mem::forget(child);
        thread::sleep(Duration::from_secs(10));
    }

    #[test]
    fn helper_watchdog_exits_after_actual_parent_process_death() {
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("orphan-ready");
        let mut parent = Owned(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "helper::tests::orphan_parent_fixture",
                    "--nocapture",
                ])
                .env("LOOM_TEST_ORPHAN_READY", &ready)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let start = Instant::now();
        while !ready.exists() {
            assert!(
                parent.0.try_wait().unwrap().is_none(),
                "parent fixture failed before child entry"
            );
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(10));
        }
        let pid = fs::read_to_string(ready).unwrap();
        parent.0.kill().unwrap();
        parent.0.wait().unwrap();
        let death = Instant::now();
        loop {
            let alive = Command::new("/bin/ps")
                .args(["-p", &pid, "-o", "stat="])
                .output()
                .unwrap()
                .stdout;
            // Only the direct owner can reap; a terminated orphan zombie has no
            // live provider work and may await init's reaper on some test hosts.
            if alive.is_empty()
                || String::from_utf8_lossy(&alive)
                    .trim_start()
                    .starts_with('Z')
            {
                break;
            }
            assert!(
                death.elapsed() < Duration::from_secs(3),
                "orphan exceeded guard lifetime"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}
