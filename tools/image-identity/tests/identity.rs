//! The identity check on the shapes a registry returns: an OCI index with an image manifest and an attestation manifest per
//! platform (what `buildx --provenance` pushes), and the cases that must never pass.

use planeter_image_identity::{Failure, config_digest, is_digest, parse, platform_manifest, same};

const AMD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const ARM: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const ATT_AMD: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
const ATT_ARM: &str = "sha256:4444444444444444444444444444444444444444444444444444444444444444";
const CONFIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn entry(digest: &str, os: &str, arch: &str, attestation_of: Option<&str>) -> String {
    let annotations = attestation_of.map_or(String::new(), |of| {
        format!(
            r#","annotations":{{"vnd.docker.reference.digest":"{of}","vnd.docker.reference.type":"attestation-manifest"}}"#
        )
    });
    format!(
        r#"{{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"{digest}","size":1234,"platform":{{"architecture":"{arch}","os":"{os}"}}{annotations}}}"#
    )
}

fn index(entries: &[String]) -> String {
    format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[{}]}}"#,
        entries.join(",")
    )
}

fn real_shape() -> String {
    index(&[
        entry(AMD, "linux", "amd64", None),
        entry(ARM, "linux", "arm64", None),
        entry(ATT_AMD, "unknown", "unknown", Some(AMD)),
        entry(ATT_ARM, "unknown", "unknown", Some(ARM)),
    ])
}

#[test]
fn the_platform_manifest_is_the_images_not_the_attestations() {
    let ix = real_shape();
    assert_eq!(platform_manifest(&ix, "amd64").unwrap(), AMD);
    assert_eq!(platform_manifest(&ix, "arm64").unwrap(), ARM);
}

#[test]
fn an_index_with_no_matching_platform_fails() {
    let ix = index(&[entry(AMD, "linux", "amd64", None)]);
    assert!(matches!(
        platform_manifest(&ix, "arm64"),
        Err(Failure::Mismatch(_))
    ));
    assert!(matches!(
        platform_manifest(&index(&[]), "amd64"),
        Err(Failure::Mismatch(_))
    ));
}

#[test]
fn an_index_with_only_attestation_manifests_fails() {
    let ix = index(&[
        entry(ATT_AMD, "unknown", "unknown", Some(AMD)),
        entry(ATT_ARM, "unknown", "unknown", Some(ARM)),
    ]);
    for arch in ["amd64", "arm64"] {
        let e = platform_manifest(&ix, arch).unwrap_err();
        assert!(matches!(e, Failure::Mismatch(_)), "{e}");
    }
}

#[test]
fn another_os_or_two_images_for_one_platform_are_not_a_match() {
    let windows = index(&[entry(AMD, "windows", "amd64", None)]);
    assert!(matches!(
        platform_manifest(&windows, "amd64"),
        Err(Failure::Mismatch(_))
    ));
    let twice = index(&[
        entry(AMD, "linux", "amd64", None),
        entry(ARM, "linux", "amd64", None),
    ]);
    let e = platform_manifest(&twice, "amd64").unwrap_err();
    assert!(
        matches!(e, Failure::Mismatch(_)) && e.to_string().contains("2 images"),
        "{e}"
    );
}

#[test]
fn input_that_is_not_an_index_is_a_usage_failure_never_a_pass() {
    for bad in [
        "",
        "not json",
        "{",
        r#"{"manifests": [}"#,
        r#"{"schemaVersion":2}"#,
        r#"{"manifests":"x"}"#,
        r#"{"config":{"digest":"sha256:aa"}}"#,
        &format!("{} trailing", real_shape()),
    ] {
        let e = platform_manifest(bad, "amd64").unwrap_err();
        assert!(matches!(e, Failure::Usage(_)), "{bad:?} gave {e:?}");
    }
    let bad_digest = index(&[entry("sha256:xyz", "linux", "amd64", None)]);
    assert!(matches!(
        platform_manifest(&bad_digest, "amd64"),
        Err(Failure::Usage(_))
    ));
}

#[test]
fn the_config_digest_is_read_from_the_manifest() {
    let m = format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"{CONFIG}","size":512}},"layers":[{{"digest":"{OTHER}","size":9}}]}}"#
    );
    assert_eq!(
        config_digest(&m).unwrap(),
        CONFIG,
        "the layers' digests are not the config's"
    );
}

#[test]
fn a_manifest_without_a_usable_config_is_a_usage_failure() {
    for bad in [
        "",
        "[]",
        r#"{"layers":[]}"#,
        r#"{"config":{}}"#,
        r#"{"config":{"digest":7}}"#,
        r#"{"config":{"digest":"sha256:short"}}"#,
        &format!(r#"{{"config":{{"digest":"{}"}}}}"#, CONFIG.to_uppercase()),
    ] {
        let e = config_digest(bad).unwrap_err();
        assert!(matches!(e, Failure::Usage(_)), "{bad:?} gave {e:?}");
    }
}

#[test]
fn equal_ids_pass_different_fail_naming_both_and_malformed_ones_are_usage() {
    assert_eq!(same(CONFIG, CONFIG), Ok(()));
    assert_eq!(
        same(&format!("{CONFIG}\n"), CONFIG),
        Ok(()),
        "a trailing newline from `cat` is not a difference"
    );
    let e = same(CONFIG, OTHER).unwrap_err();
    assert!(matches!(e, Failure::Mismatch(_)));
    let said = e.to_string();
    assert!(said.contains(CONFIG) && said.contains(OTHER), "{said}");
    for (a, b) in [
        ("", CONFIG),
        (CONFIG, ""),
        ("sha256:abc", "sha256:abc"),
        ("latest", "latest"),
    ] {
        assert!(matches!(same(a, b), Err(Failure::Usage(_))), "{a:?} {b:?}");
    }
}

#[test]
fn digests_are_exactly_sha256_and_sixty_four_lower_case_hex_digits() {
    assert!(is_digest(CONFIG));
    assert!(!is_digest("sha256:"));
    assert!(!is_digest(&CONFIG[..CONFIG.len() - 1]));
    assert!(!is_digest(&format!("{CONFIG}0")));
    assert!(!is_digest(&CONFIG.replace("sha256", "sha512")));
    assert!(!is_digest(&CONFIG.to_uppercase()));
}

#[test]
fn the_json_reader_handles_escapes_and_refuses_what_is_not_json() {
    let v = parse(r#"{"a":"x\"y\\z\u00e9\ud83d\ude00","b":[1,-2.5e3,true,false,null],"c":{}}"#)
        .unwrap();
    assert_eq!(v.get("a").and_then(|s| s.as_str()), Some("x\"y\\zé😀"));
    for bad in [
        "{\"a\":1,}",
        "[1 2]",
        "{\"a\" 1}",
        "\"open",
        "01x",
        "{\"a\":\"\u{1}\"}",
        "nul",
        "[",
        "{\"a\":\"\\q\"}",
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    let deep = "[".repeat(100) + &"]".repeat(100);
    assert!(
        parse(&deep).is_err(),
        "a document nested a hundred deep is refused, not recursed into"
    );
}
