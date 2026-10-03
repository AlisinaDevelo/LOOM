// Native disposable fault process; this is never shipped as a provider binary.
use std::{fs, io::{self, Read, Write}, time::Duration};

fn response(body: &[u8]) {
    let mut frame = b"LOOMEXT\0".to_vec();
    frame.extend(1u16.to_le_bytes());
    frame.extend(3u16.to_le_bytes());
    frame.extend((body.len() as u32).to_le_bytes());
    frame.extend(body);
    io::stdout().write_all(&frame).unwrap();
}

fn main() {
    // Complete first-execution validation outside the measured fault deadline.
    // Warmup must not create the PID marker used to prove actual fault entry.
    if std::env::args().nth(1).as_deref() == Some("--warmup") {
        return;
    }
    fs::write(PID_FILE, std::process::id().to_string()).unwrap();
    match MODE {
        "hang" => std::thread::sleep(Duration::from_secs(10)),
        "orphan-parent" => {
            let child = std::process::Command::new(HELPER)
                .args(["--parent-pid", &std::process::id().to_string()])
                // Root retains this pipe's write end after killing this parent.
                // The helper must detect parent loss, not merely input EOF.
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                .spawn().unwrap();
            fs::write(format!("{PID_FILE}.helper"), child.id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(10));
        }
        "crash" => std::process::exit(23),
        "memory" => {
            let mut memory = vec![0u8; 320 * 1024 * 1024];
            for byte in &mut memory { *byte = 42; }
            std::hint::black_box(&memory);
            std::thread::sleep(Duration::from_secs(10));
        }
        "bad-output" => {
            io::stdout().write_all(b"invalid-protocol").unwrap();
        }
        "oversize-output" => {
            let mut header = b"LOOMEXT\0".to_vec();
            header.extend(1u16.to_le_bytes());
            header.extend(3u16.to_le_bytes());
            header.extend(u32::MAX.to_le_bytes());
            io::stdout().write_all(&header).unwrap();
            io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(10));
        }
        "unavailable" => {
            response(br#"{"kind":"failure","code":"ocr_unavailable"}"#);
        }
        "metrics-wall" | "metrics-cpu" | "metrics-memory" | "metrics-guard"
        | "metrics-interval" | "metrics-zero" | "media-mismatch" => {
            io::stdin().read_to_end(&mut Vec::new()).unwrap();
            let output = if MODE == "media-mismatch" {
                r#"{"kind":"pdf","page_count":1,"pages":[[1,"fixture"]],"warnings":[]}"#
            } else {
                r#"{"kind":"text","text":"fixture"}"#
            };
            let body = format!(
                r#"{{"kind":"success","output":{output},"metrics":{{"wall_ms":{},"cpu_ms":{},"peak_resident_bytes":{},"sample_interval_ms":{},"address_space_limit_installed":{}}}}}"#,
                if MODE == "metrics-wall" { 5000 } else { 1 },
                if MODE == "metrics-cpu" { 5001 } else { 1 },
                if MODE == "metrics-memory" { 268_435_457 } else if MODE == "metrics-zero" { 0 } else { 1 },
                if MODE == "metrics-interval" { 1 } else { 25 },
                MODE != "metrics-guard",
            );
            response(body.as_bytes());
        }
        _ => panic!("unknown fixture"),
    }
}
