# Status

Working daily driver against a real KVM host. **461 unit tests + 26
integration tests** against live hypervisors (per-domain + memory +
vCPU + networking + advanced devices + live migration + templates +
cloud image catalog + OVA import), all green.

Feature parity snapshot with Python virt-manager:

| Area | Status |
|------|--------|
| Connect / list / lifecycle (start/stop/pause/resume/reboot/force-off) | ✅ |
| Multi-hypervisor connection list (right-click edit, persistence, error/red on socket loss) | ✅ |
| Hypervisor dashboard (host CPU/RAM/storage/networks at a glance) | ✅ |
| libvirt event-driven UI (push state changes, no 3s polling) | ✅ |
| Stale-connection recovery (keepalive + auto-reconnect after sleep / NAT idle) | ✅ |
| In-app unattended auto-update (signed bundle, no DMG re-download) | ✅ |
| Live CPU + memory editing (current + max) | ✅ |
| Memory hotplug — `<maxMemory slots>` + live DIMM attach | ✅ |
| Snapshots — list/create/revert/delete with parent-child tree, VFIO-aware (disk-only + quiesce toggles) | ✅ |
| Raw domain-XML editor (escape hatch for unmodelled fields) | ✅ |
| Serial console (crytter WASM terminal) | ✅ |
| VNC console (noVNC, SSH-tunneled) | ✅ |
| SPICE console ([capsaicin](https://github.com/calibrae/capsaicin), native-Rust, with cursor + absolute mouse) | ✅ |
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
| VM templates (libvirt metadata flag) + clone-from-template with cloud-init NoCloud seed (host-side mkisofs) | ✅ |
| Cloud image catalog (Fedora / Debian / Ubuntu / Alpine) — one-click download into a pool via SSH+curl | ✅ |
| OVA / OVF import (VMDK → qcow2 streaming via `qemu-img convert` over SSH stdin) | ✅ |

See [ROADMAP.md](ROADMAP.md) for the multi-phase plan beyond per-domain
config (events, snapshots, raw XML, hotplug were phases 1 + 2 + 3 + 4
+ 5 + 6 — now done), and [CONFIG_ROADMAP.md](CONFIG_ROADMAP.md) for the
full surface inventory with constraints and test expectations.
