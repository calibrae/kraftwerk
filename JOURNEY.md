# virtmanager-rs — Journey Notes

A Rust + Tauri + Svelte port of the Swift macOS VirtManager, built
out in one long session against a live KVM host (testhost). From empty
Tauri scaffold to connect / list / lifecycle / serial console / VNC /
SPICE / full device editor / passthrough / creation wizard in a
single arc. This doc is the build log.

## Where we ended up

```
src-tauri/
├── src/
│   ├── app_state.rs       Mutex-backed global state
│   ├── lib.rs             Tauri entry, command registration
│   ├── models/            Plain serializable data types
│   ├── libvirt/           Thin wrapper around the `virt` crate
│   │   ├── connection.rs  One LibvirtConnection per app
│   │   ├── console.rs     Serial console over virStream
│   │   ├── vnc_proxy.rs   SSH tunnel + WebSocket bridge for noVNC
│   │   ├── spice_proxy.rs SSH tunnel + capsaicin client
│   │   ├── domain_caps.rs virConnectGetDomainCapabilities parser
│   │   ├── domain_stats.rs libvirt → UI sparkline samples
│   │   ├── hostdev.rs     Node-device enum + hostdev XML
│   │   ├── boot_config.rs OS / firmware / machine / events
│   │   ├── disk_config.rs Disks add / edit / remove + CD-ROM media
│   │   ├── nic_config.rs  NICs — all source types
│   │   ├── display_config.rs Graphics / video / sound / input
│   │   ├── virtio_devices.rs TPM / RNG / watchdog / panic / balloon / vsock / IOMMU
│   │   ├── char_devices.rs Serial / console / channels (qemu-ga, vdagent)
│   │   ├── filesystem_config.rs virtiofs / 9p / shmem
│   │   ├── controller_config.rs USB / SCSI / virtio-serial
│   │   ├── cpu_tune_config.rs CPU model + topology + cputune + memtune + NUMA + hugepages + iothreads
│   │   ├── domain_config.rs Legacy read-only summary
│   │   ├── domain_builder.rs XML emission for the creation wizard
│   │   ├── network_config.rs Network mode + IPv4/IPv6/DHCP/DNS
│   │   └── storage_config.rs Pool + volume XML
│   ├── commands/          Tauri command handlers (thin wrappers)
│   └── ...
├── Cargo.toml             capsaicin pinned to a GitHub rev
└── tauri.conf.json

src/
├── routes/+page.svelte    Sidebar + main view router
└── lib/
    ├── stores/app.svelte.js Reactive state + IPC calls + auto-poll
    ├── components/        One .svelte per panel/dialog
    └── vendor/novnc/      Vendored noVNC ESM source

docs/
├── ARCHITECTURE.md
├── CONFIG_ROADMAP.md      Rounds A–K + deferred
└── SWIFT_REFERENCE.md     Notes from the original app
```

~150k Rust / ~15k Svelte. **353 unit tests + 75 integration tests** against a live hypervisor.

## Ground rules that paid off

- **Persistent XML round-trip via in-place mutation, not parse-and-reserialize.**
  Every editor module (boot, disks, nics, display, virtio, char, filesystem,
  controllers, cpu_tune) streams quick-xml events and splices in new
  elements without touching untouched siblings. That's how seclabel,
  IOThreads, PCI addresses, libvirt metadata, and everything we don't
  explicitly model all survive edits. Python virt-manager learned
  this lesson the hard way over many years; it was on the roadmap
  from day one for us.
- **Read the inactive (persistent) XML for config editors.**
  When you edit an already-running VM, the effect is on next boot.
  Reading the running XML shows stale config. All editors read with
  `VIR_DOMAIN_XML_INACTIVE`.
- **Live vs persistent, declared per feature.**
  Disks hotplug, NICs hotplug, memory balloon hotplugs, host passthrough
  hotplugs. TPM / watchdog / panic / firmware / machine type do not.
  The UI tells you which.
- **fedora-workstation on testhost is disposable.** Integration tests
  attach / detach / reconfigure against a real VM with Drop-guard
  cleanup. No mocks.
- **capsaicin is its own project.** Pure-Rust SPICE client (RSA auth,
  QUIC, LZ, GLZ, MJPEG, keyboard, mouse, cursor channel) sits at
  github.com/calibrae/capsaicin. virtmanager-rs depends on it by git
  rev. Two inbox notes from us (mouse-mode + cursor-channel priority)
  → capsaicin shipped both, we pulled the bump, swapped our SPICE
  console to the client/server mouse-mode-aware path.

## The path

### Scaffold → first light

Tauri v2 + SvelteKit static adapter. The `virt` crate wraps libvirt's
C API with Drop-based cleanup; a `Mutex<Option<Connect>>` keeps it
thread-safe. First milestone: auto-connect to `qemu+ssh://testuser@testhost/system`
on startup, list all five VMs in the sidebar with state badges.

### Serial console

