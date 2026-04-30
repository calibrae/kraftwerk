//! `virConnectGetDomainCapabilities` wrapper + XML parse.
//!
//! The returned document advertises exactly what machine types, CPU
//! modes/models, firmware paths, device models, and feature sets are
//! supported by the host's kernel + QEMU + libvirt combination. Use it
//! to populate pickers in the UI so we never offer a choice that
//! would fail at define time.

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::models::error::VirtManagerError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomainCaps {
    pub emulator: String,
    pub arch: String,
    pub machine: String,
    pub domain_type: String,
    pub max_vcpus: u32,
    pub iothreads_supported: bool,
    pub os: OsCaps,
    pub cpu: CpuCaps,
    pub devices: DeviceCaps,
    pub features: FeatureCaps,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OsCaps {
    pub firmware_values: Vec<String>,
    pub loader_paths: Vec<String>,
    pub loader_type_values: Vec<String>,
    pub loader_secure_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuCaps {
    pub modes_supported: Vec<String>,
    pub custom_models: Vec<String>,
    pub host_model_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceCaps {
    pub disk_devices: Vec<String>,
    pub disk_buses: Vec<String>,
    pub disk_models: Vec<String>,
    pub graphics_types: Vec<String>,
    pub video_models: Vec<String>,
    pub hostdev_modes: Vec<String>,
    pub hostdev_subsys_types: Vec<String>,
    pub rng_models: Vec<String>,
    pub rng_backend_models: Vec<String>,
    pub filesystem_driver_types: Vec<String>,
    pub tpm_models: Vec<String>,
    pub tpm_backend_models: Vec<String>,
    pub tpm_backend_versions: Vec<String>,
    pub channel_types: Vec<String>,
    pub console_types: Vec<String>,
    pub net_models: Vec<String>,
    pub audio_types: Vec<String>,
    pub watchdog_models: Vec<String>,
    pub panic_models: Vec<String>,
    pub redirdev_buses: Vec<String>,
    pub launch_security_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeatureCaps {
    pub gic_versions: Vec<String>,
    pub sev_supported: bool,
    pub sgx_supported: bool,
    pub hyperv_values: Vec<String>,
    /// SEV C-bit position. Required to populate `<launchSecurity>` for
    /// SEV/SEV-ES. Read from host caps `<sev><cbitpos>`.
    pub sev_cbitpos: Option<u32>,
    /// SEV reduced-phys-bits delta. Read from `<sev><reducedPhysBits>`.
    pub sev_reduced_phys_bits: Option<u32>,
    /// SEV-SNP supported (libvirt 9.5+ exposes `<sev><maxSnpGuests>` or
    /// the launchSecurity enum includes "sev-snp").
    pub sev_snp_supported: bool,
}

/// Parse a `<domainCapabilities>` XML document.
pub fn parse(xml: &str) -> Result<DomainCaps, VirtManagerError> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut caps = DomainCaps::default();

    // Proper stack — only mutated on Start / End, Empty never pushes.
    let mut path: Vec<String> = Vec::new();

    // Current <enum name='...'> — only valid inside a device section.
    let mut current_enum: Option<String> = None;
    // Current <mode name='...' supported='...'> — only valid inside <cpu>.
    let mut current_cpu_mode: Option<String> = None;

    // What the next Text event targets.
    let mut capturing: Option<TextTarget> = None;

    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Err(e) => {
                return Err(VirtManagerError::XmlParsingFailed {
                    reason: format!("at pos {}: {}", r.buffer_position(), e),
                })
            }
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                on_start(&n, &a, &path, &mut caps, &mut capturing, &mut current_enum, &mut current_cpu_mode);
                path.push(n);
            }

            Ok(Event::Empty(e)) => {
                let n = utf8_name(&e);
                let a = attrs(&e);
                // Treat as start-without-push. Almost no self-closing element
                // has text content we need, so we never have to "clear capture"
                // on a synthetic End for Empty.
                on_start(&n, &a, &path, &mut caps, &mut capturing, &mut current_enum, &mut current_cpu_mode);
                // Clear text target since Empty has no text.
                capturing = None;
                // And if this Empty was a <mode .../> with no children, clear
                // the CPU-mode context immediately (the </mode> End never comes).
                if n == "mode" && path.last().map(String::as_str) == Some("cpu") {
                    current_cpu_mode = None;
                }
            }

            Ok(Event::End(e)) => {
                let n = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if n == "enum" {
                    current_enum = None;
                } else if n == "mode" && path.last().map(String::as_str) == Some("mode")
                    && path.get(path.len().saturating_sub(2)).map(String::as_str) == Some("cpu")
                {
                    current_cpu_mode = None;
                } else if n == "mode" {
                    // End of a <mode> whose Start was inside <cpu>.
                    if current_cpu_mode.is_some() { current_cpu_mode = None; }
                }
                path.pop();
                capturing = None;
            }

            Ok(Event::Text(t)) => {
                let txt = t.unescape().unwrap_or_default().to_string();
                if let Some(target) = capturing {
                    match target {
                        TextTarget::Emulator => caps.emulator = txt,
                        TextTarget::Machine => caps.machine = txt,
                        TextTarget::Arch => caps.arch = txt,
                        TextTarget::Domain => caps.domain_type = txt,
                        TextTarget::HostModelName => caps.cpu.host_model_name = Some(txt),
                        TextTarget::CustomModelName => caps.cpu.custom_models.push(txt),
                        TextTarget::EnumValue => {
                            if let Some(en) = current_enum.as_deref() {
                                dispatch_enum_value(&mut caps, &path, en, &txt);
                            }
                        }
                        TextTarget::LoaderPath => caps.os.loader_paths.push(txt),
                        TextTarget::SevCbitpos => {
                            caps.features.sev_cbitpos = txt.trim().parse().ok();
                        }
                        TextTarget::SevReducedPhysBits => {
                            caps.features.sev_reduced_phys_bits = txt.trim().parse().ok();
                        }
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(caps)
}

/// Handler shared by Start and Empty — interpret the element and set
/// up whatever state (capture target, enum context) the text handler
/// or child elements will need.
#[allow(clippy::too_many_arguments)]
fn on_start(
    n: &str,
    a: &[(String, String)],
    path: &[String],
    caps: &mut DomainCaps,
    capturing: &mut Option<TextTarget>,
    current_enum: &mut Option<String>,
    current_cpu_mode: &mut Option<String>,
) {
    let parent = path.last().map(String::as_str);
    let attr = |k: &str| a.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone());

    match (parent, n) {
        (Some("domainCapabilities"), "path") => *capturing = Some(TextTarget::Emulator),
        (Some("domainCapabilities"), "domain") => *capturing = Some(TextTarget::Domain),
        (Some("domainCapabilities"), "machine") => *capturing = Some(TextTarget::Machine),
        (Some("domainCapabilities"), "arch") => *capturing = Some(TextTarget::Arch),
        (Some("domainCapabilities"), "vcpu") => {
            if let Some(m) = attr("max").and_then(|v| v.parse().ok()) {
                caps.max_vcpus = m;
            }
        }
        (Some("domainCapabilities"), "iothreads") => {
            caps.iothreads_supported = attr("supported").as_deref() == Some("yes");
        }

        // CPU modes: <mode name='...' supported='yes'>[...]</mode>
        (Some("cpu"), "mode") => {
            let mname = attr("name").unwrap_or_default();
            if attr("supported").as_deref() == Some("yes") {
                caps.cpu.modes_supported.push(mname.clone());
            }
            *current_cpu_mode = Some(mname);
        }
        // host-model sub-element
        (Some("mode"), "model") => {
            match current_cpu_mode.as_deref() {
                Some("host-model") => *capturing = Some(TextTarget::HostModelName),
                Some("custom") => *capturing = Some(TextTarget::CustomModelName),
                _ => {}
            }
        }

        // OS loader: direct <value> children are loader paths.
        (Some("loader"), "value") => *capturing = Some(TextTarget::LoaderPath),

        // Enums: remember which enum we're in.
        (_, "enum") => {
            *current_enum = attr("name");
        }
        (Some("enum"), "value") => {
            *capturing = Some(TextTarget::EnumValue);
        }

        // SEV / SGX on/off flags.
        (Some("features"), "sev") => {
            caps.features.sev_supported = attr("supported").as_deref() == Some("yes");
        }
        (Some("features"), "sgx") => {
            caps.features.sgx_supported = attr("supported").as_deref() == Some("yes");
        }

        // SEV detail children — text capture handled via TextTarget.
        (Some("sev"), "cbitpos") => *capturing = Some(TextTarget::SevCbitpos),
        (Some("sev"), "reducedPhysBits") => *capturing = Some(TextTarget::SevReducedPhysBits),
        (Some("sev"), "maxSnpGuests") => caps.features.sev_snp_supported = true,

        _ => {}
    }
}

#[derive(Clone, Copy)]
enum TextTarget {
    Emulator, Machine, Arch, Domain,
    HostModelName, CustomModelName,
    EnumValue, LoaderPath,
    SevCbitpos, SevReducedPhysBits,
}

fn utf8_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes().filter_map(|a| a.ok()).map(|a| (
        String::from_utf8_lossy(a.key.as_ref()).to_string(),
        a.unescape_value().unwrap_or_default().to_string(),
    )).collect()
}

/// Classify an `<enum name='...'><value>x</value></enum>` by walking
/// up `path` to find which device / section we're inside.
fn dispatch_enum_value(caps: &mut DomainCaps, path: &[String], enum_name: &str, value: &str) {
    // The enum's parent element is path[-2] (path[-1] is "enum").
    // Actually for a <value> inside <enum> inside <disk>:
    //   path = ["domainCapabilities", "devices", "disk", "enum"]
    // So the container is path[-2] = "disk".
    let container = if path.len() >= 3 {
        path[path.len() - 3].as_str()
    } else {
        ""
    };

    match (container, enum_name) {
        ("loader", "type") => caps.os.loader_type_values.push(value.to_string()),
        ("loader", "secure") => caps.os.loader_secure_values.push(value.to_string()),
        ("os", "firmware") => caps.os.firmware_values.push(value.to_string()),
        ("disk", "diskDevice") => caps.devices.disk_devices.push(value.to_string()),
        ("disk", "bus") => caps.devices.disk_buses.push(value.to_string()),
        ("disk", "model") => caps.devices.disk_models.push(value.to_string()),
        ("graphics", "type") => caps.devices.graphics_types.push(value.to_string()),
        ("video", "modelType") => caps.devices.video_models.push(value.to_string()),
        ("hostdev", "mode") => caps.devices.hostdev_modes.push(value.to_string()),
        ("hostdev", "subsysType") => caps.devices.hostdev_subsys_types.push(value.to_string()),
        ("rng", "model") => caps.devices.rng_models.push(value.to_string()),
        ("rng", "backendModel") => caps.devices.rng_backend_models.push(value.to_string()),
        ("filesystem", "driverType") => caps.devices.filesystem_driver_types.push(value.to_string()),
        ("tpm", "model") => caps.devices.tpm_models.push(value.to_string()),
        ("tpm", "backendModel") => caps.devices.tpm_backend_models.push(value.to_string()),
        ("tpm", "backendVersion") => caps.devices.tpm_backend_versions.push(value.to_string()),
        ("channel", "type") => caps.devices.channel_types.push(value.to_string()),
        ("console", "type") => caps.devices.console_types.push(value.to_string()),
        ("net", _) => caps.devices.net_models.push(value.to_string()),
        ("audio", "type") => caps.devices.audio_types.push(value.to_string()),
        ("watchdog", "model") => caps.devices.watchdog_models.push(value.to_string()),
        ("panic", "model") => caps.devices.panic_models.push(value.to_string()),
        ("redirdev", "bus") => caps.devices.redirdev_buses.push(value.to_string()),
        ("launchSecurity", "sectype") => caps.devices.launch_security_types.push(value.to_string()),
        ("gic", "version") => caps.features.gic_versions.push(value.to_string()),
        ("hyperv", _) => caps.features.hyperv_values.push(value.to_string()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<domainCapabilities>
  <path>/usr/bin/qemu-system-x86_64</path>
  <domain>kvm</domain>
  <machine>pc-i440fx-9.2</machine>
  <arch>x86_64</arch>
  <vcpu max='255'/>
  <iothreads supported='yes'/>
  <os supported='yes'>
    <enum name='firmware'>
      <value>efi</value>
    </enum>
    <loader supported='yes'>
      <value>/usr/share/edk2/ovmf/OVMF_CODE_4M.qcow2</value>
      <value>/usr/share/edk2/ovmf/OVMF_CODE.fd</value>
      <enum name='type'>
        <value>rom</value>
        <value>pflash</value>
      </enum>
      <enum name='secure'>
        <value>no</value>
      </enum>
    </loader>
  </os>
  <cpu>
    <mode name='host-passthrough' supported='yes'/>
    <mode name='host-model' supported='yes'>
      <model fallback='forbid'>Denverton</model>
    </mode>
    <mode name='custom' supported='yes'>
      <model usable='yes'>qemu64</model>
      <model usable='yes'>kvm64</model>
    </mode>
    <mode name='maximum' supported='yes'/>
  </cpu>
  <devices>
    <disk supported='yes'>
      <enum name='diskDevice'>
        <value>disk</value>
        <value>cdrom</value>
      </enum>
      <enum name='bus'>
        <value>virtio</value>
        <value>sata</value>
      </enum>
    </disk>
    <graphics supported='yes'>
      <enum name='type'>
        <value>vnc</value>
        <value>spice</value>
      </enum>
    </graphics>
    <video supported='yes'>
      <enum name='modelType'>
        <value>virtio</value>
        <value>qxl</value>
      </enum>
    </video>
    <rng supported='yes'>
      <enum name='model'>
        <value>virtio</value>
      </enum>
      <enum name='backendModel'>
        <value>random</value>
        <value>egd</value>
      </enum>
    </rng>
    <filesystem supported='yes'>
      <enum name='driverType'>
        <value>path</value>
        <value>virtiofs</value>
      </enum>
    </filesystem>
    <tpm supported='yes'>
      <enum name='model'>
        <value>tpm-tis</value>
        <value>tpm-crb</value>
      </enum>
      <enum name='backendModel'>
        <value>emulator</value>
      </enum>
    </tpm>
  </devices>
  <features>
    <gic supported='no'/>
    <sev supported='no'/>
  </features>
</domainCapabilities>
"#;

    #[test]
    fn parses_top_level_fields() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.emulator, "/usr/bin/qemu-system-x86_64");
        assert_eq!(c.arch, "x86_64");
        assert_eq!(c.machine, "pc-i440fx-9.2");
        assert_eq!(c.domain_type, "kvm");
        assert_eq!(c.max_vcpus, 255);
        assert!(c.iothreads_supported);
    }

    #[test]
    fn parses_os_firmware_and_loader_paths() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.os.firmware_values, vec!["efi"]);
        assert!(c.os.loader_paths.iter().any(|p| p.contains("OVMF_CODE")));
        assert!(c.os.loader_type_values.contains(&"pflash".to_string()));
        assert!(c.os.loader_secure_values.contains(&"no".to_string()));
    }

    #[test]
    fn parses_cpu_modes_and_custom_models() {
        let c = parse(SAMPLE).unwrap();
        assert!(c.cpu.modes_supported.contains(&"host-passthrough".to_string()));
        assert!(c.cpu.modes_supported.contains(&"host-model".to_string()));
        assert!(c.cpu.modes_supported.contains(&"custom".to_string()));
        assert_eq!(c.cpu.host_model_name.as_deref(), Some("Denverton"));
        assert!(c.cpu.custom_models.contains(&"qemu64".to_string()));
        assert!(c.cpu.custom_models.contains(&"kvm64".to_string()));
    }

    #[test]
    fn parses_disk_enums() {
        let c = parse(SAMPLE).unwrap();
        assert!(c.devices.disk_devices.contains(&"cdrom".to_string()));
        assert!(c.devices.disk_buses.contains(&"virtio".to_string()));
        assert!(c.devices.disk_buses.contains(&"sata".to_string()));
    }

    #[test]
    fn parses_graphics_video_rng_filesystem_tpm() {
        let c = parse(SAMPLE).unwrap();
        assert!(c.devices.graphics_types.contains(&"spice".to_string()));
        assert!(c.devices.video_models.contains(&"virtio".to_string()));
        assert!(c.devices.rng_backend_models.contains(&"egd".to_string()));
        assert!(c.devices.filesystem_driver_types.contains(&"virtiofs".to_string()));
        assert!(c.devices.tpm_models.contains(&"tpm-tis".to_string()));
        assert_eq!(c.devices.tpm_backend_models, vec!["emulator"]);
    }

    #[test]
    fn features_sev_defaults_false() {
        let c = parse(SAMPLE).unwrap();
        assert!(!c.features.sev_supported);
    }

    #[test]
    fn empty_input_returns_default_caps() {
        let c = parse("<domainCapabilities/>").unwrap();
        assert_eq!(c.max_vcpus, 0);
        assert!(c.devices.disk_buses.is_empty());
    }

    #[test]
    fn invalid_xml_returns_error() {
        assert!(parse("<not-xml").is_err());
    }
}
