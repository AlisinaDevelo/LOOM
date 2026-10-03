use std::{fs, path::PathBuf, time::Instant};

use loom_extraction::{
    ExtractionBudget, ExtractionError, ExtractionSupervisor, MediaKind, RunError,
};

fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loom-extractor"))
}

#[cfg(unix)]
fn fault(mode: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let pid = directory.path().join("pid");
    let source = directory.path().join("fault.rs");
    let executable = directory.path().join("fault");
    fs::write(
        &source,
        format!(
            "const PID_FILE: &str = {:?};\nconst MODE: &str = {mode:?};\nconst HELPER: &str = {:?};\n{}",
            pid.to_str().unwrap(),
            helper().to_str().unwrap(),
            include_str!("support/fault.rs")
        ),
    )
    .unwrap();
    let output = std::process::Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(std::process::Command::new(&executable)
        .arg("--warmup")
        .status()
        .unwrap()
        .success());
    assert!(!pid.exists(), "warmup must not count as fault entry");
    (directory, executable, pid)
}

#[cfg(unix)]
fn assert_reaped(pid_file: PathBuf) {
    let pid = fs::read_to_string(pid_file).expect("fixture must have entered main");
    let status = std::process::Command::new("/bin/ps")
        .args(["-p", &pid, "-o", "pid="])
        .output()
        .unwrap();
    assert!(status.stdout.is_empty(), "owned child was not reaped");
}

#[test]
fn real_helper_extracts_text_without_a_path_or_database() {
    let supervisor = ExtractionSupervisor::new(helper()).unwrap();
    let result = supervisor
        .extract(
            b"local\r\nsource evidence",
            MediaKind::Text,
            ExtractionBudget::for_media(MediaKind::Text),
            || Ok::<(), ()>(()),
        )
        .unwrap();
    assert_eq!(
        result.source,
        loom_extraction::SourceOutput::Text {
            text: "local\nsource evidence".into()
        }
    );
    assert!(result.metrics.address_space_limit_installed);
    assert!(result.metrics.peak_resident_bytes > 0);
}

#[cfg(unix)]
#[test]
fn hanging_child_is_killed_and_reaped_at_the_monotonic_deadline() {
    let (_directory, executable, pid_file) = fault("hang");
    let supervisor = ExtractionSupervisor::new(executable).unwrap();
    let mut budget = ExtractionBudget::for_media(MediaKind::Text);
    budget.wall_ms = 1500;
    let started = Instant::now();
    let error = supervisor
        .extract(b"rights-clean marker", MediaKind::Text, budget, || {
            Ok::<(), ()>(())
        })
        .unwrap_err();
    assert!(
        matches!(error, RunError::Extraction(ExtractionError::WallTime)),
        "{error:?}"
    );
    assert!(started.elapsed().as_secs() < 3);
    assert_reaped(pid_file);
}

#[cfg(unix)]
#[test]
fn allocation_crash_and_invalid_output_are_distinct_and_every_child_is_reaped() {
    for (mode, expected) in [
        ("memory", ExtractionError::Memory),
        ("crash", ExtractionError::ChildCrashed),
        (
            "bad-output",
            ExtractionError::Protocol("wrong frame magic/version/kind".into()),
        ),
        ("oversize-output", ExtractionError::OutputLimit),
        (
            "unavailable",
            ExtractionError::OcrUnavailable("local provider is unavailable".into()),
        ),
    ] {
        let (_directory, executable, pid_file) = fault(mode);
        let supervisor = ExtractionSupervisor::new(executable).unwrap();
        let mut budget = ExtractionBudget::for_media(MediaKind::Text);
        budget.memory_bytes = 64 * 1024 * 1024;
        let error = supervisor
            .extract(b"private-synthetic-source", MediaKind::Text, budget, || {
                Ok::<(), ()>(())
            })
            .unwrap_err();
        assert!(
            matches!(error, RunError::Extraction(ref error) if *error == expected),
            "{mode}: {error:?}"
        );
        assert_reaped(pid_file);
    }
}

#[cfg(unix)]
#[test]
fn early_rejection_preserves_the_provider_error_when_the_request_pipe_closes() {
    let (_directory, executable, pid_file) = fault("unavailable");
    let supervisor = ExtractionSupervisor::new(executable).unwrap();
    // A helper may decline before reading input (for example, unavailable guards).
    // This request cannot fit in the unread pipe, so its writer sees a broken pipe.
    let error = supervisor
        .extract(
            &vec![b'p'; loom_extraction::MAX_INPUT_BYTES],
            MediaKind::Text,
            ExtractionBudget::for_media(MediaKind::Text),
            || Ok::<(), ()>(()),
        )
        .unwrap_err();
    assert!(
        matches!(error, RunError::Extraction(ExtractionError::OcrUnavailable(ref reason)) if reason == "local provider is unavailable"),
        "{error:?}"
    );
    assert_reaped(pid_file);
}

#[cfg(unix)]
#[test]
fn early_responses_cannot_bypass_exit_framing_or_request_completion_checks() {
    for (mode, expected) in [
        ("unavailable-crash", ExtractionError::ChildCrashed),
        ("unavailable-hang", ExtractionError::WallTime),
        (
            "unavailable-trailing",
            ExtractionError::Protocol("extra frame or trailing bytes".into()),
        ),
        (
            "early-success",
            ExtractionError::Protocol("truncated or unavailable transport".into()),
        ),
    ] {
        let (_directory, executable, pid_file) = fault(mode);
        let supervisor = ExtractionSupervisor::new(executable).unwrap();
        let mut budget = ExtractionBudget::for_media(MediaKind::Text);
        if mode == "unavailable-hang" {
            budget.wall_ms = 1500;
        }
        let error = supervisor
            .extract(
                &vec![b'p'; loom_extraction::MAX_INPUT_BYTES],
                MediaKind::Text,
                budget,
                || Ok::<(), ()>(()),
            )
            .unwrap_err();
        assert!(
            matches!(error, RunError::Extraction(ref error) if *error == expected),
            "{mode}: {error:?}"
        );
        assert_reaped(pid_file);
    }
}

