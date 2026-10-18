//! RFC 011 D-5: **spawn and wait stay on one thread.**
//!
//! `bwrap --die-with-parent` is `PR_SET_PDEATHSIG` on the parent *thread*. Since `timeout` left the
//! vector, `bwrap` is the server's direct child, so if the thread that spawned it goes idle and the tokio
//! blocking pool expires it, a healthy call is killed (investigation I3: a 14 s call died at exactly
//! 10.0 s, the pool's default keep-alive).
//!
//! * the **control** shows the hazard exists in this environment (spawn on a pool thread, let it idle);
//! * the **pin** shows `Sandbox::run` — spawn and wait in one `spawn_blocking` closure, as
//!   `read_blocking` does — is immune, with a keep-alive far shorter than the call.
//!
//! Self-skips (printed reason) where the bubblewrap sandbox cannot run.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use planeter_prikk::Sandbox;
use planeter_prikk::sandbox::CallLimits;

fn root(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sandbox_works() -> bool {
    // Not `root(..)`: the two tests run in parallel and `root` deletes and recreates its directory.
    let here = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    matches!(
        Sandbox::default().run(OsStr::new("/usr/bin/true"), &here, &[], CallLimits::NONE),
        Ok(r) if r.status.success()
    )
}

fn runtime(keep_alive: Duration) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_keep_alive(keep_alive)
        .enable_time()
        .build()
        .unwrap()
}

#[test]
fn control_a_child_spawned_on_an_idle_pool_thread_dies_when_that_thread_expires() {
    if !sandbox_works() {
        eprintln!("skipping: the bubblewrap sandbox cannot run here");
        return;
    }
    let rt = runtime(Duration::from_millis(300));
    let r = root("pin-control");
    let started = Instant::now();
    let status = rt.block_on(async {
        // Spawn on a pool thread and return: that thread is now idle and will expire.
        let mut child = tokio::task::spawn_blocking(move || {
            Sandbox::default()
                .command_with(OsStr::new("/usr/bin/sleep"), &r, &["3"], CallLimits::NONE)
                .spawn()
                .unwrap()
        })
        .await
        .unwrap();
        // Wait from a plain thread, so the pool thread that spawned the child stays idle (a second
        // `spawn_blocking` would simply reuse it and hide the hazard).
        std::thread::spawn(move || child.wait().unwrap())
            .join()
            .unwrap()
    });
    assert!(
        !status.success() && started.elapsed() < Duration::from_millis(2500),
        "the premise of the pin failed: the call survived its spawning thread ({status:?}, {:?})",
        started.elapsed()
    );
}

#[test]
fn a_call_longer_than_the_pool_keep_alive_completes_when_spawned_and_waited_in_one_closure() {
    if !sandbox_works() {
        eprintln!("skipping: the bubblewrap sandbox cannot run here");
        return;
    }
    let rt = runtime(Duration::from_millis(300));
    let r = root("pin-run");
    let started = Instant::now();
    let run = rt
        .block_on(async {
            tokio::task::spawn_blocking(move || {
                // Spawn and wait both happen here, on this one thread — see `Sandbox::run`.
                Sandbox::default().run(OsStr::new("/usr/bin/sleep"), &r, &["3"], CallLimits::NONE)
            })
            .await
            .unwrap()
        })
        .expect("spawn");
    assert!(run.status.success() && !run.timed_out, "{run:?}");
    assert!(
        started.elapsed() >= Duration::from_millis(2900),
        "the call must have run its full 3 s (ten times the keep-alive): {:?}",
        started.elapsed()
    );
}
