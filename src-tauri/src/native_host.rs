//! Stdio entry point installed as the browser's native-messaging host.
//!
//! The host intentionally refuses to start without an explicit pairing secret and spool root.
//! Release packaging must provide those values from a Keychain-backed pairing flow; keeping the
//! boundary fail-closed is safer than silently accepting an unpaired browser.

use loom_browser_capture::{HostConfig, NativeHost};
use std::io::{self, BufReader, BufWriter};

fn main() {
    if let Err(error) = run() {
        eprintln!("LOOM native host stopped: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = HostConfig::from_env()?;
    let mut host = NativeHost::new(config);
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    host.run(&mut reader, &mut writer)?;
    Ok(())
}
