# kraftwerk Roadmap

Plan to close the gap between kraftwerk and full libvirt surface coverage.
The per-domain hardware editor surface is largely done (see
[`CONFIG_ROADMAP.md`](CONFIG_ROADMAP.md)); this file tracks everything
*beyond* that — lifecycle, observability, advanced devices, networking
depth, storage depth, ops UX.

Ordering reflects user-impact divided by engineering effort. Phases are
not strictly sequential — independent items can ship in any order — but
each phase represents roughly a release cadence we can hit before
needing fresh feedback.

## Phase 1 — Foundations and biggest user wins

### 1.1 libvirt events stream
Replace the 3s `list_domains` poll with `virConnectDomainEventRegister`
callbacks. Domain start/stop/migrate/define/undefine surface in the UI
within tens of milliseconds instead of seconds. Free latency win across
every VM operation; also reduces idle SSH chatter dramatically.
- New module `libvirt/events.rs`: spawn a tokio task that owns a libvirt
  event-loop thread, decodes events, forwards via `mpsc::Sender`
- Tauri-level event channel pushed to the frontend (`window.emit`)
- Frontend store: subscribe on connect, mutate `vms` / `connectionStates`
  reactively
- Keep slow-poll for networks/pools/host metrics (events for those are
  patchy across libvirt versions)
- **Estimate**: 2-3 days. Unlocks "feels instant" UX.

### 1.2 Snapshots
Most-asked virt-manager feature we don't have. Two flavours:
- Internal qcow2 snapshots (single-disk VMs) — `virDomainSnapshotCreateXML`
- External (multi-disk, includes RAM if running) — disk overlay chains
- UI: per-VM snapshots tab listing tree, create/revert/delete, current
  marker, disk-space accounting
- Add `<diskSnapshot>` XML builder + `SnapshotInfo` model (name, parent,
  state, creation time, has memory, disks affected)
- Revert reminds user about side effects (NIC MAC, clock skew if external)
- **Estimate**: 4-5 days for v1 (list/create/revert/delete). Tree-view
  polish a follow-up.

### 1.3 Raw XML editor
Escape hatch for everything we don't yet model. Per-VM "Edit XML" button
opens a Monaco-style editor pre-populated with `<domain>` XML. Save calls
`virDomainDefineXML`. Live diff vs current. Accept-with-warning if user
changes things kraftwerk doesn't understand.
- Frontend: lazy-load Monaco (~300KB gzipped), or use CodeMirror 6 (~70KB)
- Validate roundtrip via libvirt before persisting
- **Estimate**: 1-2 days. Massively expands what advanced users can do.

### 1.4 Memory hotplug slots
Unblocks live max-RAM growth (the v0.1.13 limitation). Add UI for
`<maxMemory slots="N">` and per-DIMM `<memory model="dimm">` devices.
Integration test on a wg-test-style VM verifying live grow works.
- Boot-time-only setting in BootPanel: "Hotplug slots" + "Max"
- Hotplug per-DIMM via `virDomainAttachDeviceFlags` for live grow
- **Estimate**: 2 days.

## Phase 2 — Operations polish

### 2.1 Metrics graphs
We already sample CPU/memory/disk-io/net-io stats — currently shown as
sparklines (per CONFIG_ROADMAP). Promote to a "Graphs" tab with 1m / 5m /
1h / 24h windows backed by an in-memory ring buffer (per VM, ~5MB total
for typical sample density). No persistence (yet).
- Reuse existing `domain_stats` sampling
- Frontend: a small `<canvas>`-based chart (avoid pulling Chart.js)
- **Estimate**: 2 days.

### 2.2 Bulk actions
Multi-select in the sidebar (cmd-click, shift-range), then start /
shutdown / force-off / suspend / resume the selection. Useful when
recovering after host reboot.
- Sidebar: checkbox column appears on shift/cmd-modifier
- Toolbar: actions visible when selection > 1
- **Estimate**: 1-2 days.

