# Architecture — VirtManager-RS

## Overview

VirtManager-RS is a cross-platform desktop application for managing remote KVM/QEMU virtual
machines via libvirt. It is a Rust+Tauri port of an existing Swift/macOS application.

## Layer Diagram

```
┌─────────────────────────────────────┐
│  Svelte Frontend (src/)             │
│  - Connection management UI         │
│  - VM list with state badges        │
│  - VM detail / configuration        │
│  - Console windows                  │
├─────────────────────────────────────┤
│  Tauri IPC (invoke commands)        │
├─────────────────────────────────────┤
│  Commands Layer (commands/)         │
│  - Thin adapters, no business logic │
│  - Validates input, calls services  │
├─────────────────────────────────────┤
│  App State (app_state.rs)           │
│  - Saved connections (Mutex<Vec>)   │
│  - Connection states (Mutex<Map>)   │
│  - Owns LibvirtConnection           │
├─────────────────────────────────────┤
│  Libvirt Layer (libvirt/)           │
│  - connection.rs: virt crate wrapper│
│  - xml_helpers.rs: parsing utils    │
│  - Future: domain_config, network   │
├─────────────────────────────────────┤
│  virt crate → libvirt C API         │
│  (linked via pkg-config)            │
└─────────────────────────────────────┘
```

## Design Decisions

### Why Tauri over Electron?
- Rust backend — direct libvirt FFI, no Node.js overhead
- Smaller binaries (~10MB vs 100MB+)
- Native OS integration via Rust ecosystem
- Security: Rust memory safety for the critical libvirt/XML handling layer

### Why `virt` crate over raw FFI?
- The `virt` crate provides safe Rust bindings to libvirt's C API
- Handles memory management (free/close) via Drop
- Type-safe enums for domain states, flags, etc.
- Well-maintained, tracks libvirt releases

### Error Strategy
- Single `VirtManagerError` enum for all errors (no nested error types)
- Implements `Serialize` to produce structured JSON payloads for the frontend
- Each variant carries an error code, message, and recovery suggestion
- Frontend can pattern-match on `code` field for specific handling

### Threading Model
- `LibvirtConnection` wraps `virt::Connect` in a `Mutex<Option<Connect>>`
- All libvirt calls are blocking — called from Tauri's async runtime
- One connection at a time (matches original app design)
- Future: may support multiple connections via `HashMap<Uuid, LibvirtConnection>`

### State Management
- Backend: `AppState` with interior mutability (Mutex)
- Frontend: Svelte stores, synced via Tauri invoke calls
- State changes emit Tauri events for reactive UI updates (planned)

## Module Responsibilities

| Module | Responsibility | Depends On |
|--------|---------------|------------|
| `models::vm` | VM data types, state transitions | nothing |
| `models::connection` | Connection config types | nothing |
| `models::error` | Error types + serialization | nothing |
| `models::state` | Connection state enum | nothing |
| `libvirt::connection` | Libvirt API wrapper | `virt`, `models` |
| `libvirt::xml_helpers` | XML parsing/escaping | `regex` |
| `app_state` | Global state container | `libvirt`, `models` |
| `commands::connection` | Tauri handlers for connections | `app_state` |
| `commands::domain` | Tauri handlers for VMs | `app_state` |

## Security Invariants

1. All user-provided strings are escaped via `escape_xml()` before XML interpolation
2. Credentials stored via `keyring` crate (OS keychain), never in config files
3. libvirt connections use SSH transport (`qemu+ssh://`) — never unencrypted
4. No `unsafe` code in application layer; only in `virt` crate internals

## Original Swift App Reference

The original app (~/Developer/perso/virtmanager) provides the feature specification:
- 77 Swift files, ~12K lines across 7 modules
- SwiftUI + AppKit hybrid for UI
- Pure Swift VNC (RFB 3.8) client
- SPICE via GLib bridge (deferred in Rust port)
- 10-tab VM configuration editor
- 6-step VM creation wizard
- 8-tab network configuration editor
- Network topology visualization
- Serial console via SwiftTerm
