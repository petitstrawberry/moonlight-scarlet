# Moonlight Scarlet

Moonlight Scarlet is a Moonlight game-streaming client written in Rust for
Scarlet. The application, control plane, and connection lifecycle are kept
portable so they can be developed and tested on macOS. The streaming video
backend is Scarlet-only and uses Scarlet's hardware video decoder.

## Workspace

- `moonlight`: cross-platform ScarletUI application
- `moonlight-control`: GameStream/Sunshine HTTP control plane
- `moonlight-sys`: safe ownership boundary around `moonlight-common-c`

`moonlight-common-c`, its bundled ENet revision, and mbedTLS form the streaming
transport core. Rust owns pairing, host/application discovery, launch requests,
UI, and platform media/input integration.

## Platform support

| Capability | macOS | Scarlet |
| --- | --- | --- |
| ScarletUI application | Winit | SWS |
| Control plane | Supported | Supported |
| Connection core | Supported | Supported |
| Hardware video | Unsupported (transport validation only) | H.264 via `/dev/video0` |
| Video presentation | Unsupported | NV12 to BGRA in ScarletUI |
| Audio output | Unsupported | Opus multistream via libopus to SAS |
| Keyboard/mouse input | ScarletUI/Winit | ScarletUI/SWS with pointer capture |

The decoder and decoded-frame presentation path are compiled only for Scarlet;
the macOS development build intentionally has no software-video fallback. It
still runs the same control plane, connection core, navigation, and lifecycle
code for host-side development and tests.

During a stream, click the video surface to capture input. The desktop-client
shortcuts `Ctrl+Alt+Shift+Z`, `Ctrl+Alt+Shift+Q`, and `Ctrl+Alt+Shift+X` toggle
input capture, disconnect the stream, and toggle fullscreen respectively.

Successfully connected hosts are remembered in the platform configuration
directory and restored on the next launch. Core native-component license and
attribution text is available from **Settings > Open source licenses**.

## Development

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

For the Scarlet toolchain and SDK:

```bash
nix develop
cargo build --release -p moonlight --target riscv64gc-unknown-scarlet
cargo build --release -p moonlight --target aarch64-unknown-scarlet
```

## License

Moonlight Scarlet is licensed under GPL-3.0-only because it links
`moonlight-common-c`.
