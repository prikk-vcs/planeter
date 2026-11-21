//! Shared by the test binaries that write a stand-in script and run it at once.

/// A script that was just written can meet ETXTBSY ("text file busy") when another test thread's fork still holds the write handle for a moment:
/// run it until it runs (the project's pattern: `proposals_unit.rs`, `extract.rs`). Any other error is a real failure.
pub fn wait_until_runs(path: &std::path::Path) {
    for _ in 0..200 {
        match std::process::Command::new(path).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            Err(e) => panic!("{}: {e}", path.display()),
            Ok(_) => break,
        }
    }
}
