# virtmanager-rs

Cross-platform desktop manager for remote KVM/QEMU virtual machines.
Rust + Tauri + Svelte, speaking libvirt over SSH.

A Rust port of the macOS [virtmanager](../virtmanager) Swift app with
meaningful UX overlap with Red Hat's Python `virt-manager` — without
GTK, without Python, without X11 forwarding.

```
Connect → Browse VMs → Console (serial/VNC/SPICE) → Configure → Create
```

## Status

Working daily driver against a real KVM host. **353 unit tests + 75
integration tests** against a live hypervisor, all green.

Feature parity snapshot with Python virt-manager:

| Area | Status |
|------|--------|
| Connect / list / lifecycle (start/stop/pause/resume/reboot/force-off) | ✅ |
| Live CPU + memory editing | ✅ |
| Serial console (crytter WASM terminal) | ✅ |
| VNC console (noVNC, SSH-tunneled) | ✅ |
| SPICE console (capsaicin, native-Rust, with cursor + absolute mouse) | ✅ |
| Virtual networks (list + create NAT/route/open/isolated/bridge) | ✅ |
| Storage pools + volumes (dir/netfs/logical/iscsi, qcow2/raw/iso) | ✅ |
| VM creation wizard (ISO install / import disk / empty) | ✅ |
| Boot / firmware / machine / features / events editor | ✅ |
| Disks (add/edit/remove + CD-ROM live media change) | ✅ |
| NICs (all source types, live link-state toggle) | ✅ |
| Display / video / sound / input | ✅ |
| TPM / RNG / watchdog / panic / balloon / vsock / IOMMU | ✅ |
| Serial / channels (qemu-ga + vdagent presets) | ✅ |
| Filesystem passthrough (virtiofs + 9p) + shmem | ✅ |
| Controllers (USB / SCSI / virtio-serial) | ✅ |
| USB + PCI passthrough (enumerate host devs + attach/detach) | ✅ |
| CPU model/topology/features, cputune, memtune, NUMA, hugepages, iothreads | ✅ |
| Live metrics with sparklines (CPU / memory / disk IO / network IO) | ✅ |
| SEV / TDX launch security | ❌ deferred |
| Snapshots | ❌ deferred |
| Migration UI | ❌ deferred |

See [docs/CONFIG_ROADMAP.md](docs/CONFIG_ROADMAP.md) for the full
surface inventory with constraints and test expectations.

## Running it

Prerequisites: **Rust 1.90+**, **Node 20+**, **libvirt** on both the
client machine (for the Rust FFI) and the hypervisor, SSH key
auth to the hypervisor.

```bash
git clone https://github.com/calibrae/virtmanager-rs
cd virtmanager-rs
npm install
npm run tauri dev
```

Point at your hypervisor via the connection dialog; URI shape is
`qemu+ssh://user@host/system`. The app remembers connections across
launches.

## Architecture in one paragraph

Tauri app with a Rust backend and a Svelte frontend. Rust holds the
libvirt connection (the `virt` crate wraps the C API), plus small
modules per feature that parse + patch + build libvirt XML without
touching unrelated sections. The frontend talks to Rust via Tauri's
invoke bridge and receives pushed events (console bytes, SPICE
frames, stat samples). Consoles: serial streams native libvirt
`virStream`; VNC spawns an SSH port-forward and proxies to a
WebSocket for `@novnc/novnc`; SPICE spawns an SSH port-forward and
feeds the socket to `capsaicin-client` (pure-Rust SPICE), pumping
decoded display events as Tauri messages.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
[JOURNEY.md](JOURNEY.md) for more.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). TL;DR:

- Unit tests are mandatory for new parsers / builders / patchers
- Integration tests (against a real libvirt host) are mandatory for
  anything touching live VMs
- XML escape every string that came from user input
- Read the inactive XML for config editors, not the live XML
- Prefer in-place XML mutation over parse-and-reserialize

## License

Dual-licensed under Apache-2.0 and MIT at your option. See
[LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT).
