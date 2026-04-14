# VM Configuration Surface — Roadmap

Plan for completing the VM configuration / hardware surface. Based on a
scan of `libvirt.org/formatdomain.html` and
`libvirt.org/formatdomaincaps.html`, plus the Python `virt-manager`
editor categories. This doc is the source of truth for what is in /
out of scope, in what order, and why.

## Where we are

### Done
- **Connection + VM list** (Overview, VM state, lifecycle actions)
- **VM configuration tab**: name / UUID / title / description / memory /
  vCPU / CPU mode / machine / firmware / boot order (read-only parse)
- **Live resource editing**: `set_vcpus_flags`, `set_memory_flags`
- **Network management** (virtual networks list + creation wizard,
  5 forward modes)
- **Storage** (pool list/create/delete, volume create/delete/resize)
- **Creation wizard** (5-step, minimal — name, CPU/mem, disk, net, ISO)
- **Serial console** (libvirt stream → crytter WASM terminal)
- **VNC console** (SSH-tunneled, noVNC vendored)
- **SPICE console** (capsaicin, client/server mouse mode, cursor channel)
- **PCI + USB passthrough** (enumerate host devs, attach/detach, UI)

### Missing (grouped by round below)
Disk advanced props / edit, NIC advanced props / edit, graphics +
video + sound + input devices, TPM, RNG, watchdog, panic, smartcard,
filesystem passthrough, controllers (USB/SCSI/virtio-serial), channels
(qemu-ga, vdagent), vsock, IOMMU, NUMA, hugepages, CPU feature flags,
hyperv enlightenments, SEV/TDX, cloud-init seeding, import-disk flow,
libosinfo detection.

## Foundation work (prerequisite to several rounds)

### F1. `virConnectGetDomainCapabilities` bridge

Several pickers below cannot hardcode device-model lists — what works
depends on the host kernel, QEMU version, arch, and machine type.
`virConnectGetDomainCapabilities(emulator, arch, machine, virttype,
flags)` returns an XML document enumerating valid choices per host.

**What it exposes:**
- `<vcpu max='N'/>` — max vCPUs for this (arch, machine). Cap the UI.
- `<cpu>` — host-passthrough / host-model / custom with supported models
- `<os><firmware>` — BIOS vs EFI support, secure-boot, enrolled-keys
- `<devices><disk><bus>` — IDE / SCSI / virtio / SATA / usb / fdc
- `<devices><graphics><type>` — SDL / VNC / SPICE / RDP / DBUS
- `<devices><video><modelType>` — vga / cirrus / qxl / virtio / bochs / etc.
- `<devices><hostdev><subsysType>` — usb / pci / scsi
- `<devices><rng><model>` — virtio
- `<devices><tpm><model>` and `<tpm><backendModel>`
- `<devices><filesystem><driverType>` — path, handle, loop, nbd, ploop,
  virtiofs
- `<features><sev>` / `<features><sgx>` / launch security
- `<features><hyperv>` enlightenments
- `<features><gic>` version choices (ARM)

Build: `LibvirtConnection::get_domain_capabilities(arch, machine) ->
DomainCaps` returning a struct that our pickers consume to dim/hide
unsupported options. Cache per (arch, machine) — it does not change
without a host restart.

Also query `virConnectGetMaxVcpus(type)` for a hard upper bound. There
is no analogous memory call; libvirt does not cap it — the UI should
warn above host RAM (reported by `virNodeGetInfo` / `virNodeGetMemoryStats`).

### F2. Domain XML round-trip (edit-in-place)

Current `domain_config` parses into read-only types. For editing we
need a round-trip: parse, mutate specific fields, serialize back
*without losing anything we do not know about* (metadata, seclabel,
IOThreads, features we have not explicitly modelled yet, etc).

Two approaches:

1. **Typed round-trip**: parse everything, reserialize. Risky —
   drops anything not in our model. Python virt-manager got burned by
   this repeatedly.
2. **DOM-style edit**: keep the raw XML; mutate specific nodes via
   `quick-xml::Writer` + surgical find/replace on element trees.
   Preserves unknown content.

Go with #2. It matches the Swift reference's approach and the
`libvirt_xml_update` API for in-place node updates.

