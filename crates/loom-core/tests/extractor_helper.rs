use std::{fs, path::PathBuf};

use loom_extraction::{ExtractionBudget, ExtractionSupervisor, MediaKind, SourceOutput};

fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loom-core-test-extractor"))
}

#[test]
fn cargo_builds_a_trusted_core_test_extractor() {
    let helper = helper();
    let expected_name = format!("loom-core-test-extractor{}", std::env::consts::EXE_SUFFIX);
    assert!(helper.is_absolute(), "Cargo helper path must be absolute");
    assert_eq!(
        helper.file_name().and_then(|name| name.to_str()),
        Some(expected_name.as_str()),
        "Cargo helper must use the test-only target name"
    );
    let metadata = fs::symlink_metadata(&helper).expect("Cargo must build the test helper");
    assert!(
        metadata.file_type().is_file(),
        "Cargo helper must be a regular executable, not a symlink"
    );
    ExtractionSupervisor::new(&helper)
        .expect("Cargo helper must satisfy the supervisor trust checks");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn core_test_extractor_uses_the_production_stdio_protocol() {
    let result = ExtractionSupervisor::new(helper())
        .expect("Cargo helper must satisfy the supervisor trust checks")
        .extract(
            b"core test helper",
            MediaKind::Text,
            ExtractionBudget::for_media(MediaKind::Text),
            || Ok::<(), ()>(()),
        )
        .expect("Cargo helper must serve the production extraction protocol");
    assert_eq!(
        result.source,
        SourceOutput::Text {
            text: "core test helper".into()
        }
    );
}