### 2.3 VM cloning
`virt-clone` semantics: pick a shut-off VM, name the clone, choose
storage strategy (full copy / linked / reflink), MAC randomization,
guest-agent SID reset for Windows. We already have most of the building
blocks (volume copy, disk attach, NIC MAC).
- New backend: `clone_domain(source, target, opts)` that orchestrates
  volume clone + XML mutation + define
- Wizard-style modal
- **Estimate**: 3-4 days. Big QoL.

### 2.4 Domain log viewer
Tail `/var/log/libvirt/qemu/<vm>.log` over SSH for a selected VM. Saves
opening a terminal when QEMU crashes.
- Read-only with simple line buffer
- Include a "Show last 200 lines / Stream" toggle
- **Estimate**: 1 day.

### 2.5 Save / restore / screenshot / coredump
Small but useful libvirt features:
- `virDomainSave(file)` / `virDomainRestore(file)` — suspend-to-disk
- `virDomainScreenshot()` — single PNG of the current display
- `virDomainCoreDumpWithFormat()` — for guest debugging
- One menu item each in the VM detail toolbar
- **Estimate**: 1 day combined.

## Phase 3 — Storage depth

### 3.1 Backing chain visualization
Show qcow2 backing chains as a tree, with sizes. Surface
`blockcommit` (collapse a snapshot back into base) and `blockpull`
(flatten an overlay).
- Backend: parse `qemu-img info --backing-chain --output=json` over SSH
  (libvirt doesn't expose backing chains as cleanly)
- UI: simple tree in the disk panel
- **Estimate**: 2-3 days.

### 3.2 Disk encryption (LUKS) + virSecret management
qcow2 LUKS + raw LUKS volumes, with secrets managed via `virSecret`.
Currently we don't surface either.
- Backend: `virSecretDefineXML` / `virSecretSetValue` wrappers + a model
- UI: secret library (per connection) + checkbox in volume creation
- **Estimate**: 3 days.

### 3.3 Pool auth (CHAP, gluster, NFS Kerberos)
Currently only username/password for ssh-based pools is implicit. Add
proper auth modeling for iSCSI/RBD/Gluster pools. Mostly XML editing on
top of existing pool types.
- **Estimate**: 1-2 days.

### 3.4 More pool types
ZFS (zpool dataset), RBD/Ceph, Sheepdog (deprecated, skip),
btrfs subvolume.
- **Estimate**: 1-2 days, mostly serializers.

### 3.5 Volume upload/download streams
`virStorageVolUpload` / `virStorageVolDownload` with a progress bar.
Useful for getting an ISO onto a remote host without first SCPing it.
- Tauri side-channel for stream progress events
- **Estimate**: 2 days.

## Phase 4 — Networking depth

### 4.1 DNS / DHCP host entries
Static IP assignments and `/etc/hosts`-style entries inside libvirt
virtual networks. Currently only basic netcfg. Per-host MAC → IP +
hostname → IP table editor on the network detail.
- **Estimate**: 1-2 days.

### 4.2 Static routes
Routes inside virtual networks, mostly for advanced topologies with
multiple bridges.
- **Estimate**: 1 day.

### 4.3 nwfilter (firewall rules per NIC)
libvirt's per-vNIC firewall layer. List, attach to NICs, predefined
profile editor (allow-arp, no-ip-spoofing, etc.).
- **Estimate**: 3 days for v1 with built-in profile templates.

### 4.4 Open vSwitch / VLAN trunking
For homelab + lab setups using OVS bridges with VLAN-tagged ports.
- VLAN tag picker on NIC config
- OVS bridge enumeration via `ovs-vsctl` over SSH (not in libvirt API)
- **Estimate**: 2 days.

## Phase 5 — Advanced devices and security

### 5.1 Live migration (the big one)
Cross-host migration with libvirt — biggest single feature gap vs
Proxmox/VMware. Requires kraftwerk to manage *multiple* connections in
the backend simultaneously (currently single global libvirt handle).
Pre-req: refactor `AppState` to hold a `HashMap<ConnId, LibvirtConnection>`.
- Pre-flight checks: storage compatibility, CPU compatibility (if not
  using host-model), shared storage or copy-storage
- Live (peer-to-peer or via libvirtd-tunnel), offline, and copy-storage
  flavours
- Progress tracking via `virDomainMigrateGetCompressionCache` + stats
- **Estimate**: 5-7 days. Multi-connection refactor is the heavy part.

### 5.2 Mediated devices (mdev) — vGPU
Intel GVT-g and NVIDIA vGPU partitioning. Enumerate parent devices,
create/destroy mdev instances, attach to VMs.
- Backend: `virNodeDeviceCreateXML` for mdev instances
- UI: under Hardware → Host Devices, a "vGPU" subtab
- **Estimate**: 3-4 days.

### 5.3 SR-IOV VF lifecycle
We already attach existing VFs as PCI passthrough. Add: enumerate
parent PFs, configure `numvfs` sysfs, allocate VFs.
- Requires SSH `echo N > .../sriov_numvfs` on the host (no libvirt API)
- **Estimate**: 2 days.

### 5.4 Nested virt toggle
Surface as an explicit boot-config knob rather than buried in CPU
features. Also add the host-side `kvm-intel.nested=1` / `kvm-amd.nested=1`
sanity check.
- **Estimate**: half a day.

### 5.5 SEV / SEV-SNP / TDX (`<launchSecurity>`)
Confidential-computing launch types. Mostly XML serialization + a small
UI. Limited audience but it's the kind of feature that gets us listed in
"libvirt managers that support SEV".
- **Estimate**: 2-3 days for v1 (SEV first; SNP and TDX share the path).

### 5.6 vTPM NVRAM management
We attach a TPM device today but don't surface its persistent NVRAM
state — important for Windows 11 + BitLocker + secure boot.
- Backup / restore vTPM state
- Reset NVRAM (with confirm — destroys BitLocker keys)
- **Estimate**: 1-2 days.

## Phase 6 — Import / templates / catalog

### 6.1 VM templates
Mark a shut-off VM as a template; clone-from-template flow that
randomizes UUIDs, MACs, hostnames, and runs cloud-init NoCloud seed
generation.
- **Estimate**: 3 days, builds on §2.3.

### 6.2 OVA / OVF import
Parse OVF metadata, extract VMDK disks, convert with `qemu-img` over
SSH on the target host, generate domain XML. Major value for users
moving from VMware.
- **Estimate**: 3-4 days.

### 6.3 Image library / ISO catalog
Tracked + automatically downloaded cloud images (Fedora, Debian,
Ubuntu, Alpine). Pool-aware: download once per host. Cloud-init seed
generator built in.
- Optional SHA verification, signed manifest source
- **Estimate**: 4 days.

## Cross-cutting work

These are not features but should be picked up alongside the phases:

- **Multi-connection AppState refactor** (pre-req for §5.1, also fixes
  the v0.1.11 reconnect-on-switch limitation). 1-2 days standalone.
- **Frontend a11y pass**: keyboard navigation, screen-reader labels,
  proper focus ring across the editors. 2 days.
- **Test coverage expansion**: integration tests for snapshots, clone,
  migration, mdev. Likely 1 day per feature added above.
- **Per-VM permission gating**: read-only / shutdown-only / full-control
  per saved connection (defense-in-depth, not real security since the
  hypervisor already authoritative). 1 day.
- **Localization scaffold**: kraftwerk is single-language English. Add
  i18n harness now while strings are still few. 1 day.

## Estimated total

Phase 1: ~10 days · Phase 2: ~7-8 days · Phase 3: ~10 days · Phase 4: ~7
days · Phase 5: ~13-15 days · Phase 6: ~10 days · cross-cutting: ~7 days.

Roughly **9-10 person-weeks** to ship everything. Phase 1 alone (10
days) gets us past Cockpit's UX for individual-VM management; phase 1+2
(20 days) is enough to call kraftwerk a real virt-manager replacement.
