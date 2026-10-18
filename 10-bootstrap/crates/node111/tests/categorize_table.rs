//! T4 acceptance: table-driven category tests (spec rules 0–7 + gateway overlay),
//! ≥20 cases incl. unknown-degradation.

use node111::install::categorize::{categorize, Category, Refusal};
use node111::install::probe::{Facts, Gpu, LoraDevice};

const GB: u64 = 1 << 30;

fn base() -> Facts {
    Facts {
        arch: Some("x86_64".into()),
        cores: Some(4),
        ram_bytes: Some(8 * GB),
        free_storage_bytes: Some(64 * GB),
        distro: Some("debian".into()),
        init: Some("systemd".into()),
        systemd: Some(true),
        in_container: Some(false),
        is_wsl: Some(false),
        battery: Some(false),
        wifi_ap_capable: Some(false),
        ethernet: Some(false),
        ble: Some(false),
        lora: Some(vec![]),
        gpu: Some(vec![]),
        pi_model: None,
        internet: Some(true),
        is_phone: false,
    }
}

fn lora() -> LoraDevice {
    LoraDevice {
        vid: "10c4".into(),
        pid: "ea60".into(),
        product: "CP2102 USB to UART Bridge Controller".into(),
    }
}

struct Case {
    name: &'static str,
    mutate: fn(&mut Facts),
    want: Category,
    rule: u8,
    gateway: bool,
    degraded: bool,
}

