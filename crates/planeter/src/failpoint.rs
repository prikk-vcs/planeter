//! The delivery engine's **test hook** (RFC 008 handoff 2, T5). Compiled into **debug builds only** (`#[cfg(debug_assertions)]` on the
//! module): the release server has no such code, and the ENF-2 script fails if the release binary contains the variable's name.
//! It is not an operator's setting, so it is not in the usage text.

use std::sync::Arc;

/// What the delivery engine calls at each named point of an install.
pub type Failpoint = Arc<dyn Fn(&str) + Send + Sync>;

/// `PLANETER_FAILPOINT=<point>=<file>` makes the delivery engine write `<file>` and then stop forever when it reaches the named point, so
/// that a test can SIGKILL the real process exactly there. The points are the engine's (`planeter_transport::delivery`, `with_failpoint`).
pub fn from_env() -> Option<Failpoint> {
    let spec = std::env::var("PLANETER_FAILPOINT").ok()?;
    let (point, file) = spec.split_once('=')?;
    let (point, file) = (point.to_owned(), file.to_owned());
    Some(Arc::new(move |name: &str| {
        if name == point {
            let _ = std::fs::write(&file, name);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    }))
}
