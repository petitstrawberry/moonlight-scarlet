# Upstream provenance

This directory vendors `moonlight-stream/moonlight-common-c` and its required
submodules at the following revisions:

- moonlight-common-c: `874ac9548f1bd6f095ef2b435c42cdde460e7821`
- enet: `aca87840b57f045a1f7f9299e4b1b9b8e2a5e2f1`
- nanors: `b1e3c22ca0cdc0bb83e3cd6ed1a2fc77869ed99a`

The vendored source is kept unmodified. Scarlet-specific headers and runtime
bindings live under `moonlight-sys/platform/scarlet`; `moonlight-sys/build.rs`
adds that compatibility layer only for Scarlet targets.
