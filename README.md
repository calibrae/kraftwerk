# kraftwerk

[![CI](https://github.com/calibrae/kraftwerk/actions/workflows/ci.yml/badge.svg)](https://github.com/calibrae/kraftwerk/actions/workflows/ci.yml)
[![Release](https://github.com/calibrae/kraftwerk/actions/workflows/release.yml/badge.svg)](https://github.com/calibrae/kraftwerk/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/calibrae/kraftwerk)](https://github.com/calibrae/kraftwerk/releases/latest)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

Cross-platform desktop manager for remote KVM/QEMU virtual machines.
Rust + Tauri + Svelte, speaking libvirt over SSH.

A Rust port of the macOS [virtmanager](../virtmanager) Swift app with
meaningful UX overlap with Red Hat's Python `virt-manager` — without
GTK, without Python, without X11 forwarding.

```
Connect → Browse VMs → Console (serial/VNC/SPICE) → Configure → Create
```

## Status

Working daily driver against a real KVM host. **399 unit tests + 75
integration tests** against live hypervisors (per-domain + memory + vCPU max edits), all green.

Feature parity snapshot with Python virt-manager:

| Area | Status |
|------|--------|
| Connect / list / lifecycle (start/stop/pause/resume/reboot/force-off) | ✅ |
| Multi-hypervisor connection list (right-click edit, persistence, error/red on socket loss) | ✅ |
| Hypervisor dashboard (host CPU/RAM/storage/networks at a glance) | ✅ |
| libvirt event-driven UI (push state changes, no 3s polling) | ✅ |
| Live CPU + memory editing (current + max) | ✅ |
| Memory hotplug — `<maxMemory slots>` + live DIMM attach | ✅ |
| Snapshots — list/create/revert/delete with parent-child tree, VFIO-aware (disk-only + quiesce toggles) | ✅ |
| Raw domain-XML editor (escape hatch for unmodelled fields) | ✅ |
| Serial console (crytter WASM terminal) | ✅ |
| VNC console (noVNC, SSH-tunneled) | ✅ |
| SPICE console (capsaicin, native-Rust, with cursor + absolute mouse) | ✅ |
| Virtual networks (list + create NAT/route/open/isolated/bridge) | ✅ |
| Storage pools + volumes (dir/netfs/logical/iscsi, qcow2/raw/iso) | ✅ |
| Pool/volume delete guards (refuse with named domains when still attached) | ✅ |
| VM creation wizard (ISO install / import disk / empty) | ✅ |
| Boot / firmware / machine / features / events editor (with state-change warnings) | ✅ |
| Disks (add/edit/remove + CD-ROM live media change, boot-disk + bus-change confirms) | ✅ |
| NICs (all source types, live link-state toggle) | ✅ |
| Display / video / sound / input | ✅ |
| TPM / RNG / watchdog / panic / balloon / vsock / IOMMU | ✅ |
| Serial / channels (qemu-ga + vdagent presets) | ✅ |
| Filesystem passthrough (virtiofs + 9p) + shmem | ✅ |
| Controllers (USB / SCSI / virtio-serial — live model swap warning when devices attached) | ✅ |
| USB + PCI passthrough (enumerate host devs + attach/detach) | ✅ |
| CPU model/topology/features, cputune, memtune, NUMA, hugepages, iothreads | ✅ |
| Live metrics with sparklines (CPU / memory / disk IO / network IO) | ✅ |
| Bulk actions / multi-select (cmd/shift-click + start/stop/force-off/etc) | ✅ |
| Metrics graphs (CPU/RAM/disk-io/net-io · 1m/5m/15m/1h windows) | ✅ |
| VM cloning (full-copy via virStorageVolCreateXMLFrom · MAC strip · start-after) | ✅ |
| qemu log viewer (tails /var/log/libvirt/qemu/<vm>.log over SSH) | ✅ |
| Managed save / restore / screenshot / coredump | ✅ |
| Backing chain viewer + blockcommit/blockpull (active commit + pivot) | ✅ |
| Disk LUKS encryption + virSecret CRUD UI | ✅ |
| iSCSI CHAP / Ceph RBD pool auth | ✅ |
| Pool types: dir, fs, netfs, logical, iscsi, iscsi-direct, rbd, zfs | ✅ |
| Volume upload streaming (local file → pool over virStream) | ✅ |
| nwfilter (firewall rules per NIC) | ✅ |
| DHCP reservations + DNS hostname overrides on virtual networks | ✅ |
| Static routes on virtual networks | ✅ |
| Open vSwitch virtualport on NICs | ✅ |
| Live migration (peer-to-peer with auto-converge, bandwidth cap, persist/undefine flags) | ✅ |
| Mediated devices (mdev / NVIDIA vGPU / vfio-mdev) | ✅ |
| SR-IOV PF/VF enumeration (attach VFs via existing PCI passthrough) | ✅ |
| Nested virtualization toggle (Intel vmx / AMD svm + host kernel module probe) | ✅ |
| SEV / SEV-SNP / TDX launch security | ✅ (SEV writable, SEV-SNP/TDX read-only) |
| vTPM persistent NVRAM path + backup snippets | ✅ |
| OVA / OVF import | 🚧 phase 6 |

See [docs/ROADMAP.md](docs/ROADMAP.md) for the multi-phase plan
beyond per-domain config (events, snapshots, raw XML, hotplug were
phases 1 + 2 + 3 + 4 + 5 — now done), and [docs/CONFIG_ROADMAP.md](docs/CONFIG_ROADMAP.md)
for the full surface inventory with constraints and test expectations.

## Running it

Prerequisites: **Rust 1.90+**, **Node 20+**, **libvirt** on both the
client machine (for the Rust FFI) and the hypervisor, SSH key
auth to the hypervisor.

```bash
git clone https://github.com/calibrae/kraftwerk
cd kraftwerk
npm install
npm run tauri dev
```

Point at your hypervisor via the connection dialog; URI shape is
`qemu+ssh://user@host/system`. The app remembers connections across
launches.

## Releases

Pre-built bundles are produced by CI for **Linux** (AppImage, deb, rpm)
and **macOS Apple Silicon** (signed + notarized DMG). Grab them from
the [releases page](https://github.com/calibrae/kraftwerk/releases).

Windows is built best-effort (libvirt-on-Windows is second-class; the
bundle is not signed).

### Client-side libvirt is required

Kraftwerk links against `libvirt` via FFI, so the **client machine**
needs it installed even when the hypervisor is remote. The bundle is
not self-contained.

| Platform | Install |
|---|---|
| macOS (Apple Silicon) | `brew install libvirt` |
| macOS (Intel, local build) | `arch -x86_64 /usr/local/bin/brew install libvirt` |
| Debian / Ubuntu (.deb) | `sudo apt install libvirt0` (pulled in automatically) |
| Fedora / RHEL (.rpm) | `sudo dnf install libvirt-libs` (pulled in automatically) |
| Arch (AppImage) | `sudo pacman -S libvirt` |

On first launch the app will crash with a `Library not loaded: libvirt.0.dylib`
error if libvirt is missing — install it, then reopen.

### Intel Macs — build from source

GitHub retired free Intel macOS CI runners in early 2026, so we no
longer ship an x86_64 DMG. Intel Macs can still build locally:

```bash
# Install Intel Homebrew alongside Apple Silicon Homebrew.
arch -x86_64 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
# Install libvirt via the Intel brew (goes to /usr/local).
arch -x86_64 /usr/local/bin/brew install libvirt pkg-config

# Point cargo at the Intel libvirt during the build.
cd kraftwerk
npm install
PKG_CONFIG_PATH=/usr/local/opt/libvirt/lib/pkgconfig \
  npm run tauri build -- --target x86_64-apple-darwin
```

The DMG lands in `src-tauri/target/x86_64-apple-darwin/release/bundle/dmg/`.
It is not code-signed; you'll need to `xattr -dr com.apple.quarantine`
the `.app` on first launch, or sign it with your own Developer ID.

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
