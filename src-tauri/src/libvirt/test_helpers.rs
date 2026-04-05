/// Test helpers for libvirt module.
/// Real integration tests require a live hypervisor.
/// Unit tests use mock data or the xml_helpers directly.

use crate::models::vm::{VmInfo, VmState, GraphicsType};

/// Create a fake VmInfo for testing.
pub fn fake_vm(name: &str, state: VmState) -> VmInfo {
    VmInfo {
        name: name.to_string(),
        uuid: format!("00000000-0000-0000-0000-{:012x}", name.len()),
        state,
        vcpus: 2,
        memory_mb: 2048,
        graphics_type: Some(GraphicsType::Vnc),
        has_serial: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_vm_has_correct_fields() {
        let vm = fake_vm("test-vm", VmState::Running);
        assert_eq!(vm.name, "test-vm");
        assert_eq!(vm.state, VmState::Running);
        assert_eq!(vm.vcpus, 2);
    }
}
