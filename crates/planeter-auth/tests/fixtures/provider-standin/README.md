Test-only files for `tests/oidc_private_provider.rs` (RFC 014 D-11): a self-made certificate and its key (valid for `idp.test`, `other.test`,
`mixed.test`, `127.0.0.1`, `127.0.0.2`; **a throw-away key that protects nothing and is trusted by nothing but those tests**, through `curl --cacert`) and
`standin.py`, a stand-in OpenID provider. They were made with
`openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 36500 -subj "/CN=planeter test stand-in provider" -addext "subjectAltName=DNS:idp.test,DNS:other.test,DNS:mixed.test,IP:127.0.0.1,IP:127.0.0.2" -keyout key.pem -out cert.pem`.
