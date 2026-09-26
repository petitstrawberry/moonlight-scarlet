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
| Video presentation | Unsupported | Shared NV12 images, GPU color conversion/scaling |
| Audio output | Unsupported | Opus multistream via libopus to SAS |
| Keyboard/mouse input | ScarletUI/Winit | ScarletUI/SWS; optional mouse capture |
| Gamepad input | Raw snapshots | Native gamepads, independent of mouse capture |
| Touch input | No video preview | Native multitouch; absolute mouse fallback |

The decoder and decoded-frame presentation path are compiled only for Scarlet;
the macOS development build intentionally has no software-video fallback. It
still runs the same control plane, connection core, navigation, and lifecycle
code for host-side development and tests.

Scarlet keeps decoded NV12 frames in shared image leases and samples them directly
through ScarletUI/SGFX. Scaling and color conversion run on the GPU, with no
full-frame CPU canvas or BGRA conversion. The decoder must support shared image
output. Only the latest unpublished frame is retained; in-flight GPU commands
keep their own image lease until sampling completes.

On Scarlet, touching the video sends native touchscreen contacts to compatible
Sunshine hosts without requiring mouse capture. Hosts without that capability
receive a one-finger absolute mouse drag instead. Touches starting in the black
bars or on local controls are not forwarded. Focus loss and stream shutdown
cancel outstanding contacts.

During a stream, click the video surface or use **Capture Mouse** to lock the mouse.
Keyboard input follows video focus, while touch and gamepads work without mouse
capture. Gamepad menu navigation is disabled during streaming so buttons are not
also sent as synthetic keyboard navigation. The desktop-client
shortcuts `Ctrl+Alt+Shift+Z`, `Ctrl+Alt+Shift+Q`, and `Ctrl+Alt+Shift+X` toggle
mouse capture, disconnect the stream, and toggle fullscreen respectively.

**Settings > Swap A/B** exchanges only the A and B buttons sent to the host.
It is off by default, preserving incoming button identities. The choice is
saved in `settings.json` in the platform configuration directory.

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
