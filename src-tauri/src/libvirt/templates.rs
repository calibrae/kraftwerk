//! VM templates: a "template" is an ordinary shut-off domain marked
//! with a kraftwerk metadata flag. Templates surface in a separate
//! catalog in the UI and feed the clone-from-template flow which
//! optionally seeds the new guest with a cloud-init NoCloud ISO.
//!
//! libvirt's `<metadata>` block accepts arbitrary user XML scoped by
//! namespace. We store ours under a fixed namespace so list / probe
//! ops are stable across libvirt restarts and survive `virsh dumpxml`
//! round-trips.
//!
//! Cloud-init NoCloud requires three files in an ISO with the
//! `cidata` filesystem label:
//! - `meta-data` (instance-id + local-hostname)
//! - `user-data` (#cloud-config payload: users, ssh keys, runcmd…)
//! - `network-config` (optional v2 netplan)
//!
//! We build the file *contents* in Rust; the actual ISO is created on
//! the hypervisor host via SSH because that's where the resulting
//! CD-ROM image needs to land (a libvirt-managed pool path).

use serde::{Deserialize, Serialize};

use crate::libvirt::xml_helpers::escape_xml;

/// Namespace + element marking a domain as a kraftwerk template.
pub const TEMPLATE_NS_URI: &str = "https://github.com/calibrae/kraftwerk/template";
pub const TEMPLATE_NS_PREFIX: &str = "kraftwerk";
pub const TEMPLATE_ELEMENT: &str = "template";

/// True when the domain XML has our template marker. Implemented as
/// a substring check rather than full XML parse — the namespace + tag
/// shape is stable enough that this is fine.
pub fn is_template(xml: &str) -> bool {
    xml.contains(TEMPLATE_NS_URI)
        && xml.contains(&format!("<{TEMPLATE_NS_PREFIX}:{TEMPLATE_ELEMENT}"))
}

/// User-supplied cloud-init seed. Each field is optional so the
/// caller can ship a minimal `meta-data`-only ISO when they just want
/// instance-id + hostname randomisation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudInitConfig {
    /// `local-hostname` set in meta-data + `hostname` in user-data.
    pub hostname: Option<String>,
    /// Username to provision (default `cali` if empty).
    pub username: Option<String>,
    /// One ssh public key per entry. Each gets added to
    /// `ssh_authorized_keys` for the user.
    pub ssh_authorized_keys: Vec<String>,
    /// Hashed password (e.g. `mkpasswd --method=SHA-512`). Plain text
    /// is rejected by cloud-init unless `passwd:` is used (which we
    /// don't, on purpose). When empty the user is created without a
    /// password — ssh-key only login.
    pub password_hash: Option<String>,
    /// Commands run once on first boot.
    pub runcmd: Vec<String>,
    /// Extra packages installed via the distro's package manager.
    pub packages: Vec<String>,
    /// Optional v2 netplan network-config block. Passed through verbatim.
    pub network_config: Option<String>,
}

/// Build the `meta-data` file contents.
pub fn build_meta_data(instance_id: &str, hostname: &str) -> String {
    format!("instance-id: {instance_id}\nlocal-hostname: {hostname}\n")
}

/// Build the `user-data` file as a `#cloud-config` document. The
/// indentation matches cloud-init's expected YAML — two-space.
pub fn build_user_data(cfg: &CloudInitConfig) -> String {
    let user = cfg.username.as_deref().unwrap_or("cali");
    let mut s = String::from("#cloud-config\n");
    if let Some(h) = &cfg.hostname {
        s.push_str(&format!("hostname: {h}\nfqdn: {h}\nmanage_etc_hosts: true\n"));
    }
    s.push_str("users:\n");
    s.push_str(&format!("  - name: {user}\n"));
    s.push_str("    sudo: ALL=(ALL) NOPASSWD:ALL\n");
    s.push_str("    shell: /bin/bash\n");
    if !cfg.ssh_authorized_keys.is_empty() {
        s.push_str("    ssh_authorized_keys:\n");
        for k in &cfg.ssh_authorized_keys {
            // Strip embedded newlines so a malformed key can't break out
            // into the parent YAML structure.
            let safe = k.replace('\n', " ").replace('\r', " ");
            s.push_str(&format!("      - {safe}\n"));
        }
    }
    if let Some(hash) = &cfg.password_hash {
        s.push_str(&format!("    passwd: {hash}\n"));
        s.push_str("    lock_passwd: false\n");
    }
    if !cfg.packages.is_empty() {
        s.push_str("packages:\n");
        for p in &cfg.packages {
            s.push_str(&format!("  - {p}\n"));
        }
    }
    if !cfg.runcmd.is_empty() {
        s.push_str("runcmd:\n");
        for cmd in &cfg.runcmd {
            // Use the YAML list form — string entries are run via /bin/sh -c.
            let safe = cmd.replace('\n', " ");
            s.push_str(&format!("  - {safe}\n"));
        }
    }
    s
}