#[cfg(unix)]
#[test]
fn success_framing_does_not_authorize_forged_metrics_or_wrong_media() {
    for (mode, expected) in [
        ("metrics-wall", ExtractionError::WallTime),
        ("metrics-cpu", ExtractionError::CpuTime),
        ("metrics-memory", ExtractionError::Memory),
        ("metrics-guard", ExtractionError::GuardUnavailable),
        ("metrics-interval", ExtractionError::GuardUnavailable),
        ("metrics-zero", ExtractionError::GuardUnavailable),
        (
            "media-mismatch",
            ExtractionError::Protocol("invalid evidence geometry or media".into()),
        ),
    ] {
        let (_directory, executable, pid_file) = fault(mode);
        let supervisor = ExtractionSupervisor::new(executable).unwrap();
        let error = supervisor
            .extract(
                b"rights-clean marker",
                MediaKind::Text,
                ExtractionBudget::for_media(MediaKind::Text),
                || Ok::<(), ()>(()),
            )
            .unwrap_err();
        assert!(
            matches!(error, RunError::Extraction(ref error) if *error == expected),
            "{mode}: {error:?}"
        );
        assert_reaped(pid_file);
    }
}

#[cfg(unix)]
#[test]
fn prelaunch_interruption_prevents_spawn_and_live_interruption_kills_before_return() {
    let (_directory, executable, pid_file) = fault("hang");
    let supervisor = ExtractionSupervisor::new(executable).unwrap();
    let error = supervisor
        .extract(
            b"evidence",
            MediaKind::Text,
            ExtractionBudget::for_media(MediaKind::Text),
            || Err("prelaunch revoked"),
        )
        .unwrap_err();
    assert!(matches!(error, RunError::Interrupted("prelaunch revoked")));
    assert!(!pid_file.exists());
    let error = supervisor
        .extract(
            b"evidence",
            MediaKind::Text,
            ExtractionBudget::for_media(MediaKind::Text),
            || {
                if pid_file.exists() {
                    Err("live revoked")
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
    assert!(matches!(error, RunError::Interrupted("live revoked")));
    assert_reaped(pid_file);
}

#[cfg(unix)]
#[test]
fn an_unwinding_probe_does_not_deadlock_pipe_threads_or_leave_a_child() {
    let (_directory, executable, pid_file) = fault("hang");
    let supervisor = ExtractionSupervisor::new(executable).unwrap();
    let started = Instant::now();
    let panic = std::panic::catch_unwind(|| {
        let _ = supervisor.extract(
            b"evidence",
            MediaKind::Text,
            ExtractionBudget::for_media(MediaKind::Text),
            || {
                assert!(!pid_file.exists(), "fixture probe panic");
                Ok::<(), ()>(())
            },
        );
    });
    assert!(panic.is_err());
    assert!(started.elapsed().as_secs() < 3);
    assert_reaped(pid_file);
}

#[cfg(unix)]
#[test]
fn helper_resolution_refuses_symlinks_relative_paths_and_untrusted_modes() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    let alias = directory.path().join("alias");
    symlink(helper(), &alias).unwrap();
    assert!(ExtractionSupervisor::new(alias).is_err());
    assert!(ExtractionSupervisor::new("loom-extractor").is_err());
    let copied = directory.path().join("copied");
    fs::copy(helper(), &copied).unwrap();
    fs::set_permissions(&copied, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(ExtractionSupervisor::new(copied).is_err());
}

#[cfg(unix)]
#[test]
fn shipped_helper_exits_when_parent_dies_before_metadata_even_without_input_eof() {
    use std::{
        process::{Child, Command, Stdio},
        time::Duration,
    };
    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (_directory, executable, pid_file) = fault("orphan-parent");
    let helper_pid = pid_file.with_extension("helper");
    let mut parent = Owned(
        Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    // Child::wait closes any stdin it still owns. Retain this writer separately
    // so that parent-loss, rather than EOF, is the helper's only exit signal.
    let input = parent
        .0
        .stdin
        .take()
        .expect("root must keep helper input open");
    let start = Instant::now();
    while !helper_pid.exists() {
        assert!(parent.0.try_wait().unwrap().is_none());
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = fs::read_to_string(helper_pid).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        !Command::new("/bin/ps")
            .args(["-p", &pid, "-o", "pid="])
            .output()
            .unwrap()
            .stdout
            .is_empty(),
        "helper exited before metadata fixture was established"
    );
    parent.0.kill().unwrap();
    parent.0.wait().unwrap();
    let death = Instant::now();
    loop {
        let state = Command::new("/bin/ps")
            .args(["-p", &pid, "-o", "stat="])
            .output()
            .unwrap()
            .stdout;
        if state.is_empty()
            || String::from_utf8_lossy(&state)
                .trim_start()
                .starts_with('Z')
        {
            break;
        }
        assert!(
            death.elapsed() < Duration::from_secs(3),
            "live orphan waiting for metadata"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    drop(input);
}
