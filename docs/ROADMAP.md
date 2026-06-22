# kraftwerk Roadmap

This started as a forward-looking plan to close the gap between
kraftwerk and full libvirt surface coverage. Phases 1–6 are now all
shipped — see [STATUS.md](STATUS.md) for the live feature matrix.

This doc is kept as the historical scoping record (what was planned,
why) and to track what's still ahead.

## Shipped — Phases 1 through 6

All six original phases landed. Per-area highlights below; estimates
removed since they're moot now.

### Phase 1 — Foundations
- **libvirt events stream** — push-driven UI updates via
  `virConnectDomainEventRegister`, no more 3s polling. ✅
- **Snapshots** — list/create/revert/delete with parent–child tree,
  VFIO-aware (disk-only + quiesce toggles). ✅
- **Raw XML editor** — Monaco-style escape hatch with roundtrip
  validation. ✅
- **Memory hotplug** — `<maxMemory slots>` + live DIMM attach. ✅

### Phase 2 — Operations polish
- **Metrics graphs** — CPU/RAM/disk-io/net-io · 1m/5m/15m/1h windows. ✅
- **Bulk actions** — cmd/shift-click multi-select + start/stop/force-off
  on the selection. ✅
- **VM cloning** — full-copy via `virStorageVolCreateXMLFrom`, MAC
  strip, start-after. ✅
- **qemu log viewer** — tails `/var/log/libvirt/qemu/<vm>.log` over SSH. ✅
- **Save / restore / screenshot / coredump** — all four shipped. ✅

### Phase 3 — Storage depth
- **Backing chain visualisation** + blockcommit/blockpull (active
  commit + pivot). ✅
- **Disk LUKS encryption** + virSecret CRUD. ✅
- **Pool auth** — iSCSI CHAP, Ceph RBD. ✅
- **More pool types** — dir, fs, netfs, logical, iscsi, iscsi-direct,
  rbd, zfs. ✅
- **Volume upload streaming** — local file → pool over virStream. ✅

### Phase 4 — Networking depth
- **DHCP reservations + DNS hostname overrides** on virtual networks. ✅
- **Static routes** on virtual networks. ✅
- **nwfilter** (firewall rules per NIC). ✅
- **Open vSwitch virtualport** on NICs. ✅

### Phase 5 — Advanced devices and security
- **Live migration** — peer-to-peer with auto-converge, bandwidth cap,
  persist/undefine flags. Multi-connection `AppState` refactor done. ✅
- **Mediated devices** — mdev / NVIDIA vGPU / vfio-mdev. ✅
- **SR-IOV** — PF/VF enumeration, attach VFs via existing PCI
  passthrough. ✅
- **Nested virt toggle** — Intel vmx / AMD svm + host kernel module
  probe. ✅
- **SEV / SEV-SNP / TDX** — SEV writable, SEV-SNP/TDX read-only. ✅
- **vTPM persistent NVRAM** path + backup snippets. ✅

### Phase 6 — Import / templates / catalog
- **VM templates** — libvirt metadata flag + clone-from-template with
  cloud-init NoCloud seed (host-side mkisofs). ✅
- **OVA / OVF import** — VMDK → qcow2 streaming via `qemu-img convert`
  over SSH stdin. ✅
- **Cloud image catalog** — Fedora / Debian / Ubuntu / Alpine, one-click
  download into a pool via SSH+curl. ✅

### Bonus — beyond the original roadmap
Things that weren't planned but ended up mattering:
- **Hypervisor dashboard** — host CPU/RAM/storage/networks at a glance.
- **Multi-hypervisor connection list** with persistence + per-conn
  error/red on socket loss.
- **Stale-connection recovery** — libvirt keepalive + auto-reconnect
  after laptop sleep / NAT idle. (v0.2.9 / v0.2.10)
- **In-app unattended auto-update** — signed bundle, minisign-verified
  swap, no DMG re-download. (v0.2.10)
- **Native menu bar** with always-available logs viewer + check-for-
  updates entry. (v0.2.6 / v0.2.10)

## What's actually next

The original phases are done. From here, work is opportunistic — driven
by daily-driver friction or specific use cases. Candidates with rough
shape:

- **Snapshot tree polish** — drag to revert, snapshot diff viewer,
  per-disk preview.
- **Network topology view** — visual graph of bridges/NICs/VMs per
  hypervisor rather than the flat list.
- **VM groups / tags** — filtering and bulk actions across arbitrary
  groupings, not just hypervisor.
- **Backups / restore UI** — wrap existing `virDomainBackupBegin` and
  push-pull-incremental backups into a first-class flow.
- **Linux + Windows update channels** — auto-update is macOS-only
  today; the signing path generalises but `latest.json` needs
  per-platform entries.
- **i18n harness** — strings are still few, easy to bolt on.
- **CHM-style remote help** — context-sensitive docs in-app for the
  more obscure libvirt knobs.

None of this is on a deadline; pick by what hurts most when using
kraftwerk for the work.

## Related docs

- [STATUS.md](STATUS.md) — current feature matrix.
- [CONFIG_ROADMAP.md](CONFIG_ROADMAP.md) — full per-domain surface
  inventory with constraints and test expectations.
- [ARCHITECTURE.md](ARCHITECTURE.md) — module layout and dataflow.