/// Build the `<disk>` XML to attach a NoCloud ISO as a CD-ROM (slot
/// `sda` by default; caller may override). `iso_path` must be a path
/// the hypervisor can read.
pub fn build_seed_iso_disk_xml(iso_path: &str, target_dev: &str) -> String {
    format!(
        "<disk type='file' device='cdrom'>\n  <driver name='qemu' type='raw'/>\n  <source file='{}'/>\n  <target dev='{}' bus='sata'/>\n  <readonly/>\n</disk>\n",
        escape_xml(iso_path),
        escape_xml(target_dev),
    )
}

/// Compose the `<metadata>` injection that marks a domain as template.
/// libvirt happily round-trips namespaced child elements as long as
/// the parent `<metadata>` exists.
pub fn template_metadata_block() -> String {
    format!(
        "<metadata>\n  <{p}:{el} xmlns:{p}='{ns}'/>\n</metadata>",
        p = TEMPLATE_NS_PREFIX,
        el = TEMPLATE_ELEMENT,
        ns = TEMPLATE_NS_URI,
    )
}

/// Strip our template marker from a domain XML in place. Returns the
/// rewritten XML; idempotent — domains without the marker pass through.
/// Quotes-agnostic: libvirt re-emits XML with double quotes even when
/// we wrote single quotes, so we don't predicate the strip on quote
/// style.
pub fn remove_template_marker(xml: &str) -> String {
    let prefix_open = format!("<{TEMPLATE_NS_PREFIX}:{TEMPLATE_ELEMENT}");
    if !xml.contains(&prefix_open) {
        return xml.to_string();
    }
    let mut out = xml.to_string();
    while let Some(start) = out.find(&prefix_open) {
        // Find the closing `/>` — our element is always self-closing.
        let after = &out[start..];
        let Some(end_rel) = after.find("/>") else { break };
        let absolute_end = start + end_rel + 2;
        // Eat preceding indentation + trailing newline so we don't
        // leave an empty line behind.
        let mut span_start = start;
        if let Some(last_nl) = out[..start].rfind('\n') {
            let trail = &out[last_nl + 1..start];
            if trail.chars().all(|c| c.is_whitespace()) {
                span_start = last_nl;
            }
        }
        let mut span_end = absolute_end;
        if out[span_end..].starts_with('\n') {
            // Already covered by eating the leading newline above; only
            // bump if we did NOT swallow the leading newline.
            if span_start == start {
                span_end += 1;
            }
        }
        out.replace_range(span_start..span_end, "");
    }
    out
}

