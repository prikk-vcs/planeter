//! The prikk floor the client names is the server's own (`crates/planeter-prikk/src/cli.rs`): they cannot drift apart.

use planeter_client::prikk::{PRIKK_FLOOR, floor_text, parse_version};

#[test]
fn the_clients_floor_is_the_servers_floor() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../planeter-prikk/src/cli.rs"
    ))
    .unwrap();
    let line = source
        .lines()
        .find(|l| l.contains("pub const MIN_PRIKK_VERSION"))
        .expect("the server's floor constant");
    let digits: Vec<u64> = line
        .split_once('=')
        .map_or("", |(_, value)| value)
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    assert_eq!((digits[0], digits[1], digits[2]), PRIKK_FLOOR, "{line}");
    assert_eq!(
        floor_text(),
        format!("{}.{}.{}", digits[0], digits[1], digits[2])
    );
}

#[test]
fn prikks_version_line_is_read() {
    assert_eq!(parse_version("prikk 0.55.0\n"), Some((0, 55, 0)));
    assert_eq!(parse_version("prikk 0.54.1"), Some((0, 54, 1)));
    assert_eq!(parse_version("nothing"), None);
}