Task: `domain_config::apply_patch(original_xml, patch: DomainPatch) ->
String` where `DomainPatch` is a set of { target: DomainPath, value: PatchValue }.

### F3. Live vs persistent matrix

Each device type has different hotplug semantics. Libvirt's
`VIR_DOMAIN_AFFECT_LIVE` / `VIR_DOMAIN_AFFECT_CONFIG` flags only work
when QEMU supports it. Per libvirt docs (formatdomain.html), these are
**persistent-only** (require VM restart):

- TPM
- Watchdog
- Panic notifier
- NVRAM
- Some controller model changes
- CPU topology changes

These **do support live hotplug**:

- Disks (and hot-unplug, mostly)
- NICs
- USB redirection
- Memory DIMM devices (if `maxMemory` + slots pre-configured)
- vCPU hotplug (if guest kernel supports it)
- Memory balloon target
- Host device passthrough (PCI + USB, already implemented)

Encode this as a property on each `DeviceKind` so UI can show the
right affordance ("Apply now" vs "Apply on next boot").

## Rounds

Ordered by (user impact × frequency-of-use) / implementation cost.

---

### Round A — Boot, firmware, machine (small, high value)

**Scope:** edit the `<os>` block and derived features.

- Boot device ordering with drag-and-drop (hd/cdrom/network/fd)
- Boot menu: enabled + timeout
- Firmware: BIOS vs UEFI (`<loader>` + optional `<nvram>` template).
  UEFI toggles Secure Boot + `loader@secure='yes'`.
- Machine type from capabilities (`pc-q35-*`, `pc-i440fx-*`).
- `<features>` toggles: ACPI, APIC, SMM (auto-on with UEFI), PAE, HAP.
- Events: `<on_poweroff>`, `<on_reboot>`, `<on_crash>` — enum per the
  capabilities doc (destroy / restart / preserve / rename-restart /
  coredump-destroy / coredump-restart).

**Constraints**
- Secure Boot requires EFI + SMM=on.
- Machine type change requires shut-off VM.
- Boot menu is cosmetic; independent of firmware.
- `<nvram template='...'>` comes from `getDomainCapabilities` for EFI.

**Tests**
- Unit: each field round-trips through parse/patch/build.
- Integration: flip fedora-workstation firmware to BIOS and back
  (only while shut off — we won't do this against testhost's running
  VMs; use a disposable test VM).

---

### Round B — Disks (large, very high value)

**Scope:** add/edit/remove `<disk>` devices and CD-ROM media change.

- Add disk: pick existing volume OR create new in pool. Already have
  the picker components from the wizard; generalize.
- Edit: bus (virtio/sata/scsi/ide), cache (none/writethrough/writeback/
  directsync/unsafe/default), io (native/threads/io_uring), discard
  (unmap/ignore), detect_zeroes (off/on/unmap), serial string,
  readonly, shareable, removable (cdrom only), rotation_rate
  (nonrotational=1).
- CD-ROM live media change via `virDomainUpdateDeviceFlags`.
- Backing chain view (read-only for now).
- iothread assignment.

**Constraints**
- Virtio disks don't appear on a SCSI bus. Validate bus/device combos.
- `rotation_rate=1` (SSD hint) only valid on SCSI.
- `discard='unmap'` needs the backing file system / pool to support it
  (no way to know from XML alone — surface a warning).
- SCSI discard/trim requires `<driver discard='unmap'/>` + a
  `virtio-scsi` controller with `queues='N'`.
- A VM boots from the first disk by default unless `<boot order='N'/>`
  is set per disk. Coordinate with Round A boot order.
- Max disks: no hard libvirt limit, but depends on bus
  (IDE=4, SATA=6 on pc-i440fx, virtio+SCSI effectively unlimited).

**Tests**
- Unit: each property round-trips.
- Integration: live CD-ROM change on fedora-workstation (eject+insert).
- Integration: hot-add a 64 MiB qcow2 to fedora-workstation, then
  detach, then remove the volume.

---

### Round C — Network interfaces (large, very high value)

**Scope:** edit `<interface>` entries.

