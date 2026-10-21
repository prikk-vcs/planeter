//! RFC 004 D-6: the two optional sandbox mounts, **artifact-in** (read-only) and **spool-out** (read-write).
//!
//! The vector tests run everywhere and never self-skip. The real-`bwrap` test runs a stand-in "prikk" (a shell
//! script) inside the production vector and shows what it can and cannot do; it self-skips, printing why, where
//! bubblewrap cannot run (no user namespaces).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use planeter_prikk::Sandbox;
use planeter_prikk::sandbox::{BwrapConfig, CallLimits, ExtraMounts, MountError};

fn args_of(sandbox: &Sandbox) -> Vec<String> {
    sandbox
        .command(OsStr::new("/opt/prikk"), Path::new("/repo"), &["status"])
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn with_mounts(m: ExtraMounts) -> Sandbox {
    Sandbox::Bwrap(BwrapConfig {
        mounts: m,
        ..BwrapConfig::default()
    })
}

// ----------------------------------------------------------------------------------------------- the vector

#[test]
fn no_mount_configured_is_the_default_vector_exactly() {
    assert_eq!(
        args_of(&with_mounts(ExtraMounts::default())),
        args_of(&Sandbox::default())
    );
}

#[test]
fn artifact_in_is_read_only_spool_out_is_read_write_and_both_come_after_the_repository() {
    let v = args_of(&with_mounts(ExtraMounts {
        artifact_in: Some("/srv/planeter/artifacts".into()),
        spool_out: Some("/srv/planeter/spool".into()),
    }));
    let default = args_of(&Sandbox::default());
    // The added arguments are exactly these six, inserted between the repository bind and `--chdir`.
    let at = default.iter().position(|a| a == "--chdir").unwrap();
    let mut expected = default.clone();
    expected.splice(
        at..at,
        [
            "--ro-bind",
            "/srv/planeter/artifacts",
            "/srv/planeter/artifacts",
            "--bind",
            "/srv/planeter/spool",
            "/srv/planeter/spool",
        ]
        .map(String::from),
    );
    assert_eq!(v, expected);
    // Only one `--bind` for the repository plus one for the spool; every other bind stays read-only.
    assert_eq!(v.iter().filter(|a| *a == "--bind").count(), 2);
}

#[test]
fn each_mount_alone_adds_only_its_own_three_arguments() {
    let default = args_of(&Sandbox::default());
    let a = args_of(&with_mounts(ExtraMounts {
        artifact_in: Some("/srv/a".into()),
        spool_out: None,
    }));
    assert_eq!(a.len(), default.len() + 3);
    assert!(a.windows(3).any(|w| w == ["--ro-bind", "/srv/a", "/srv/a"]));
    let s = args_of(&with_mounts(ExtraMounts {
        artifact_in: None,
        spool_out: Some("/srv/s".into()),
    }));
    assert_eq!(s.len(), default.len() + 3);
    assert!(s.windows(3).any(|w| w == ["--bind", "/srv/s", "/srv/s"]));
    assert!(!s.iter().any(|x| x == "/srv/a"));
}

#[test]
fn a_ceiling_still_goes_in_front_of_the_whole_vector_with_mounts() {
    let sb = with_mounts(ExtraMounts {
        artifact_in: Some("/srv/a".into()),
        spool_out: Some("/srv/s".into()),
    });
    let c = sb.command_with(
        OsStr::new("/opt/prikk"),
        Path::new("/repo"),
        &["status"],
        CallLimits::data_ceiling(1 << 30),
    );
    assert_eq!(c.get_program(), OsStr::new("prlimit"));
    let a: Vec<String> = c
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(&a[..3], ["--data=1073741824", "--", "bwrap"]);
    assert!(a.windows(3).any(|w| w == ["--ro-bind", "/srv/a", "/srv/a"]));
}

#[test]
fn mounts_are_validated() {
    let root = Path::new("/repos/alice/app");
    let ok = ExtraMounts {
        artifact_in: Some("/srv/a".into()),
        spool_out: Some("/srv/s".into()),
    };
    assert_eq!(ok.validate(root), Ok(()));
    assert_eq!(ExtraMounts::default().validate(root), Ok(()));
    let rel = ExtraMounts {
        artifact_in: Some("relative/dir".into()),
        spool_out: None,
    };
    assert!(matches!(
        rel.validate(root),
        Err(MountError::NotAbsolute(_))
    ));
    let dots = ExtraMounts {
        artifact_in: None,
        spool_out: Some("/srv/../etc".into()),
    };
    assert!(matches!(
        dots.validate(root),
        Err(MountError::NotAbsolute(_))
    ));
    for (a, s) in [
        ("/srv/x", "/srv/x"),
        ("/srv/x", "/srv/x/inner"),
        ("/srv/x/inner", "/srv/x"),
    ] {
        let m = ExtraMounts {
            artifact_in: Some(a.into()),
            spool_out: Some(s.into()),
        };
        assert!(
            matches!(m.validate(root), Err(MountError::Overlap(..))),
            "{a} {s}"
        );
    }
    // A mount inside the repository, or around it, is refused: the repository is the only writable place and
    // the spool must never be reachable through it.
    for inside in ["/repos/alice/app/spool", "/repos/alice", "/repos"] {
        let m = ExtraMounts {
            artifact_in: None,
            spool_out: Some(inside.into()),
        };
        assert!(
            matches!(m.validate(root), Err(MountError::Overlap(..))),
            "{inside}"
        );
    }
}

#[test]
fn run_refuses_a_bad_mount_before_spawning_anything() {
    let sb = with_mounts(ExtraMounts {
        artifact_in: None,
        spool_out: Some("/repos/alice/app/inside".into()),
    });
    let e = sb
        .run(
            OsStr::new("/bin/true"),
            Path::new("/repos/alice/app"),
            &[],
            CallLimits::NONE,
        )
        .unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    assert!(e.to_string().contains("must not overlap"), "{e}");
}

// ----------------------------------------------------------------------------------------------- real bwrap

fn bwrap_works() -> Result<(), String> {
    let out = Command::new("bwrap")
        .args([
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/lib",
            "/lib64",
            "--symlink",
            "usr/bin",
            "/bin",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--unshare-all",
            "/usr/bin/true",
        ])
        .output()
        .map_err(|e| format!("bwrap cannot be run: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "bwrap cannot create a sandbox here: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mounts-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A stand-in `prikk`: runs the operations given as `<op> <path>` pairs and prints `op path ok|fail` for each.
fn stand_in(dir: &Path) -> PathBuf {
    let script = dir.join("fake-prikk");
    std::fs::write(
        &script,
        r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
  op="$1"; path="$2"; shift 2
  case "$op" in
    read)   if cat "$path" >/dev/null 2>&1; then echo "$op $path ok"; else echo "$op $path fail"; fi ;;
    write)  if echo "written by the sandbox" > "$path" 2>/dev/null; then echo "$op $path ok"; else echo "$op $path fail"; fi ;;
    mkdir)  if mkdir -p "$path" 2>/dev/null && echo x > "$path/f" 2>/dev/null; then echo "$op $path ok"; else echo "$op $path fail"; fi ;;
  esac
done
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn outcomes(sb: &Sandbox, script: &Path, root: &Path, ops: &[(&str, &Path)]) -> Vec<String> {
    let owned: Vec<String> = ops
        .iter()
        .flat_map(|(op, p)| [op.to_string(), p.to_string_lossy().into_owned()])
        .collect();
    let args: Vec<&str> = owned.iter().map(String::as_str).collect();
    let run = sb
        .run(script.as_os_str(), root, &args, CallLimits::NONE)
        .expect("run the stand-in");
    assert!(
        run.status.success(),
        "stand-in failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn under_real_bwrap_artifact_in_reads_spool_out_writes_and_nothing_else_does() {
    if let Err(why) = bwrap_works() {
        eprintln!("skipping: {why}");
        return;
    }
    let base = scratch("real");
    let root = base.join("repos/alice/app");
    let sibling = base.join("repos/alice/other-app");
    let artifacts = base.join("artifacts");
    let spool = base.join("spool");
    let elsewhere = base.join("elsewhere");
    for d in [&root, &sibling, &artifacts, &spool, &elsewhere] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(artifacts.join("accepted.bundle"), b"an accepted artifact").unwrap();
    std::fs::write(sibling.join("secret"), b"another repository").unwrap();
    let script = stand_in(&base);

    let sb = with_mounts(ExtraMounts {
        artifact_in: Some(artifacts.clone()),
        spool_out: Some(spool.clone()),
    });
    let got = outcomes(
        &sb,
        &script,
        &root,
        &[
            // read first…
            ("read", &artifacts.join("accepted.bundle")),
            // …write second…
            ("write", &spool.join("export.bundle")),
            ("read", &spool.join("export.bundle")),
            // …and cannot write anywhere else.
            ("write", &artifacts.join("planted.bundle")),
            ("write", &artifacts.join("accepted.bundle")),
            ("write", &sibling.join("planted")),
            ("read", &sibling.join("secret")),
            ("mkdir", &base.join("repos/alice/new-sibling")),
            ("write", &elsewhere.join("planted")),
            ("write", &base.join("planted-at-the-top")),
            ("write", &root.join("in-the-repository")),
        ],
    );
    // Writes under a directory that bubblewrap created only to hold a bind (`mkdir` of a repository sibling, the
    // top of the scratch tree) land in the sandbox's private in-memory root and vanish with it; whether they
    // "succeed" inside is not the guarantee. The guarantee is checked on the host below: nothing arrived.
    let private = |op: &str, p: &Path| format!("{op} {}", p.display());
    let strip = |line: &String| line.rsplit_once(' ').map(|(a, _)| a.to_owned()).unwrap();
    let verdict = |line: &String| line.rsplit_once(' ').map(|(_, b)| b.to_owned()).unwrap();
    assert_eq!(got.len(), 11, "{got:#?}");
    let must = [
        (0, "ok"),
        (1, "ok"),
        (2, "ok"),
        (3, "fail"),
        (4, "fail"),
        (5, "fail"),
        (6, "fail"),
        (8, "fail"),
        (10, "ok"),
    ];
    for (i, want) in must {
        assert_eq!(verdict(&got[i]), want, "{}", got[i]);
    }
    assert_eq!(
        strip(&got[7]),
        private("mkdir", &base.join("repos/alice/new-sibling"))
    );
    assert_eq!(
        strip(&got[9]),
        private("write", &base.join("planted-at-the-top"))
    );

    // What the host sees afterwards: the spool file and the repository file exist; nothing else was created,
    // and the accepted artifact is untouched.
    assert_eq!(
        std::fs::read(spool.join("export.bundle")).unwrap(),
        b"written by the sandbox\n"
    );
    assert_eq!(
        std::fs::read(root.join("in-the-repository")).unwrap(),
        b"written by the sandbox\n"
    );
    assert_eq!(
        std::fs::read(artifacts.join("accepted.bundle")).unwrap(),
        b"an accepted artifact"
    );
    assert!(!artifacts.join("planted.bundle").exists());
    assert!(!sibling.join("planted").exists());
    assert!(!base.join("repos/alice/new-sibling").exists());
    assert!(!elsewhere.join("planted").exists());
    assert!(!base.join("planted-at-the-top").exists());
    assert_eq!(std::fs::read_dir(&sibling).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn without_the_mounts_the_same_directories_are_invisible_and_unwritable() {
    if let Err(why) = bwrap_works() {
        eprintln!("skipping: {why}");
        return;
    }
    let base = scratch("default");
    let root = base.join("repos/alice/app");
    let artifacts = base.join("artifacts");
    let spool = base.join("spool");
    for d in [&root, &artifacts, &spool] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(artifacts.join("accepted.bundle"), b"x").unwrap();
    let script = stand_in(&base);
    let got = outcomes(
        &Sandbox::default(),
        &script,
        &root,
        &[
            ("read", &artifacts.join("accepted.bundle")),
            ("write", &spool.join("export.bundle")),
        ],
    );
    assert_eq!(
        got,
        [
            format!("read {} fail", artifacts.join("accepted.bundle").display()),
            format!("write {} fail", spool.join("export.bundle").display()),
        ]
    );
    assert!(!spool.join("export.bundle").exists());
    let _ = std::fs::remove_dir_all(&base);
}
