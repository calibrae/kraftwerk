use serde::{Deserialize, Serialize};

/// VM power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmState {
    Running,
    Paused,
    ShutOff,
    Crashed,
    Suspended,
    Unknown,
}

impl VmState {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::ShutOff => "Shut Off",
            Self::Crashed => "Crashed",
            Self::Suspended => "Suspended",
            Self::Unknown => "Unknown",
        }
    }

    pub fn can_start(&self) -> bool {
        matches!(self, Self::ShutOff | Self::Crashed)
    }

    pub fn can_shutdown(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn can_force_off(&self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Crashed | Self::Suspended)
    }

    pub fn can_pause(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn can_resume(&self) -> bool {
        matches!(self, Self::Paused | Self::Suspended)
    }

    pub fn can_reboot(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn can_open_console(&self) -> bool {
        matches!(self, Self::Running)
    }

    /// Map libvirt domain state integer to VmState.
    pub fn from_libvirt(state: u32) -> Self {
        // libvirt constants: 1=running, 3=paused, 5=shutoff, 6=crashed, 7=pmsuspended
        match state {
            1 => Self::Running,
            3 => Self::Paused,
            5 => Self::ShutOff,
            6 => Self::Crashed,
            7 => Self::Suspended,
            _ => Self::Unknown,
        }
    }
}

/// Graphics device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsType {
    Vnc,
    Spice,
}

/// A virtual machine's summary info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmInfo {
    pub name: String,
    pub uuid: String,
    pub state: VmState,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub graphics_type: Option<GraphicsType>,
    pub has_serial: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_transitions_shut_off() {
        let s = VmState::ShutOff;
        assert!(s.can_start());
        assert!(!s.can_shutdown());
        assert!(!s.can_pause());
        assert!(!s.can_resume());
        assert!(!s.can_open_console());
    }

    #[test]
    fn state_transitions_running() {
        let s = VmState::Running;
        assert!(!s.can_start());
        assert!(s.can_shutdown());
        assert!(s.can_pause());
        assert!(s.can_reboot());
        assert!(s.can_open_console());
        assert!(s.can_force_off());
    }

    #[test]
    fn state_transitions_paused() {
        let s = VmState::Paused;
        assert!(s.can_resume());
        assert!(s.can_force_off());
        assert!(!s.can_start());
        assert!(!s.can_shutdown());
    }

    #[test]
    fn from_libvirt_maps_known_states() {
        assert_eq!(VmState::from_libvirt(1), VmState::Running);
        assert_eq!(VmState::from_libvirt(3), VmState::Paused);
        assert_eq!(VmState::from_libvirt(5), VmState::ShutOff);
        assert_eq!(VmState::from_libvirt(6), VmState::Crashed);
        assert_eq!(VmState::from_libvirt(7), VmState::Suspended);
        assert_eq!(VmState::from_libvirt(99), VmState::Unknown);
    }

    #[test]
    fn display_names() {
        assert_eq!(VmState::Running.display_name(), "Running");
        assert_eq!(VmState::ShutOff.display_name(), "Shut Off");
    }

    #[test]
    fn graphics_type_serialization() {
        let json = serde_json::to_string(&GraphicsType::Vnc).unwrap();
        assert_eq!(json, "\"vnc\"");
        let json = serde_json::to_string(&GraphicsType::Spice).unwrap();
        assert_eq!(json, "\"spice\"");
    }
}