- Source: virtual network (picked from our network list), bridge
  (host bridge picker), direct (macvtap modes bridge/vepa/private/
  passthrough), user (SLIRP), vhostuser, ethernet, hostdev (NIC
  passthrough).
- Model: virtio / e1000 / e1000e / rtl8139 / pcnet / ne2k_pci / mvnetacard.
  Enumerate from caps.
- MAC address (generated by libvirt if omitted; show and allow edit).
- Link state: up / down.
- MTU.
- `<bandwidth>` inbound/outbound average/peak/burst.
- `<vlan>` tags (native + trunk).
- `<driver>` tuning: queues, txmode, ioeventfd, event_idx, rx_queue_size,
  tx_queue_size, iommu, ats, packed.
- `<filterref>` (nwfilter). Rare but virt-manager exposes it.
- `<port isolated='yes'/>` (OVS).

**Constraints**
- Virtio + `queues>1` requires vhost-net.
- Macvtap + user-mode networking are exclusive to everything else.
- Live hot-plug supported for all standard models.
- PCI passthrough NICs (hostdev type='network') are SR-IOV VFs; they
  need special handling (already partially in hostdev.rs).

**Tests**
- Unit: per-field round-trip.
- Integration: hot-add a second virtio NIC to fedora-workstation on
  an existing testhost network, change link state down/up, detach.

---

### Round D — Graphics, video, sound, input (medium)

**Scope:** display / user-I/O devices.

- `<graphics>`: type (vnc/spice/rdp/sdl/none), listen, port/autoport,
  tlsPort, passwd, passwdValidTo, connected, keymap, defaultMode,
  gl accel, rendernode, image compression, streamingMode, mousegrab.
- `<video>`: model type (qxl/virtio/vga/cirrus/bochs/ramfb/none),
  vram, ram, vgamem, heads, primary, blob, accel3d.
- `<sound>`: model (ich9/ich7/ich6/ac97/hda/es1370/sb16/usb),
  codec list (duplex/micro/output), streams, multichannel.
- `<input>`: mouse / keyboard / tablet / passthrough / evdev,
  bus (usb/virtio/ps2/xen).
- `<hub>` for USB tree.
- `<audio>` backend for modern spice (pulseaudio/pipewire/jack/...).

**Constraints**
- Switching graphics type on a running VM requires `device-update` and
  support varies.
- VirtIO video + OpenGL requires rendernode + DRM passthrough setup.
- SPICE + GL accel needs spice-gtk client ≥ 0.31 (capsaicin is
  catching up).
- QXL is SPICE-only; virtio works for both.
- Tablet input gives absolute positioning (what we want for SPICE
  CLIENT mouse mode).

**Tests**
- Unit + integration: change video model, toggle OpenGL.

---

### Round E — Virtio-adjacent (small, high value)

Batch of small, mostly-independent devices that each add one tab /
dialog row.

- **TPM** (`<tpm>`): model (tpm-tis / tpm-crb / tpm-spapr), backend
  (passthrough=/dev/tpm0, emulator, external). Needs
  `swtpm_setup`-provisioned state. Persistent-only.
- **RNG** (`<rng>`): model=virtio, backend (/dev/urandom, /dev/random,
  egd via tcp/unix). Rate-limited by `<rate period='ms' bytes='N'/>`.
  Live hotplug supported.
- **Watchdog** (`<watchdog>`): model (i6300esb / ib700 / diag288 for
  s390 / itco), action (reset / shutdown / poweroff / pause / dump /
  inject-nmi / none). Persistent-only.
- **Panic notifier** (`<panic>`): model (isa / pseries / hyperv / s390 /
  pvpanic). Persistent-only.
- **Balloon** (`<memballoon>`): model (virtio / virtio-transitional /
  none), autodeflate, freepage-reporting, stats period.
- **vsock** (`<vsock>`): guest CID for host↔guest socket communication.
  Usage: systemd-socket-proxyd, vhost-user.
- **IOMMU** (`<iommu>`): intel / smmuv3 / virtio. Required for nested
  passthrough.

**Constraints**
- One TPM per domain.
- One watchdog per domain.
- RNG rate limits exist to prevent guest RNG exhaustion abuse.
- vsock CID must be unique across VMs on the host (≥ 3, avoiding 0-2).