#[test]
fn table() {
    let cases: Vec<Case> = vec![
        Case {
            name: "plain desktop → 2A (rule 7 catch)",
            mutate: |_| {},
            want: Category::Node2A,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "nvidia gpu → 4B",
            mutate: |f| {
                f.gpu = Some(vec![Gpu {
                    vendor: "10de".into(),
                    class: "0300".into(),
                }])
            },
            want: Category::Hub4B,
            rule: 1,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "amd gpu → 4B",
            mutate: |f| {
                f.gpu = Some(vec![Gpu {
                    vendor: "1002".into(),
                    class: "0302".into(),
                }])
            },
            want: Category::Hub4B,
            rule: 1,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "intel igpu alone does NOT trigger 4B",
            mutate: |f| {
                f.gpu = Some(vec![Gpu {
                    vendor: "8086".into(),
                    class: "0300".into(),
                }])
            },
            want: Category::Node2A,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "8c/32G no gpu → 4B",
            mutate: |f| {
                f.cores = Some(8);
                f.ram_bytes = Some(32 * GB);
            },
            want: Category::Hub4B,
            rule: 1,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "8c/31.9G stays below 4B, lands on 4A",
            mutate: |f| {
                f.cores = Some(8);
                f.ram_bytes = Some(31 * GB);
                f.free_storage_bytes = Some(600 * GB);
            },
            want: Category::Hub4A,
            rule: 2,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "512G free + 8G → 4A",
            mutate: |f| f.free_storage_bytes = Some(600 * GB),
            want: Category::Hub4A,
            rule: 2,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "600G free but 4G ram → not 4A",
            mutate: |f| {
                f.free_storage_bytes = Some(600 * GB);
                f.ram_bytes = Some(4 * GB);
            },
            want: Category::Node2A,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "lora + ap → bridge",
            mutate: |f| {
                f.lora = Some(vec![lora()]);
                f.wifi_ap_capable = Some(true);
            },
            want: Category::Bridge3,
            rule: 3,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "lora + ethernet → bridge",
            mutate: |f| {
                f.lora = Some(vec![lora()]);
                f.ethernet = Some(true);
            },
            want: Category::Bridge3,
            rule: 3,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "lora alone → NOT bridge",
            mutate: |f| f.lora = Some(vec![lora()]),
            want: Category::Node2A,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "ap + ethernet (no lora) → bridge",
            mutate: |f| {
                f.wifi_ap_capable = Some(true);
                f.ethernet = Some(true);
            },
            want: Category::Bridge3,
            rule: 3,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "pi5 8G → 2B",
            mutate: |f| f.pi_model = Some("Raspberry Pi 5 Model B Rev 1.0".into()),
            want: Category::Node2B,
            rule: 4,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "pi5 4G → falls past rule 4",
            mutate: |f| {
                f.pi_model = Some("Raspberry Pi 5 Model B Rev 1.0".into());
                f.ram_bytes = Some(4 * GB);
            },
            want: Category::Node2A,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "laptop 16G → 2B",
            mutate: |f| {
                f.battery = Some(true);
                f.ram_bytes = Some(16 * GB);
            },
            want: Category::Node2B,
            rule: 5,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "laptop any-gpu → 2B (intel counts as any-gpu)",
            mutate: |f| {
                f.battery = Some(true);
                f.gpu = Some(vec![Gpu {
                    vendor: "8086".into(),
                    class: "0300".into(),
                }]);
            },
            want: Category::Node2B,
            rule: 5,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "laptop plain → 2A",
            mutate: |f| f.battery = Some(true),
            want: Category::Node2A,
            rule: 6,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "phone 8G → 2B (meminfo rule)",
            mutate: |f| {
                f.battery = Some(true);
                f.is_phone = true;
            },
            want: Category::Node2B,
            rule: 6,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "phone 4G → 2A",
            mutate: |f| {
                f.battery = Some(true);
                f.is_phone = true;
                f.ram_bytes = Some(4 * GB);
            },
            want: Category::Node2A,
            rule: 6,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "pi4 → relay",
            mutate: |f| f.pi_model = Some("Raspberry Pi 4 Model B".into()),
            want: Category::Relay1,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "pi3 → relay",
            mutate: |f| f.pi_model = Some("Raspberry Pi 3 Model B+".into()),
            want: Category::Relay1,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "sub-1G board → relay",
            mutate: |f| f.ram_bytes = Some(512 * 1024 * 1024),
            want: Category::Relay1,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "no internet → gateway overlay off",
            mutate: |f| f.internet = Some(false),
            want: Category::Node2A,
            rule: 7,
            gateway: false,
            degraded: false,
        },
        Case {
            name: "unknown ram/gpu/storage degrades, not satisfies",
            mutate: |f| {
                f.ram_bytes = None;
                f.gpu = None;
                f.free_storage_bytes = None;
                f.cores = None;
            },
            want: Category::Relay1,
            rule: 7,
            gateway: true,
            degraded: false,
        },
        Case {
            name: "systemd-ful container → degraded relay",
            mutate: |f| f.in_container = Some(true),
            want: Category::Relay1,
            rule: 0,
            gateway: true,
            degraded: true,
        },
        Case {
            name: "wsl with systemd → degraded relay",
            mutate: |f| f.is_wsl = Some(true),
            want: Category::Relay1,
            rule: 0,
            gateway: true,
            degraded: true,
        },
    ];

    let mut n = 0;
    for case in &cases {
        let mut f = base();
        (case.mutate)(&mut f);
        let got =
            categorize(&f).unwrap_or_else(|r| panic!("{}: unexpected refusal {r:?}", case.name));
        assert_eq!(
            (got.category, got.rule, got.gateway, got.degraded),
            (case.want, case.rule, case.gateway, case.degraded),
            "case {:?}",
            case.name
        );
        n += 1;
    }
    assert!(n >= 20, "spec asks ≥20 table cases, got {n}");
}

#[test]
fn refusals() {
    let mut f = base();
    f.in_container = Some(true);
    f.systemd = Some(false);
    assert_eq!(categorize(&f), Err(Refusal::SystemdLess));

    let mut f = base();
    f.is_wsl = Some(true);
    f.systemd = None; // unknown counts as not-systemd for rule 0
    assert_eq!(categorize(&f), Err(Refusal::SystemdLess));

    let mut f = base();
    f.arch = Some("armv7l".into());
    assert_eq!(categorize(&f), Err(Refusal::Armv7lKernel));
}
