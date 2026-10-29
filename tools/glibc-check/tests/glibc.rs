//! The check against the real tables: the version needs of the release binaries before and after the aws-lc switch (D-51), read from
//! the rehearsal artifacts' `readelf -V`. Planted cases both ways.

use planeter_glibc_check::{Version, base_version, check, parse_needs};

fn table(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/readelf-V-{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

#[test]
fn versions_compare_as_numbers_not_as_text() {
    assert!(v("2.38") > v("2.4"));
    assert!(v("2.10") > v("2.9"));
    assert!(v("2.2.5") > v("2.2"));
    assert_eq!(v("2.3"), v("2.3.0"));
    assert!(Version::parse("2").is_none() && Version::parse("x.y").is_none());
}

#[test]
fn the_base_comes_from_the_ldd_line_or_a_bare_version() {
    assert_eq!(
        base_version("ldd (Debian GLIBC 2.36-9+deb12u10) 2.36\nCopyright …"),
        Some(v("2.36"))
    );
    assert_eq!(
        base_version("ldd (Debian GLIBC 2.41-12) 2.41"),
        Some(v("2.41"))
    );
    assert_eq!(base_version("2.41"), Some(v("2.41")));
    assert_eq!(base_version("not a version"), None);
}

#[test]
fn the_x86_64_binary_before_the_switch_ran_on_bookworm_and_after_it_does_not() {
    let bookworm = v("2.36");
    let before = check(&parse_needs(&table("before-x86_64")), &bookworm);
    assert!(before.ok(), "{before}");
    assert_eq!(before.highest_strong, Some(v("2.34")));
    // The weak 2.39 need (Rust std's pidfd_spawnp) is above the base and is reported, not failed.
    assert_eq!(before.highest_weak, Some(v("2.39")));
    assert_eq!(before.weak_above.len(), 1);
    assert!(
        before.to_string().contains("NOTE: a WEAK need GLIBC_2.39"),
        "{before}"
    );

    let after = check(&parse_needs(&table("after-x86_64")), &bookworm);
    assert!(!after.ok(), "{after}");
    assert_eq!(after.highest_strong, Some(v("2.38")));
    assert_eq!(after.too_new.len(), 1);
    assert!(
        after
            .to_string()
            .contains("strictly needs GLIBC_2.38 of libc.so.6"),
        "{after}"
    );
}

#[test]
fn the_aarch64_binary_needed_2_39_even_before_the_switch() {
    let bookworm = v("2.36");
    for name in ["before-aarch64", "after-aarch64"] {
        let r = check(&parse_needs(&table(name)), &bookworm);
        assert!(!r.ok(), "{name}: {r}");
        assert_eq!(r.highest_strong, Some(v("2.39")), "{name}");
    }
}

#[test]
fn trixie_defines_everything_all_four_binaries_need() {
    for name in [
        "before-x86_64",
        "after-x86_64",
        "before-aarch64",
        "after-aarch64",
    ] {
        let r = check(&parse_needs(&table(name)), &v("2.41"));
        assert!(r.ok(), "{name}: {r}");
        assert!(r.weak_above.is_empty(), "{name}: nothing is above 2.41");
    }
}

#[test]
fn other_libraries_versions_are_not_glibc_versions() {
    let text = "  000000: Version: 1  File: libgcc_s.so.1  Cnt: 2\n  0x0010:   Name: GCC_4.2.0  Flags: none  Version: 3\n  0x0020: Version: 1  File: libc.so.6  Cnt: 2\n  0x0030:   Name: GLIBC_PRIVATE  Flags: none  Version: 4\n  0x0040:   Name: GLIBC_2.2.5  Flags: none  Version: 5\n";
    let needs = parse_needs(text);
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].version, v("2.2.5"));
    assert_eq!(needs[0].file, "libc.so.6");
}

#[test]
fn a_planted_strong_need_above_the_base_fails_and_a_weak_one_is_reported_not_failed() {
    let strong = "  0x0020: Version: 1  File: libc.so.6  Cnt: 1\n  0x0030:   Name: GLIBC_2.40  Flags: none  Version: 4\n";
    let r = check(&parse_needs(strong), &v("2.39"));
    assert!(!r.ok() && r.too_new.len() == 1, "{r}");
    let weak = "  0x0020: Version: 1  File: libc.so.6  Cnt: 1\n  0x0030:   Name: GLIBC_2.40  Flags: WEAK  Version: 4\n";
    let r = check(&parse_needs(weak), &v("2.39"));
    assert!(
        r.ok() && r.weak_above.len() == 1 && r.to_string().contains("NOTE"),
        "{r}"
    );
    // Exactly at the base is fine.
    assert!(check(&parse_needs(strong), &v("2.40")).ok());
}

#[test]
fn the_program_itself_is_run_on_a_real_binary_and_refuses_what_it_cannot_read() {
    use std::process::Command;
    let exe = env!("CARGO_BIN_EXE_planeter-glibc-check");
    // No arguments, or an unreadable base: usage (2).
    assert_eq!(Command::new(exe).output().unwrap().status.code(), Some(2));
    assert_eq!(
        Command::new(exe)
            .args(["/bin/true", "banana"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    // A file with no version needs (a text file): not a glibc binary (2), never "ok".
    let me = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    let out = Command::new(exe).args([&me, "2.41"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
