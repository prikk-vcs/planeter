# planeter server image. Built by .github/workflows/release.yml from the PREBUILT, attested release
# binaries (staged under ctx/<arch>/planeter) — this file never compiles Rust. It bundles the two
# runtime prerequisites a bare binary would otherwise need on the host: a pinned prikk release
# (dependency-ledger PK-26), bubblewrap (the sandbox, RFC 001 T4) and curl (the confined outbound
# HTTPS client behind the egress guard — OIDC discovery/JWKS/token exchange). `prlimit` (the memory
# ceiling, RFC 011 D-2) comes from util-linux, which the base image carries as an Essential package; the
# build checks it below so an image without it cannot be produced.
#
# Running it: bubblewrap needs user namespaces. On most hosts that means `--security-opt
# seccomp=unconfined` or a seccomp profile that permits unshare/clone with CLONE_NEWUSER, and
# `--cap-add SYS_ADMIN` on kernels that restrict unprivileged user namespaces. See docs/RELEASING.md.
# Debian 13 "trixie" (glibc 2.41), the multi-arch index digest read from the registry on 2026-10-29 (D-51; was bookworm-slim, glibc 2.36, which the
# release binaries — built on glibc-2.39 runners — cannot start on: GLIBC_2.38 not found). `release.yml` runs the image and checks the glibc before any push.
FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4

ARG TARGETARCH
ARG PRIKK_VERSION=0.54.0

RUN apt-get update \
 && apt-get install -y --no-install-recommends bubblewrap ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && prlimit --version

# prikk: the exact release asset for this architecture, checksum-verified against the release's own
# .sha256 file before extraction.
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) T=x86_64-unknown-linux-gnu ;; \
      arm64) T=aarch64-unknown-linux-gnu ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    cd /tmp; \
    curl -fsSLO "https://github.com/prikk-vcs/prikk/releases/download/${PRIKK_VERSION}/prikk-${T}.tar.gz"; \
    curl -fsSLO "https://github.com/prikk-vcs/prikk/releases/download/${PRIKK_VERSION}/prikk-${T}.tar.gz.sha256"; \
    sha256sum -c "prikk-${T}.tar.gz.sha256"; \
    tar -xzf "prikk-${T}.tar.gz" -C /usr/local/bin prikk; \
    rm -f "prikk-${T}.tar.gz" "prikk-${T}.tar.gz.sha256"; \
    prikk --version

COPY ctx/${TARGETARCH}/planeter /usr/local/bin/planeter

RUN useradd --system --uid 10001 --home /var/lib/planeter --shell /usr/sbin/nologin planeter \
 && mkdir -p /var/lib/planeter \
 && chown planeter:planeter /var/lib/planeter

USER planeter
WORKDIR /var/lib/planeter
ENV PLANETER_ADDR=0.0.0.0:8080 \
    PLANETER_REPOS_ROOT=/var/lib/planeter/repos \
    PLANETER_DB=/var/lib/planeter/planeter.db
VOLUME ["/var/lib/planeter"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/planeter"]
