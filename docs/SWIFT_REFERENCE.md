# Swift VirtManager — Reference for Porting

This document captures the key details from the original Swift app that inform the Rust port.

## Data Models

### SavedConnection
- `id: UUID`, `displayName: String`, `uri: String`
- `authType: AuthType` — `.sshKey`, `.password`, `.sshAgent`
- `lastConnected: Date?`
- Persisted as JSON via Codable

### VMInfo
- `name`, `uuid`, `state: VMState`, `vcpus: Int`, `memoryMB: Int`
- `graphicsType: GraphicsType?` — `.vnc` or `.spice`
- `hasSerial: Bool`

### VMState
States: `running`, `paused`, `shutOff`, `crashed`, `suspended`, `unknown`

Transition rules:
- `canStart` → shutOff, crashed
- `canShutdown` → running
- `canForceOff` → running, paused, crashed, suspended
- `canPause` → running
- `canResume` → paused, suspended
- `canReboot` → running
- `canOpenConsole` → running

### ConnectionState
`disconnected`, `connecting`, `connected`, `disconnecting`, `error(String)`

## Libvirt Connection API

All operations are synchronous/blocking, protected by NSLock.

### Connection lifecycle
- `open(uri: String)` → virConnectOpen
- `close()` → virConnectClose
- `isConnected` → check pointer
- `hostname()` → virConnectGetHostname

### Domain operations
- `listAllDomains()` → virConnectListAllDomains → parse each to VMDomainInfo
- `startDomain(name)` → virDomainCreate
- `shutdownDomain(name)` → virDomainShutdown
- `destroyDomain(name)` → virDomainDestroy
- `suspendDomain(name)` → virDomainSuspend
- `resumeDomain(name)` → virDomainResume
- `rebootDomain(name)` → virDomainReboot
- `getDomainXML(name, inactive)` → virDomainGetXMLDesc
- `defineDomainXML(xml)` → virDomainDefineXML
- `undefineDomain(name)` → virDomainUndefine

### Device hot-plug
- `attachDevice(domainName, deviceXML, live, config)`
- `detachDevice(domainName, deviceXML, live, config)`
- `updateDevice(domainName, deviceXML, live, config)`

### Live resource changes
- `setMemory(domainName, memoryKB, live, config)`
- `setVcpus(domainName, count, live, config)`

### Graphics console
- `openGraphicsFD(name)` → virDomainOpenGraphicsFD → returns FD speaking VNC/SPICE

### Storage pools
- `listStoragePools()`, `listVolumes(poolName)`
- `createVolume(poolName, volumeXML)`, `deleteVolume(path)`
- `resizeVolume(path, capacityBytes)`
- `refreshPool(name)`, `createPool(xml)`
- `startPool(name)`, `stopPool(name)`, `deletePool(name)`
- `getPoolXML(name)`, `getVolumePath(poolName, volumeName)`

### Networks
- `listNetworks()`, `startNetwork(name)`, `stopNetwork(name)`
- `createNetwork(xml)`, `deleteNetwork(name)`
- `getNetworkXML(name)`, `defineNetwork(xml)`
- `undefineNetwork(name)`, `setNetworkAutostart(name, autostart)`
- `getNetworkDHCPLeases(name)`
- `updateNetworkSection(name, command, section, xml)`
- `detectOVSAvailable()`

## XML Parsing

### VMDomainInfo (parsed from virDomain)
- name from virDomainGetName
- uuid from virDomainGetUUIDString
- state from virDomainGetInfo → mapped via stateFromLibvirt()
- vcpus, memoryKB from virDomainInfo struct
- graphicsType from XML regex: `<graphics\s+type="(\w+)"`
- hasSerial: checks for `<serial type=` or `<console type=` in XML

### DomainConfig (full XML round-trip parser)
- Preserves original XMLDocument
- Parses: name, uuid, title, description, memory, vcpus, cpu config
- Parses devices: disks, NICs, graphics, video, input, sound, controllers, serial, host devices
- `toXML()` patches changes back into the document (never strips unknown elements)

### NetworkConfig
- Forward mode (nat/routed/isolated/open/bridge/macvtap/ovs)
- IPv4/IPv6 config with DHCP ranges and static entries
- DNS: forwarders, SRV records, TXT records, host records
- Port forwarding (NAT DNAT rules)
- QoS/bandwidth limits

## UI Structure (SwiftUI)

### Main layout
- NavigationSplitView: sidebar (connection list, VM tree) + detail
- SidebarView shows connections with expand/collapse, VMs with state badges
- VMDetailView: summary card + action buttons + tabbed config editor

### VM Configuration (10 tabs)
1. Overview — name, title, description
2. CPU — vCPUs, mode, model, topology
3. Memory — size in MiB
4. Boot — firmware (BIOS/EFI), boot order, machine type
5. Disks — list, add/edit/delete
6. Network — NIC list, add/edit/delete
7. Graphics/Other — graphics device config
8. USB Passthrough — USB device addresses
9. PCIe Passthrough — PCI device addresses
10. XML Editor — raw XML with validation

### VM Creation Wizard (6 steps)
1. Name & OS type/variant
2. CPU & Memory
3. Storage (new disk or existing)
4. Network
5. Install media (ISO source)
6. Review & create

### Network Management
- Network list view with state, mode, IP ranges
- 8-tab config editor (overview, IPv4, IPv6, DNS, DHCP, port forwarding, QoS, XML)
- Creation wizard (5 steps)
- Topology graph visualization (canvas-based)

### Console Windows
- ConsoleWindowController: VNC/SPICE/serial in NSWindow
- Toolbar: Ctrl+Alt+Del, keyboard grab, screenshot, fullscreen, USB devices
- Keyboard grab via CGEvent tap
- VNC: pure Swift RFB 3.8 (RFBConnection, RFBProtocol, RFBFramebuffer, RFBSecurity)
- Serial: SwiftTerm library, bidirectional LibvirtStream
- SPICE: GLib bridge on dedicated thread (deferred in Rust port)

## OS Variant Defaults

| Variant | Disk Bus | NIC | Video | Machine | Firmware |
|---------|----------|-----|-------|---------|----------|
| Linux (fedora/ubuntu/debian/centos/rhel) | virtio | virtio | virtio | q35 | bios |
| Windows 10 | sata | e1000e | qxl | q35 | efi |
| Windows 11 | virtio | e1000e | qxl | q35 | efi |
| Generic Windows | sata | e1000e | qxl | q35 | efi |
| FreeBSD | virtio | virtio | virtio | q35 | bios |

## Security Measures (from original)
- SSH host key verification
- XML escaping on all user input
- SSH component sanitization (allowlist)
- XXE protection on XML parsing
- Keychain-only credential storage
- VNC/SPICE tunneled through SSH only