**Tests**
- Unit round-trip each.
- Integration: add RNG to fedora-workstation (live hotplug works).

---

### Round F — Console / serial / channels (medium)

**Scope:** `<serial>` / `<console>` / `<parallel>` / `<channel>`.

- Serial: type (pty/tcp/unix/file/pipe/nmdm), target type (isa-serial/
  usb-serial/pci-serial/sclp-serial), multiple ports. We already open
  the default console for our terminal emulator.
- Channel: for vdagent / qemu-guest-agent / named SPICE ports.
- Parallel: legacy; low priority but easy to include.
- Port: `<channel type='spiceport'>` for app-specific SPICE streams.

Important channel types:
- `<channel type='unix'> <target type='virtio' name='org.qemu.guest_agent.0'/>`
  — qemu-guest-agent. Enables `virsh shutdown --mode agent`, filesystem
  freeze, etc.
- `<channel type='spicevmc'> <target type='virtio' name='com.redhat.spice.0'/>`
  — SPICE vdagent. Enables clipboard, dynamic resolution, seamless
  mouse (coordinates with capsaicin).

**Constraints**
- Both vdagent and guest-agent channels require a `virtio-serial`
  controller.
- The guest needs the matching vdagent / guest-agent daemon installed.

**Tests**
- Integration: add guest-agent channel to fedora-workstation, verify
  `virsh domfsinfo` starts returning filesystems.

---

### Round G — Storage advanced + filesystem passthrough (medium)

**Scope:** things storage-adjacent that don't belong in Round B.

- Filesystem passthrough (`<filesystem>`): accessmode (passthrough/
  mapped/squash/default), driver (path/handle/loop/nbd/ploop/virtiofs).
  virtiofs is the modern choice; it needs a `<memoryBacking><access
  mode='shared'/></memoryBacking>` on the domain.
- Shared memory (`<shmem>`): ivshmem for inter-VM comms.

**Constraints**
- virtiofs needs `<memoryBacking>` shared, which in turn needs
  hugepages or regular shared memory setup.
- Filesystem hotplug supported with virtiofs; not with legacy 9p.

---

### Round H — Controllers (medium, mostly cosmetic UNLESS passthrough)

**Scope:** `<controller>` entries the user cares about.

- USB controller model: piix3-uhci, piix4-uhci, ehci, ich9-ehci1,
  ich9-uhci1/2/3, qemu-xhci, nec-xhci, none. `qemu-xhci` is the
  modern default.
- SCSI controller: virtio-scsi (with queues/ioeventfd), lsilogic,
  buslogic, etc.
- virtio-serial: ports count, controls how many channels fit.
- PCI topology: root / pcie-root-port / pcie-upstream-port / etc.
  Almost always auto-managed by libvirt; exposing it only matters
  for power users.
- CCID for smartcard.

**Constraints**
- Max USB ports per xhci = 15.
- Max channels per virtio-serial = 31.
- PCI topology on q35 is a minefield: if the user adds devices faster
  than libvirt can add PCIe root ports, libvirt adjusts; we should
  not fight it.

---

### Round I — CPU features + NUMA + hugepages (complex, low frequency)

**Scope:** advanced tuning rarely needed but expected in a
virt-manager replacement.

- `<cpu>`: features (add/remove), policy (force/require/optional/
  disable/forbid), cache mode, migratable, check.
- `<cpu><topology sockets='...' cores='...' threads='...'/>` with
  hardware-derived constraints from caps.
- `<cpu><numa>`: per-cell memory, cpus, distances.
- `<memoryBacking><hugepages>`: per-NUMA-node page sizes.
- `<memtune>`: hard_limit, soft_limit, swap_hard_limit, min_guarantee.
- `<vcpu><vcpus>` individual vCPU online/offline, hotplug.
- `<cputune>`: vcpu pinning, emulator pinning, period/quota.
- `<iothreads>` count; per-disk iothread assignment ties back to
  Round B.

**Constraints**
- NUMA cells need `<memoryBacking><access mode='shared'/>` for
  some features (vfio-pci in non-IOMMU domains).
