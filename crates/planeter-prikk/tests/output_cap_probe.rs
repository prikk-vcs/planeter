//! IS-55: the start-up probe (`prikk --version`) is read up to 64 KiB and no further; a binary that prints without end is
//! `Unreadable`, promptly (the probe used to wait for the end of the output).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use planeter_prikk::cli::{BinaryProbe, probe_binary};

fn have(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn a_binary_that_prints_without_end_is_unreadable_and_a_normal_one_is_read() {
    if !have(planeter_prikk::sandbox::PRLIMIT) {
        eprintln!("skipping: needs prlimit");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("output-cap-probe");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, body: &str| {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };
    let endless = write("endless", "#!/bin/sh\nexec yes 'prikk 0.54.1'\n");
    let on_stderr = write("on-stderr", "#!/bin/sh\nexec yes 'x' >&2\n");
    let normal = write("normal", "#!/bin/sh\necho 'prikk 0.54.1'\n");
    // Let the files settle (a parallel fork can hold one open for writing for a moment: ETXTBSY).
    std::thread::sleep(Duration::from_millis(50));

    for (name, bin) in [("stdout", &endless), ("stderr", &on_stderr)] {
        let started = Instant::now();
        match probe_binary(bin.as_os_str()) {
            BinaryProbe::Unreadable(why) => assert!(why.contains("without end"), "{name}: {why}"),
            other => panic!("{name}: expected Unreadable, got {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{name}");
    }
    assert!(matches!(
        probe_binary(normal.as_os_str()),
        BinaryProbe::Supported(_)
    ));
}
