# VirtManager-RS

## What This Is

A cross-platform virtual machine manager built with **Rust (Tauri)** backend and **Svelte** frontend.
This is a **Rust port** of an existing Swift/macOS app (VirtManager). The original manages remote
KVM/QEMU VMs over SSH via libvirt.

## Architecture

```
src-tauri/src/
├── lib.rs              # Tauri app entry, command registration
├── app_state.rs        # Global state (libvirt conn, saved connections)
├── models/             # Data types, serializable for frontend
│   ├── connection.rs   # SavedConnection, AuthType
│   ├── vm.rs           # VmInfo, VmState, GraphicsType
│   ├── error.rs        # VirtManagerError + serializable ErrorPayload
│   └── state.rs        # ConnectionState enum
├── libvirt/            # libvirt wrapper layer
│   ├── connection.rs   # LibvirtConnection — thread-safe virt crate wrapper
│   ├── xml_helpers.rs  # XML parsing utilities (graphics detection, escaping)
│   └── test_helpers.rs # Test factories
└── commands/           # Tauri command handlers (thin layer over libvirt/)
    ├── connection.rs   # add/remove/connect/disconnect
    └── domain.rs       # list/start/shutdown/destroy/suspend/resume/reboot
```

Frontend: `src/` — SvelteKit app.

## Development Principles

- **SOLID**: Single responsibility per module. Libvirt wrapper knows nothing about Tauri.
  Commands are thin adapters. Models are plain data.
- **Design Patterns**: Repository pattern for connections. Strategy for auth. Observer via
  Tauri events for state changes.
- **Unit Tests**: Every module has `#[cfg(test)]` tests. Run `cargo test` frequently.
  Integration tests requiring a live hypervisor are gated behind feature flags.
- **Error Handling**: Use `VirtManagerError` for all errors. Never panic in library code.
  Errors serialize to structured JSON for the frontend.
- **Thread Safety**: LibvirtConnection uses Mutex. All libvirt calls are blocking —
  Tauri handles async dispatch.
- **Security**: Always escape XML user input via `xml_helpers::escape_xml()`.
  Never interpolate raw strings into XML or shell commands.

## Key Dependencies

- `virt` — Rust libvirt bindings (wraps libvirt C API)
- `tauri` v2 — Desktop app framework
- `serde` — Serialization for frontend IPC
- `thiserror` — Ergonomic error types
- `keyring` — Cross-platform credential storage
- `quick-xml` — XML parsing (for domain/network config, coming later)
- `regex` — XML helpers

## Implementation Roadmap (step by step)

1. **Connect + list VMs** ← DONE (current state)
2. VM detail view (show domain XML parsed info)
3. VM serial console (connect to serial stream)
4. VM lifecycle actions (start/stop/pause/resume from UI)
5. VM configuration editor
6. Network management
7. Storage management
8. VM creation wizard
9. VNC console
10. SPICE console (deferred)

## Porting Reference

The original Swift app lives at `~/Developer/perso/virtmanager` (same machine or local).
Key mappings from Swift → Rust:

| Swift | Rust |
|-------|------|
| `LibvirtConnection` (NSLock + virConnectPtr) | `LibvirtConnection` (Mutex + virt::Connect) |
| `VMInfo` / `VMState` | `VmInfo` / `VmState` |
| `SavedConnection` (Codable) | `SavedConnection` (Serialize/Deserialize) |
| `ConnectionError` / `LibvirtError` | `VirtManagerError` (unified) |
| `CredentialStore` (macOS Keychain) | `keyring` crate (cross-platform) |
| `XMLHelpers` | `libvirt::xml_helpers` |
| `AppState` (@Observable) | `AppState` (Tauri managed state + Mutex) |
| SwiftUI views | Svelte components |
| `virDomainOpenGraphicsFD` → VNC FD | TBD — may use websocket proxy |

## Commands

```bash
# Run dev
npm run tauri dev

# Run Rust tests
cd src-tauri && cargo test

# Build release
npm run tauri build
```

## Git

- No `Co-Authored-By` in commits
- Descriptive but concise commit messages

## Vault (Secrets Management)
HashiCorp Vault on mista (10.10.0.3:8200) stores all infra secrets.

### Access
- **URL**: `http://10.10.0.3:8200`
- **Read-only token**: in `$VAULT_TOKEN` env var on speedwagon
- **Read-write token**: stored at `~/.vault/rw_token` on speedwagon (never commit)
- **Paths**: `secret/data/infra/*` and `secret/data/nxp/*`

### Reading a secret
```bash
curl -s -H "X-Vault-Token: $VAULT_TOKEN" http://10.10.0.3:8200/v1/secret/data/infra/<name> | jq '.data.data'
```

### Relevant secrets for Kraftwerk
- `secret/infra/default` — default password for all machines
- `secret/infra/example-firewall` — OPNsense API keys
- `secret/infra/telegram` — bot tokens + chat ID
- `secret/infra/hass` — Home Assistant token
- `secret/infra/mqtt` — MQTT credentials
- `secret/infra/gitea` — Gitea API token
- `secret/infra/unifi-ap` — UniFi AP SSH credentials

### Hypervisor connections
Kraftwerk manages VMs on:
- **polnareff** (10.10.0.7) — KVM/libvirt, 9 VMs, Terraform-managed
- **testhost** (192.0.2.1) — KVM/libvirt, manual VMs (HASS, brokers, UniFi, OPNsense)
- **doppio** (10.10.0.12) — KVM/libvirt, Fedora 43, GPU passthrough (mira Windows VM)

SSH credentials for all hypervisors are in Vault at `secret/infra/default`. User `cali`, key `~/.ssh/cali_net_rsa`.

### In Rust
Use `reqwest` to call Vault API:
```rust
let token = std::env::var("VAULT_TOKEN")?;
let resp: serde_json::Value = reqwest::Client::new()
    .get("http://10.10.0.3:8200/v1/secret/data/infra/default")
    .header("X-Vault-Token", &token)
    .send().await?
    .json().await?;
let password = resp["data"]["data"]["password"].as_str().unwrap();
```