`virDomainOpenConsole` returns a bidirectional `virStream`. Blocking
`recv` on a dedicated thread; bytes arrive in a callback that emits
Tauri events. Frontend pulls `crytter` — the user's own pure-Rust WASM
terminal emulator, full VT100 + scrollback + selection. Dropped into a
Svelte component. Key events mapped to PC-AT scancodes with extended
0xE0 prefix packing.

### VNC

Libvirt's `virDomainOpenGraphicsFD` returns the RFB-speaking fd… only
over Unix socket connections. Over SSH you can't pass fds. Solution:
parse VNC port from domain XML, spawn `ssh -N -L`, bridge it to a
local WebSocket, point noVNC at it. noVNC was a saga in itself: the
`@novnc/novnc` npm package ships CJS with top-level-await in a way
that Vite can't bundle; `novnc-core` is also CJS; installing direct
from the GitHub tag strips the ESM `core/`. Ended up vendoring the
ESM source under `src/lib/vendor/novnc/` and doing `import RFB from
"$lib/vendor/novnc/core/rfb.js"`. Lazy-loaded on button click so a
failure there can't blank the whole app.

### SPICE

Three paths considered: spice-html5 over WS proxy, native Rust client,
or `remote-viewer` subprocess. The user's own `capsaicin` project
became the native-Rust answer — purpose-built to be embedded. We:
- pull it by path-dep during bring-up, then switch to a GitHub rev
  once the API stabilised
- SSH-tunnel the SPICE port (same pattern as VNC)
- feed the socket to `capsaicin_client::SpiceClient`
- pump `ClientEvent`s to the frontend via Tauri events
- base64-encode raw BGRA rect payloads (JSON array encoding 3x'd the
  bandwidth)
- canvas + cursor-overlay layered renderer
- keyboard: PC-AT scancodes with 0x80 break-bit on the *low byte only*
  for KeyUp (the sticky-keys bug was 90 minutes of debugging before
  the capsaicin inbox note pointed at the fix)
- mouse: negotiate client (absolute) vs server (relative) mode via
  `ClientEvent::MouseMode`. SERVER mode uses pointer lock with
  Ctrl+Alt+Shift as release — ESC would release too (browser
  enforced) but we don't advertise it since ESC is a valid guest key

### Networks + storage

Same shape for each: list / create / delete / lifecycle / per-row
actions / creation dialogs. Network supports five forward modes
(nat/route/open/isolated/bridge) with conditional fields per mode.
Storage supports four pool types (dir/netfs/logical/iscsi) with
volume management (qcow2/raw/iso, thin-provisioning).

### Config surface rounds A–K

With F1 (domain capabilities) and F3 (live/persistent matrix) in place,
we fanned out — seven parallel worktrees on speedwagon, one agent
per round, all implementing the same {parser, builder, patcher,
connection method, commands, UI tab} slice:

- **A** boot / firmware / machine / features / events
- **B** disks — add/edit/remove + CD-ROM live media change
- **C** NICs — every source type, link-state live toggle
- **D** display / video / sound / input
- **E** TPM / RNG / watchdog / panic / balloon / vsock / IOMMU
- **F** serial / console / channels (qemu-ga + vdagent presets)
- **G** filesystem passthrough (virtiofs + 9p) + shmem
- **H** USB / SCSI / virtio-serial controllers
- **I** CPU model + topology + cputune + memtune + NUMA + hugepages + iothreads
- **K** creation wizard 2.0 — ISO install / import disk / empty

Merge strategy: one at a time into main, with a union-resolution
script for additive conflicts (lib.rs command table, mod.rs entries,
VmDetail.svelte tab list, integration_testhost.rs test appends). A few
were not mechanical — B and F needed closing-brace fixes because the
union cut through method bodies.

### Things that were pulled out to separate projects

- **crytter** — WASM terminal emulator, already separate
- **capsaicin** — SPICE client, already separate

Those got proper API surfaces (events / ports / named functions) and
virtmanager-rs just consumes them.

## Things we deliberately deferred

- **Round J**: SEV / SEV-SNP / TDX launch security, seclabel editor.
  Rare enough that it's not blocking anyone; leave it until someone
  asks.
- **libosinfo integration**. Kept our handwritten OS-variant table.
  Hooking libosinfo (C lib) for richer OS detection is a
  nice-to-have, not a must.
- **Migration UI**. `virDomainMigrateToURI3` works; the UX is a
  separate project — live migration, offline migration, tunnelled vs
  direct, incoming connection URIs, post-copy, etc. Out of scope.
- **Snapshot management**. `virDomainSnapshotCreateXML` is there;
  didn't wire a UI.
- **Window state plugin**. Window size doesn't remember across
  launches. One line of Cargo.toml; add when we care.

## Credits

- Swift app that inspired this port — `~/Developer/perso/virtmanager`
- libvirt, the `virt` crate, quick-xml, tokio, Tauri, Svelte
- crytter (terminal) + capsaicin (SPICE) — the sibling projects that
  did the hard codec/protocol work