- hugepage allocation must be pre-reserved on the host.
- cpu pinning requires affinity permissions.

---

### Round J — Security, launch, seclabel (complex, low frequency)

- `<seclabel>` type (dynamic/static/none), model (selinux/apparmor/dac),
  relabel on/off.
- `<launchSecurity>`: SEV, SEV-SNP, TDX launch measurements.
- Key wrap.
- IOMMUFD backend for newer passthrough.

---

### Round K — Creation wizard 2.0 (large, high value)

Rebuild the new-VM flow to match virt-manager's UX:

1. **Entry point**: pick from four paths
   - Local install media (ISO/CD)
   - Network install URL (kickstart / preseed / autoyast URL)
   - Import existing disk image
   - Manual (empty VM, customize before install)
2. **OS detection** via libosinfo (if we bundle it) or user-selected
   from `OSVariants`. Drives sane defaults (machine, disk bus, nic,
   firmware, CPU flags).
3. **Memory + CPU** with caps-driven maximums.
4. **Storage**: new disk OR existing, with format + size. Reuses our
   volume picker.
5. **Network**: virtual network / bridge / none. Reuses our network
   list.
6. **Advanced** (collapsed by default): firmware, boot device order,
   custom emulator, clock offset.
7. **Customize before install** toggle: if checked, after Create,
   jump into the Configuration tab with the VM in a not-yet-started
   state so the user can tweak any device round from the list above
   before first boot.
8. **Cloud-init seed generator** (Linux guests only): given a
   user-data + meta-data pair, build a CIDATA ISO, drop it in a pool,
   attach as a cdrom. virt-manager has this as a separate dialog.

**Constraints**
- Boot from CD auto-selected when install media provided, then dropped
  after first successful boot (virt-manager does this via a post-install
  hook — we can match with a follow-up Edit step).
- `import` flow differs: skip installer selection, just wire the
  existing qcow2 as the boot disk.

---

## What we defer / decline

- **libosinfo integration** (C library). Either bundle it or reimplement
  the small lookup table we already have in `os_variants.rs`. For
  now, keep our handwritten table; revisit once Round K is ready.
- **Migrate across hosts** (offline/live migration UI). Out of scope
  for the initial parity effort; libvirt's `virDomainMigrateToURI3`
  supports it but the UX is its own project.
- **Snapshot management** (`virDomainSnapshotCreateXML`). Could fit a
  Round L if there's demand.
- **Storage volume actions** (clone, upload, download, format probe).
  Partial coverage already in the storage UI; expand if needed.

## Proposed execution order

The user-facing priority suggests: **A, B, C, D, E, K** first (core
editing of the fields every VM owner actually touches, plus a
proper creation wizard), then F / G / H as polish, I / J on demand.

Round-of-rounds structure: each round is a single commit series —
parser + builder + connection method + commands + UI + tests +
docstring update — so progress is visible and reversible.

## Per-round test expectations

- Each data type: unit test for parse, unit test for build, unit test
  for round-trip, unit test for injection safety (no field allows
  untrusted strings into XML).
- Each hotplug device: integration test on testhost's
  fedora-workstation that add → verify → remove.
- Each persistent-only device: integration test that define → shut-off
  → start → verify → define again to remove. We will need a throwaway
  test VM on testhost that we can power-cycle without fear (current
  `fedora-workstation` is it, but we should consider creating a
  `virtmanager-tests` VM specifically for this so we stop using the
  user's daily driver).

## Decisions (locked in 2026-04-14)

1. **Test VM**: `fedora-workstation` is expendable. Reuse it as the
   disposable test subject for any round that needs reconfiguration /
   power cycles. No need to spin up a separate `virtmanager-tests` VM.
2. **Creation wizard entry paths**: both **local ISO install** AND
   **import existing disk** are must-haves in Round K. Network-install
   and manual paths can come later.
3. **Customize flow**: skip the Python virt-manager "customize before
   first boot" mid-wizard detour. Simpler flow: wizard creates the VM
   shut-off, then drops the user straight into the config tab. They
   edit whatever, then click Start. One consistent UI rather than a
   wizard-that-becomes-the-config-editor.