/// Insert (or replace) the kraftwerk template marker in a domain XML.
/// Inserts a `<metadata>` block when one is absent.
pub fn add_template_marker(xml: &str) -> String {
    if is_template(xml) {
        return xml.to_string();
    }
    let snippet = format!(
        "<{p}:{el} xmlns:{p}='{ns}'/>",
        p = TEMPLATE_NS_PREFIX,
        el = TEMPLATE_ELEMENT,
        ns = TEMPLATE_NS_URI,
    );

    if let Some(close) = xml.find("</metadata>") {
        // Inject the marker before the existing </metadata>.
        let mut s = String::with_capacity(xml.len() + snippet.len() + 4);
        s.push_str(&xml[..close]);
        s.push_str("  ");
        s.push_str(&snippet);
        s.push('\n');
        s.push_str(&xml[close..]);
        return s;
    }

    // No <metadata> yet — insert one right after </name> (always present
    // and near the top of <domain>).
    let block = format!("<metadata>\n  {snippet}\n</metadata>\n");
    if let Some(after_name) = xml.find("</name>") {
        let split = after_name + "</name>".len();
        let mut s = String::with_capacity(xml.len() + block.len() + 4);
        s.push_str(&xml[..split]);
        s.push_str("\n  ");
        s.push_str(&block);
        s.push_str(&xml[split..]);
        return s;
    }
    // Last resort — append before </domain>.
    if let Some(close) = xml.rfind("</domain>") {
        let mut s = String::with_capacity(xml.len() + block.len() + 4);
        s.push_str(&xml[..close]);
        s.push_str("  ");
        s.push_str(&block);
        s.push_str(&xml[close..]);
        return s;
    }
    xml.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_data_includes_id_and_hostname() {
        let m = build_meta_data("iid-abc", "host42");
        assert!(m.contains("instance-id: iid-abc"));
        assert!(m.contains("local-hostname: host42"));
    }

    #[test]
    fn user_data_minimal_has_default_user_and_sudo() {
        let cfg = CloudInitConfig::default();
        let u = build_user_data(&cfg);
        assert!(u.starts_with("#cloud-config"));
        assert!(u.contains("name: cali"));
        assert!(u.contains("NOPASSWD"));
    }

    #[test]
    fn user_data_includes_ssh_keys_packages_runcmd() {
        let cfg = CloudInitConfig {
            hostname: Some("vm1".into()),
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA me@host".into()],
            packages: vec!["htop".into(), "vim".into()],
            runcmd: vec!["systemctl enable foo".into()],
            ..Default::default()
        };
        let u = build_user_data(&cfg);
        assert!(u.contains("hostname: vm1"));
        assert!(u.contains("ssh_authorized_keys"));
        assert!(u.contains("ssh-ed25519 AAAA me@host"));
        assert!(u.contains("packages:"));
        assert!(u.contains("- htop"));
        assert!(u.contains("- vim"));
        assert!(u.contains("runcmd"));
        assert!(u.contains("systemctl enable foo"));
    }

    #[test]
    fn ssh_key_with_newline_does_not_break_yaml() {
        let cfg = CloudInitConfig {
            ssh_authorized_keys: vec!["ssh-ed25519 AAA\nrm -rf /".into()],
            ..Default::default()
        };
        let u = build_user_data(&cfg);
        // The newline must be neutered so the malicious second line
        // doesn't appear at YAML root level.
        assert!(!u.lines().any(|l| l.starts_with("rm -rf")));
    }

    #[test]
    fn password_hash_passes_through() {
        let cfg = CloudInitConfig {
            password_hash: Some("$6$rounds=...$abc".into()),
            ..Default::default()
        };
        let u = build_user_data(&cfg);
        assert!(u.contains("passwd: $6$rounds=...$abc"));
        assert!(u.contains("lock_passwd: false"));
    }

    #[test]
    fn template_marker_round_trips() {
        let xml = "<domain>\n  <name>tpl</name>\n  <memory unit='KiB'>1024</memory>\n</domain>";
        assert!(!is_template(xml));
        let marked = add_template_marker(xml);
        assert!(is_template(&marked));
        let unmarked = remove_template_marker(&marked);
        assert!(!is_template(&unmarked));
    }

    #[test]
    fn add_marker_idempotent() {
        let xml = "<domain><name>x</name><metadata><kraftwerk:template xmlns:kraftwerk='https://github.com/calibrae/kraftwerk/template'/></metadata></domain>";
        assert!(is_template(xml));
        let again = add_template_marker(xml);
        // No duplicate marker.
        let count = again.matches("<kraftwerk:template").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn add_marker_inserts_metadata_when_absent() {
        let xml = "<domain>\n  <name>foo</name>\n</domain>";
        let marked = add_template_marker(xml);
        assert!(marked.contains("<metadata>"));
        assert!(is_template(&marked));
    }

    #[test]
    fn seed_iso_disk_xml_is_readonly_cdrom() {
        let d = build_seed_iso_disk_xml("/var/lib/libvirt/images/seed.iso", "sda");
        assert!(d.contains("device='cdrom'"));
        assert!(d.contains("<readonly/>"));
        assert!(d.contains("/var/lib/libvirt/images/seed.iso"));
    }
}
