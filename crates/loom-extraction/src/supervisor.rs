use std::{
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::{
    platform, protocol, ExtractionBudget, ExtractionError, ExtractionMetrics, HelperResponse,
    MediaKind, RequestMetadata, Result, SourceOutput, MAX_INPUT_BYTES, WATCH_INTERVAL_MS,
};

#[derive(Debug, Clone)]
pub struct ExtractionSupervisor {
    executable: PathBuf,
}

#[derive(Debug)]
pub enum RunError<E> {
    Extraction(ExtractionError),
    Interrupted(E),
}

#[derive(Debug)]
pub struct SupervisedOutput {
    pub source: SourceOutput,
    pub metrics: ExtractionMetrics,
}

impl ExtractionSupervisor {
    pub fn new(executable: impl AsRef<Path>) -> Result<Self> {
        let executable = executable.as_ref();
        if !executable.is_absolute()
            || executable.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return Err(ExtractionError::HelperUnavailable);
        }
        validate_executable(executable)?;
        Ok(Self {
            executable: executable.to_owned(),
        })
    }

    /// No PATH lookup or environment override. Release/install must include both binaries.
    pub fn adjacent() -> Result<Self> {
        let executable = std::env::current_exe().map_err(|_| ExtractionError::HelperUnavailable)?;
        let directory = executable
            .parent()
            .ok_or(ExtractionError::HelperUnavailable)?;
        Self::new(directory.join(if cfg!(windows) {
            "loom-extractor.exe"
        } else {
            "loom-extractor"
        }))
    }

    pub fn extract<E>(
        &self,
        bytes: &[u8],
        media: MediaKind,
        budget: ExtractionBudget,
        mut probe: impl FnMut() -> std::result::Result<(), E>,
    ) -> std::result::Result<SupervisedOutput, RunError<E>> {
        let rejected = RunError::Extraction;
        budget.validate().map_err(rejected)?;
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(rejected(ExtractionError::OutputLimit));
        }
        if !cfg!(any(target_os = "macos", target_os = "linux")) {
            return Err(rejected(ExtractionError::GuardUnavailable));
        }
        validate_executable(&self.executable).map_err(rejected)?;
        probe().map_err(RunError::Interrupted)?;
        let start = Instant::now();
        let deadline = Duration::from_millis(u64::from(budget.wall_ms));
        // No shell, inherited environment, source-working-directory, or pre_exec.
        // The trusted helper installs OS limits itself, after exec and before input.
        let child = Command::new(&self.executable)
            .args(["--parent-pid", &std::process::id().to_string()])
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| rejected(ExtractionError::Launch))?;
        thread::scope(|scope| {
            // Construct inside the scope: cleanup precedes joining blocked pipe threads.
            let mut child = OwnedChild(child);
            let stdin = child
                .0
                .stdin
                .take()
                .ok_or_else(|| rejected(ExtractionError::Launch))?;
            let stdout = child
                .0
                .stdout
                .take()
                .ok_or_else(|| rejected(ExtractionError::Launch))?;
            let (write_tx, write_rx) = mpsc::sync_channel(1);
            let (read_tx, read_rx) = mpsc::sync_channel(1);
            thread::Builder::new()
                .name("extractor-input".into())
                .spawn_scoped(scope, move || {
                    let result =
                        protocol::write_request(stdin, &RequestMetadata { media, budget }, bytes);
                    let _ = write_tx.send(result);
                    // Closing stdin is required by the one-request protocol.
                })
                .map_err(|_| rejected(ExtractionError::Launch))?;
            thread::Builder::new()
                .name("extractor-output".into())
                .spawn_scoped(scope, move || {
                    let _ = read_tx.send(protocol::read_response(stdout));
                })
                .map_err(|_| rejected(ExtractionError::Launch))?;
            let mut response = None;
            let mut input_result = None;
            let mut status = None;
            let mut sampled_peak = 0;
            loop {
                probe().map_err(RunError::Interrupted)?;
                if start.elapsed() >= deadline {
                    return Err(rejected(ExtractionError::WallTime));
                }
                if status.is_none() {
                    status = child
                        .0
                        .try_wait()
                        .map_err(|_| rejected(ExtractionError::ChildCrashed))?;
                }
                if let Some(exit) = status {
                    if !exit.success() {
                        return Err(rejected(exit_error(exit)));
                    }
                } else {
                    match platform::sample_process(child.0.id()) {
                        Ok(sample) => {
                            sampled_peak = sampled_peak.max(sample.resident_bytes);
                            if sample.resident_bytes.max(sample.footprint_bytes)
                                > budget.memory_bytes
                            {
                                return Err(rejected(ExtractionError::Memory));
                            }
                        }
                        Err(error) => {
                            // A process may exit between try_wait and the OS sample.
                            status = child
                                .0
                                .try_wait()
                                .map_err(|_| rejected(ExtractionError::ChildCrashed))?;
                            if status.is_none() {
                                return Err(rejected(error));
                            }
                            continue;
                        }
                    }
                }
                if response.is_none() {
                    match read_rx.try_recv() {
                        Ok(value) => response = Some(value),
                        Err(mpsc::TryRecvError::Disconnected) => {
                            return Err(rejected(ExtractionError::ChildCrashed))
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                }
                if input_result.is_none() {
                    match write_rx.try_recv() {
                        Ok(value) => input_result = Some(value),
                        Err(mpsc::TryRecvError::Disconnected) => {
                            return Err(rejected(ExtractionError::ChildCrashed))
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                }
                if let Some(Err(error)) = response.as_ref() {
                    return Err(rejected(error.clone()));
                }
                if status.is_some() && response.is_some() && input_result.is_some() {
                    input_result.take().unwrap().map_err(rejected)?;
                    return match response.take().unwrap().map_err(rejected)? {
                        HelperResponse::Failure { code } => Err(rejected(code.error())),
                        HelperResponse::Success {
                            output,
                            mut metrics,
                        } => {
                            output.validate(media, budget).map_err(rejected)?;
                            if metrics.sample_interval_ms != WATCH_INTERVAL_MS
                                || !metrics.address_space_limit_installed
                                || metrics.peak_resident_bytes == 0
                            {
                                return Err(rejected(ExtractionError::GuardUnavailable));
                            }
                            if metrics.wall_ms >= u64::from(budget.wall_ms) {
                                return Err(rejected(ExtractionError::WallTime));
                            }
                            if metrics.cpu_ms > u64::from(budget.cpu_seconds) * 1000 {
                                return Err(rejected(ExtractionError::CpuTime));
                            }
                            metrics.peak_resident_bytes =
                                metrics.peak_resident_bytes.max(sampled_peak);
                            if metrics.peak_resident_bytes > budget.memory_bytes {
                                return Err(rejected(ExtractionError::Memory));
                            }
                            finalize_output(output, metrics, start, deadline).map_err(rejected)
                        }
                    };
                }
                thread::sleep(Duration::from_millis(u64::from(WATCH_INTERVAL_MS)));
            }
        })
    }
}

fn finalize_output(
    source: SourceOutput,
    mut metrics: ExtractionMetrics,
    start: Instant,
    deadline: Duration,
) -> Result<SupervisedOutput> {
    let elapsed = start.elapsed();
    if elapsed >= deadline {
        return Err(ExtractionError::WallTime);
    }
    metrics.wall_ms =
        u64::try_from(elapsed.as_millis()).map_err(|_| ExtractionError::GuardUnavailable)?;
    Ok(SupervisedOutput { source, metrics })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_validated_success_response_cannot_cross_the_final_wall_deadline() {
        let deadline = Duration::from_millis(25);
        let start = Instant::now().checked_sub(deadline).unwrap();
        let result = finalize_output(
            SourceOutput::Text {
                text: "validated source".into(),
            },
            ExtractionMetrics {
                wall_ms: 1,
                ..Default::default()
            },
            start,
            deadline,
        );
        assert!(
            matches!(result, Err(ExtractionError::WallTime)),
            "{result:?}"
        );
    }
}

fn exit_error(status: ExitStatus) -> ExtractionError {
    match status.code() {
        Some(crate::helper::EXIT_WALL) => ExtractionError::WallTime,
        Some(crate::helper::EXIT_MEMORY) => ExtractionError::Memory,
        Some(crate::helper::EXIT_GUARD) => ExtractionError::GuardUnavailable,
        _ => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if status.signal() == Some(libc::SIGXCPU) {
                    return ExtractionError::CpuTime;
                }
            }
            ExtractionError::ChildCrashed
        }
    }
}

fn validate_executable(path: &Path) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| ExtractionError::HelperUnavailable)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ExtractionError::HelperUnavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = crate::platform::effective_uid();
        if ![0, uid].contains(&metadata.uid())
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o111 == 0
        {
            return Err(ExtractionError::HelperUnavailable);
        }
    }
    Ok(())
}

/// Scope owns the Child before any I/O threads. Drop kills/reaps on every exit,
/// including an unwinding probe; it runs before scoped threads are joined.
struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
