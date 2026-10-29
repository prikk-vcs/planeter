#![forbid(unsafe_code)]
//! `planeter-image-identity platform INDEX_FILE ARCH` prints the digest of the index's `linux/ARCH` manifest;
//! `planeter-image-identity config MANIFEST_FILE` prints the manifest's config digest;
//! `planeter-image-identity same RUN_ID PUSHED_ID` exits 0 when they are one digest.
//! Exit 0 on success, 1 when the registry's answer says the image is not the one that was run, 2 on input it cannot read.

use std::process::ExitCode;

use planeter_image_identity::{Failure, config_digest, platform_manifest, same};

fn read(path: &str) -> Result<String, Failure> {
    std::fs::read_to_string(path).map_err(|e| Failure::Usage(format!("cannot read {path}: {e}")))
}

fn run(args: &[String]) -> Result<Option<String>, Failure> {
    match args {
        [cmd, index, arch] if cmd == "platform" => platform_manifest(&read(index)?, arch).map(Some),
        [cmd, manifest] if cmd == "config" => config_digest(&read(manifest)?).map(Some),
        [cmd, run, pushed] if cmd == "same" => same(run, pushed).map(|()| None),
        _ => Err(Failure::Usage(
            "usage: planeter-image-identity platform INDEX_FILE ARCH | config MANIFEST_FILE | same RUN_ID PUSHED_ID".to_owned(),
        )),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(Some(digest)) => {
            println!("{digest}");
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("the pushed image is the image that was run");
            ExitCode::SUCCESS
        }
        Err(f @ Failure::Mismatch(_)) => {
            eprintln!("planeter-image-identity: {f}");
            ExitCode::from(1)
        }
        Err(f) => {
            eprintln!("planeter-image-identity: {f}");
            ExitCode::from(2)
        }
    }
}
