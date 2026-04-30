//! Integration tests for phase 3 features (storage depth):
//! 3.1 backing chain parsing on a domain XML pulled from the hypervisor
//! 3.2 secret CRUD + LUKS volume creation
//! 3.5 streaming upload of a local file into a volume
//!
//! Configure the target via `KRAFTWERK_TEST_URI`. The default storage
//! pool is used for volume creation; the test cleans up after itself
//! via RAII guards. Skips when the env var is unset.

use kraftwerk_lib::libvirt::connection::LibvirtConnection;
use kraftwerk_lib::libvirt::secrets::SecretUsage;
use std::env;
use std::io::Write;

fn test_uri() -> Option<String> {
    env::var("KRAFTWERK_TEST_URI").ok().filter(|s| !s.is_empty())
}

fn connect() -> Option<LibvirtConnection> {
    let uri = test_uri()?;
    let conn = LibvirtConnection::new();
    conn.open(&uri).expect("connection.open");
    Some(conn)
}

/// Drop a leftover volume by path. Errors are silenced — best-effort.
struct VolumeCleanup<'a> {
    conn: &'a LibvirtConnection,
    path: String,
}
impl<'a> Drop for VolumeCleanup<'a> {
    fn drop(&mut self) {
        let _ = self.conn.delete_volume(&self.path);
    }
}

struct SecretCleanup<'a> {
    conn: &'a LibvirtConnection,
    uuid: String,
}
impl<'a> Drop for SecretCleanup<'a> {
    fn drop(&mut self) {
        let _ = self.conn.delete_secret(&self.uuid);
    }
}

const TEST_POOL: &str = "default";

// 3.2 — secrets + LUKS

#[test]
fn test_secret_crud_round_trip() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let pool_path = pool_target_path(&conn, TEST_POOL).expect("pool target_path");
    let usage_id = format!("{pool_path}/kraftwerk-secret-test.qcow2");

    let uuid = conn
        .define_secret(SecretUsage::Volume, Some(&usage_id), Some("kraftwerk it"), false, true)
        .expect("define_secret");
    let _g = SecretCleanup { conn: &conn, uuid: uuid.clone() };
    conn.set_secret_value(&uuid, b"hunter2").expect("set_secret_value");

    let secrets = conn.list_secrets().expect("list_secrets");
    let mine = secrets
        .iter()
        .find(|s| s.uuid == uuid)
        .expect("our secret should be listed");
    assert_eq!(mine.usage, SecretUsage::Volume);
    assert_eq!(mine.usage_id.as_deref(), Some(usage_id.as_str()));
    assert!(mine.private, "private flag preserved");
}

#[test]
fn test_luks_volume_create_and_delete() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let pool_path = pool_target_path(&conn, TEST_POOL).expect("pool target_path");
    let vol_name = "kraftwerk-luks-test.qcow2";
    let vol_path = format!("{pool_path}/{vol_name}");
    // Best-effort wipe of leftovers from a previous failed run.
    let _ = conn.delete_volume(&vol_path);

    let secret_uuid = conn
        .define_secret(SecretUsage::Volume, Some(&vol_path), Some("LUKS test"), false, true)
        .expect("define_secret");
    let _sg = SecretCleanup { conn: &conn, uuid: secret_uuid.clone() };
    conn.set_secret_value(&secret_uuid, b"correct horse battery staple")
        .expect("set_secret_value");

    let xml = kraftwerk_lib::libvirt::secrets::build_luks_volume_xml(
        vol_name,
        128 * 1024 * 1024, // 128 MiB — enough for a LUKS header + a sliver
        &secret_uuid,
    );
    let path = conn
        .create_volume(TEST_POOL, &xml)
        .expect("create_volume (LUKS)");
    let _vg = VolumeCleanup { conn: &conn, path: path.clone() };

    let vols = conn.list_volumes(TEST_POOL).expect("list_volumes");
    let ours = vols
        .iter()
        .find(|v| v.name == vol_name)
        .expect("LUKS volume should appear in list");
    assert_eq!(ours.path, path);
}

// 3.5 — streaming upload

#[test]
fn test_volume_upload_round_trip() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let pool_path = pool_target_path(&conn, TEST_POOL).expect("pool target_path");
    let vol_name = "kraftwerk-upload-test.raw";
    let vol_path = format!("{pool_path}/{vol_name}");
    let _ = conn.delete_volume(&vol_path);

    // Make a deterministic 4 MiB local file with known bytes.
    let payload_len: u64 = 4 * 1024 * 1024;
    let mut tmp = std::env::temp_dir();
    tmp.push("kraftwerk-upload-fixture.bin");
    {
        let mut f = std::fs::File::create(&tmp).expect("create fixture");
        let chunk = vec![0xABu8; 64 * 1024];
        for _ in 0..(payload_len as usize / chunk.len()) {
            f.write_all(&chunk).expect("write fixture");
        }
    }

    // Allocate the destination volume.
    let xml = kraftwerk_lib::libvirt::storage_config::build_volume_xml(
        &kraftwerk_lib::libvirt::storage_config::VolumeBuildParams {
            name: vol_name,
            capacity_bytes: payload_len,
            format: "raw",
            allocation_bytes: None,
        },
    );
    let path = conn.create_volume(TEST_POOL, &xml).expect("create_volume");
    let _vg = VolumeCleanup { conn: &conn, path: path.clone() };

    // Stream upload. Track the last-seen progress to prove the callback fires.
    let last_seen = std::sync::Mutex::new((0u64, 0u64));
    let sent = conn
        .upload_volume_from_path(
            TEST_POOL,
            vol_name,
            tmp.to_str().unwrap(),
            512 * 1024,
            |sent, total| {
                let mut g = last_seen.lock().unwrap();
                *g = (sent, total);
            },
        )
        .expect("upload_volume_from_path");
    assert_eq!(sent, payload_len);
    let final_seen = *last_seen.lock().unwrap();
    assert_eq!(final_seen.0, payload_len);
    assert_eq!(final_seen.1, payload_len);

    let _ = std::fs::remove_file(&tmp);
}

// 3.1 — backing chain parsing of a real domain XML

#[test]
fn test_get_backing_chains_returns_disks_for_some_domain() {
    let Some(conn) = connect() else {
        eprintln!("SKIP: KRAFTWERK_TEST_URI unset");
        return;
    };
    let domains = conn.list_all_domains().expect("list_all_domains");
    let Some(d) = domains.into_iter().next() else {
        eprintln!("SKIP: no domains on hypervisor");
        return;
    };
    let chains = conn
        .get_backing_chains(&d.name)
        .expect("get_backing_chains");
    // Every domain has at least one disk; readonly cdroms are fine, just
    // assert the helper parses without panicking and returns something
    // with a target field set.
    assert!(
        chains.iter().any(|c| !c.target.is_empty()),
        "at least one disk with a target should be reported"
    );
}

// ─── helpers ───

fn pool_target_path(conn: &LibvirtConnection, pool: &str) -> Option<String> {
    let cfg = conn.get_pool_config(pool).ok()?;
    cfg.target_path
}
