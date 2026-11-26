# The sui-id interoperability procedure (RFC 014 D-12)

Test-only files. Nothing here runs in the gates or in CI.

* `run.sh SUI_ID_REPO TAG EVIDENCE_DIR` — the whole run, from a tagged sui-id to the evidence, in one work directory that is removed at the
  end (and the two processes it starts are stopped by their recorded PIDs) on every way out. Run it from the repository root.
* `tlsproxy.py` — a TLS terminator with a throw-away CA's certificate in front of sui-id (which does not terminate TLS), that also writes a
  request log and can, on a control file, serve a stale key set to a request without `no-cache`, delay the token endpoint, answer a garbage
  discovery document, or close the connection (the provider is down).
* `admin.py` — sui-id's own admin console driven with plain HTTP calls (create a user or a client, rotate the signing key, disable and enable a
  user), and the two signals that stop and resume the provider's process.
* The test itself is `../../interop_sui_id.rs` (`#[ignore]`): `cargo test -p planeter-auth --test interop_sui_id -- --ignored --test-threads=1`
  with the `PLANETER_SUI_ID_*` settings that `run.sh` writes; without them every test says it was skipped and passes.
