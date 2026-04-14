//! Integration test: round-trip real fedora-workstation XML through parse/apply.
//! Run with: cargo run --example round_i_integration -- /tmp/fedora-ws-original.xml

use std::env;
use std::fs;

use kraftwerk_lib::libvirt::cpu_tune_config::{
    apply, parse, validate, CpuConfig, CpuTunePatch, IoThreadsConfig, VcpuConfig,
};

fn main() {
    let path = env::args().nth(1).expect("usage: <path-to-xml>");
    let xml = fs::read_to_string(&path).expect("read xml");

    println!("━━━ TEST 1: parse ━━━");
    let snap = parse(&xml).expect("parse");
    println!("cpu.mode        = {}", snap.cpu.mode);
    println!("cpu.migratable  = {:?}", snap.cpu.migratable);
    println!("vcpus.max       = {}", snap.vcpus.max);
    println!("vcpus.current   = {}", snap.vcpus.current);
    println!("vcpus.placement = {:?}", snap.vcpus.placement);
    println!("iothreads.count = {}", snap.iothreads.count);
    println!("cputune.vcpupin = {} entries", snap.cputune.vcpupin.len());
    println!("memtune hard    = {:?}", snap.memtune.hard_limit_kib);
    println!("numa cells      = {}", snap.numa.cells.len());
    println!("hugepages       = {} pages", snap.hugepages.pages.len());
    assert!(snap.vcpus.max > 0, "expected non-zero vcpus");
    validate(&snap).expect("validate baseline");

    println!("\n━━━ TEST 2: toggle CPU mode (host-passthrough → host-model → back) ━━━");
    let p1 = CpuTunePatch {
        cpu: Some(CpuConfig {
            mode: "host-model".into(),
            check: Some("partial".into()),
            migratable: None,
            ..snap.cpu.clone()
        }),
        ..Default::default()
    };
    // Clear fields incompatible with host-model.
    let mut cpu_hostmodel = snap.cpu.clone();
    cpu_hostmodel.mode = "host-model".into();
    cpu_hostmodel.check = Some("partial".into());
    cpu_hostmodel.migratable = None;
    let p1 = CpuTunePatch { cpu: Some(cpu_hostmodel), ..Default::default() };
    let xml1 = apply(&xml, &p1).expect("apply host-model");
    let s1 = parse(&xml1).expect("reparse");
    validate(&s1).expect("validate host-model");
    assert_eq!(s1.cpu.mode, "host-model");
    println!("OK — now host-model");

    let mut cpu_back = s1.cpu.clone();
    cpu_back.mode = "host-passthrough".into();
    cpu_back.check = Some("none".into());
    cpu_back.migratable = Some(true);
    let p2 = CpuTunePatch { cpu: Some(cpu_back), ..Default::default() };
    let xml2 = apply(&xml1, &p2).expect("apply back");
    let s2 = parse(&xml2).expect("reparse");
    validate(&s2).expect("validate back");
    assert_eq!(s2.cpu.mode, "host-passthrough");
    println!("OK — back to host-passthrough");

    println!("\n━━━ TEST 3: set + clear iothreads count ━━━");
    let p3 = CpuTunePatch {
        iothreads: Some(IoThreadsConfig { count: 2 }),
        ..Default::default()
    };
    let xml3 = apply(&xml, &p3).expect("apply iothreads");
    let s3 = parse(&xml3).expect("reparse");
    assert_eq!(s3.iothreads.count, 2);
    println!("OK — iothreads=2");

    let p4 = CpuTunePatch {
        iothreads: Some(IoThreadsConfig { count: 0 }),
        ..Default::default()
    };
    let xml4 = apply(&xml3, &p4).expect("apply iothreads=0");
    let s4 = parse(&xml4).expect("reparse");
    assert_eq!(s4.iothreads.count, 0);
    assert!(!xml4.contains("<iothreads>"), "iothreads element should be removed");
    println!("OK — iothreads cleared");

    println!("\n━━━ TEST 4: vCPU current change (→ 1 → 2 → original) ━━━");
    let p5 = CpuTunePatch {
        vcpus: Some(VcpuConfig { max: snap.vcpus.max, current: 1,
            placement: snap.vcpus.placement.clone(), cpuset: snap.vcpus.cpuset.clone() }),
        ..Default::default()
    };
    let xml5 = apply(&xml, &p5).expect("apply vcpus=1");
    let s5 = parse(&xml5).expect("reparse");
    assert_eq!(s5.vcpus.current, 1);
    validate(&s5).expect("validate vcpus=1");
    println!("OK — vcpus.current=1");

    let p6 = CpuTunePatch {
        vcpus: Some(VcpuConfig { max: snap.vcpus.max, current: snap.vcpus.current,
            placement: snap.vcpus.placement.clone(), cpuset: snap.vcpus.cpuset.clone() }),
        ..Default::default()
    };
    let xml6 = apply(&xml5, &p6).expect("apply restore");
    let s6 = parse(&xml6).expect("reparse");
    assert_eq!(s6.vcpus.current, snap.vcpus.current);
    println!("OK — vcpus restored to {}", snap.vcpus.current);

    println!("\n✓ All integration checks passed");
}
