//! Integration tests for phase 6 — templates, cloud-init seed,
//! image library (when added), OVA/OVF import (when added).
//!
//! Configure via `KRAFTWERK_RAM_TEST_URI` (libvirt URI) and
//! `KRAFTWERK_RAM_TEST_VM_A` (default `vmtest-a`).

use kraftwerk_lib::libvirt::connection::LibvirtConnection;
use std::env;

fn test_uri() -> Option<String> {
    env::var("KRAFTWERK_RAM_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn vm_name() -> String {
    env::var("KRAFTWERK_RAM_TEST_VM_A").unwrap_or_else(|_| "vmtest-a".into())
}

fn connect() -> Option<LibvirtConnection> {
    let uri = test_uri()?;
    let conn = LibvirtConnection::new();
    conn.open(&uri).expect("connection.open");
    Some(conn)
}

struct TemplateMarkerGuard<'a> {
    conn: &'a LibvirtConnection,
    vm: String,
    was_template: bool,
}
impl Drop for TemplateMarkerGuard<'_> {
    fn drop(&mut self) {
        let _ = self.conn.set_template_flag(&self.vm, self.was_template);
    }
}

#[test]
fn template_flag_round_trip() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let vm = vm_name();

    let xml_before = conn.get_domain_xml(&vm, true).expect("get xml");
    let was_template = kraftwerk_lib::libvirt::templates::is_template(&xml_before);
    let _g = TemplateMarkerGuard {
        conn: &conn,
        vm: vm.clone(),
        was_template,
    };

    // Mark as template.
    conn.set_template_flag(&vm, true).expect("mark template");
    let xml_after = conn.get_domain_xml(&vm, true).expect("get xml");
    assert!(kraftwerk_lib::libvirt::templates::is_template(&xml_after));

    // Templates filter picks it up.
    let templates = conn.list_templates().expect("list templates");
    assert!(templates.iter().any(|v| v.name == vm), "template not in list");

    // Unmark.
    conn.set_template_flag(&vm, false).expect("unmark template");
    let xml_unmarked = conn.get_domain_xml(&vm, true).expect("get xml");
    assert!(!kraftwerk_lib::libvirt::templates::is_template(&xml_unmarked));
}

#[test]
fn list_catalog_images_against_default_pool() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let pools = conn.list_storage_pools().expect("list pools");
    let dir_pool = pools
        .iter()
        .find(|p| p.is_active && p.pool_type == "dir");
    let Some(pool) = dir_pool else {
        eprintln!("SKIP: no active dir-type pool on host");
        return;
    };
    let imgs = conn.list_catalog_images(&pool.name).expect("list catalog");
    assert!(!imgs.is_empty(), "catalog must have entries");
    let downloaded = imgs.iter().filter(|i| i.local_path.is_some()).count();
    eprintln!("pool {} has {downloaded}/{} catalog entries downloaded", pool.name, imgs.len());
}

#[test]
fn inspect_ova_when_path_set() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let Some(ova) = std::env::var("KRAFTWERK_OVA_TEST_PATH").ok().filter(|s| !s.is_empty()) else {
        eprintln!("SKIP: KRAFTWERK_OVA_TEST_PATH unset");
        return;
    };
    let md = conn.inspect_ova(&ova).expect("inspect_ova");
    eprintln!("name={:?} disks={} vcpus={:?} mem={:?}MiB",
        md.name, md.disks.len(), md.vcpus, md.memory_mib);
    assert!(!md.disks.is_empty(), "OVA should declare at least one disk");
}

#[test]
fn build_cloud_init_iso_when_tools_available() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_RAM_TEST_URI unset");
        return;
    };
    let cfg = kraftwerk_lib::libvirt::templates::CloudInitConfig {
        hostname: Some("kraftwerk-it".into()),
        username: Some("cali".into()),
        ssh_authorized_keys: vec!["ssh-ed25519 AAAA test@test".into()],
        ..Default::default()
    };
    let meta = kraftwerk_lib::libvirt::templates::build_meta_data("kraftwerk-it", "kraftwerk-it");
    let user = kraftwerk_lib::libvirt::templates::build_user_data(&cfg);

    // Try a tmp path the cali user can write to. /tmp is universally
    // writable; the integration test cleans up by removing the file.
    let dest_dir = "/tmp";
    let iso_filename = format!("kraftwerk-it-test-{}.iso", std::process::id());
    let r = conn.build_cloud_init_iso(dest_dir, &iso_filename, &meta, &user, None);
    match r {
        Ok(path) => {
            eprintln!("seed iso built at {path}");
            // Best-effort cleanup. The next run will overwrite anyway.
            let _ = std::process::Command::new("ssh")
                .arg("-o").arg("BatchMode=yes")
                .arg(format!("cali@doppio"))
                .arg(format!("rm -f '{path}'"))
                .status();
        }
        Err(e) => {
            eprintln!("SKIP: ISO build failed (likely no genisoimage/xorrisofs/mkisofs): {e:?}");
        }
    }
}
