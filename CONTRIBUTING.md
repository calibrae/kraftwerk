# Contributing

## Development setup

```bash
# Rust toolchain 1.90+
rustup default stable

# Node 20+
nvm use 20

# libvirt headers (macOS + Homebrew)
brew install libvirt pkg-config
```

Clone + build:

```bash
git clone https://github.com/calibrae/virtmanager-rs
cd virtmanager-rs
npm install
cd src-tauri && cargo build
```

Run dev:

```bash
npm run tauri dev
```

## Testing

**Both unit and integration tests are mandatory** for new modules that
touch libvirt, the domain XML, or device lifecycle.

```bash
cd src-tauri
cargo test --lib                 # unit tests (no libvirt required)
cargo test --test integration_testhost -- --test-threads=1
                                  # live-host tests — requires a
                                  # configured hypervisor (see below)
```

### Integration test setup

The integration suite (`tests/integration_testhost.rs`) talks to a real
libvirt hypervisor. It's named for the author's host but you can
point it at any host. See `connect_testhost()` near the top of the
file.

Tests that *modify* state only touch `fedora-workstation` (see
`TEST_VM`). Production-style VMs on the same host are asserted to be
unchanged at suite exit.

New tests that mutate state **must** wrap the mutation in a Drop
guard that restores the original state, even on panic. See
`NicCleanup`, `PanicGuard`, `RngGuard`, `RoundGCleanup`,
`NetworkCleanup` for the pattern.

## Style rules

### Rust

- XML mutation: prefer in-place (parse → splice) over
  parse-and-reserialize. Unknown elements must round-trip exactly.
  See `boot_config::apply` or `nic_config::apply_*` as the pattern.
- Config editors read **inactive** (persistent) XML, not live —
  editing an already-running VM takes effect on next boot, and
  reading live shows stale config mid-edit.
- Every user string going into XML: escape via
  `crate::libvirt::xml_helpers::escape_xml`. There's a test for
  injection safety in every config module; add one.
- Errors: no panics in library code. Map to `VirtManagerError`.
- `unsafe` only in `libvirt/vnc_proxy.rs`, `libvirt/console.rs`,
  `libvirt/hostdev.rs` for FFI / fd / raw-pointer work. Everything
  else is safe Rust.

### Svelte / frontend

- Svelte 5 runes syntax (`$state`, `$derived`, `$effect`, `$props`).
- Never name a local `state` — conflicts with the `$state` rune.
  Convention in this codebase: `appState`.
- Lazy-import third-party panels that can blow up (SPICE console,
  VNC console, anything with WASM) so a failure there can't blank
  the whole app.
- Always escape the wrapping `<div>` tab-content when touching
  VmDetail so a crashed tab doesn't take down siblings.

### Tests

- Unit: parse / build / round-trip / injection-safe + whatever
  validation rules you added.
- Integration: add → verify → remove, always with a Drop guard. Use
  a unique name (timestamp suffix) if there's any chance of collision
  with a prior aborted run.

## Commit messages

Short title, focus on the *why*. Body for anything that isn't
obvious from the diff. We don't sign off.

No AI coauthor attribution.

## License

Contributions are accepted under the project's dual Apache-2.0 / MIT
terms. By submitting a PR you agree that your contribution is
licensed under those terms.
