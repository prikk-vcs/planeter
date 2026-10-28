//! IS-55: the outbound fetcher reads curl's body up to the size bound and no further. A stand-in `curl` prints the body.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use planeter_core::{
    CurlFetcher, EgressError, EgressGuard, FetchError, FetchLimits, FetchRequest, HttpsFetcher,
    ResolvedTarget,
};

struct Allow;
impl EgressGuard for Allow {
    fn check(&self, url: &str) -> Result<ResolvedTarget, EgressError> {
        Ok(ResolvedTarget {
            url: url.to_owned(),
            scheme: "https".into(),
            host: "idp.example".into(),
            port: 443,
            addrs: vec!["203.0.113.7:443".parse().unwrap()],
        })
    }
}

#[test]
fn a_body_is_read_up_to_the_bound_and_one_byte_more_is_too_large() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fetch-output-cap");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The body size comes from the URL's last path segment, as the stand-in's only input.
    let curl = dir.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\nfor a; do last=\"$a\"; done\nn=\"${last##*/}\"\nhead -c \"$n\" /dev/zero | tr '\\0' 'x'\n",
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fetcher = CurlFetcher::unconfined(Box::new(Allow))
        .with_curl_binary(&curl)
        .with_limits(FetchLimits {
            max_bytes: 1000,
            max_secs: 10,
        });
    let body = |n: u64| fetcher.fetch(&FetchRequest::get(format!("https://idp.example/{n}")));

    assert_eq!(body(1000).expect("exactly the bound").len(), 1000);
    assert!(matches!(body(1001), Err(FetchError::TooLarge)));
    // A gigabyte is cut at the bound: the call returns at once, it does not read it all.
    let started = std::time::Instant::now();
    assert!(matches!(body(1 << 30), Err(FetchError::TooLarge)));
    assert!(started.elapsed() < std::time::Duration::from_secs(8));
}
