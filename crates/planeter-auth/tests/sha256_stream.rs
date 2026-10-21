//! RFC 004 D-2: the streaming SHA-256 behind the artifact store equals the whole-buffer digest.

use planeter_auth::{Sha256Stream, sha256_hex};

#[test]
fn the_streamed_digest_equals_the_whole_buffer_digest_at_any_split() {
    let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let whole = sha256_hex(&data);
    for split in [0, 1, 63, 64, 65, 5_000, 9_999, 10_000] {
        let mut s = Sha256Stream::new();
        s.update(&data[..split]);
        s.update(&data[split..]);
        assert_eq!(s.finish_hex(), whole, "split at {split}");
    }
    // The well-known digest of the empty input.
    assert_eq!(
        Sha256Stream::new().finish_hex(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}
