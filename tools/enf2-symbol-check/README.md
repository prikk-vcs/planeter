# ENF-2 symbol-absence check

`check.sh` enforces **ENF-2** (RFC 001 §0a / LAY-4 / threat-model INV-2): planeter is a **keyless
forge**, so its default build must contain **no** history-signing ("forge-seal") symbol.

- Any code that could sign history lives behind the `forge-seal` cargo feature on `planeter-core`,
  which is **off by default**. No such code exists yet — this check passes trivially today.
- The check builds the default profile (`cargo build --release --workspace`) and greps the artifacts'
  symbols (`nm`) for the reserved marker `forge_seal_sign`. It **fails** if that symbol is present in a
  default build.
- **Convention:** when the `forge-seal` feature is eventually implemented, its signer entry point MUST
  be named so its symbol contains `forge_seal_sign` (e.g. `forge_seal_sign_history`). That is what makes
  an accidental default-build inclusion detectable here.

Run locally:

```sh
tools/enf2-symbol-check/check.sh
```

CI runs it in the `enf2` job (see `.github/workflows/ci.yml`).
