//! T4 acceptance: probe collection over an in-memory fixture env — every fact
//! degrades to None when sources are absent; LoRa matching honors VID:PID AND
//! product-substring.

use node111::install::probe::{collect, Facts};
use node111::testkit::MapEnv;

const MEMINFO_8G: &str = "MemTotal:        8167848 kB\nMemFree: 1000000 kB\n";
const CPUINFO_4: &str = "processor\t: 0\n\nprocessor\t: 1\n\nprocessor\t: 2\n\nprocessor\t: 3\n";

#[test]
fn empty_env_everything_unknown_except_systemd() {
    let facts = collect(&MapEnv::new());
    let expect = Facts {
        systemd: Some(false),
        ..Facts::default()
    };
    assert_eq!(facts.arch, expect.arch);
    assert_eq!(facts.cores, None);
    assert_eq!(facts.ram_bytes, None);
    assert_eq!(facts.distro, None);
    assert_eq!(facts.battery, None);
    assert_eq!(facts.in_container, None);
    assert_eq!(facts.lora, None);
    assert_eq!(facts.gpu, None);
    assert_eq!(facts.systemd, Some(false));
}

#[test]
fn full_debian_desktop() {
    let env = MapEnv::new()
        .cmd("uname -m", "x86_64\n")
        .file("/proc/cpuinfo", CPUINFO_4)
        .file("/proc/meminfo", MEMINFO_8G)
        .file("/etc/os-release", "ID=debian\nID_LIKE=\n")
        .file("/proc/1/comm", "systemd\n")
        .dir("/run/systemd/system", &["control"])
        .cmd("systemd-detect-virt --container", "none\n")
        .file("/proc/version", "Linux version 6.1.0-17-amd64 (debian-kernel@lists.debian.org)\n")
        .dir("/sys/class/power_supply", &["AC"])
        .cmd("iw phy", "        Supported interface modes:\n                 * IBSS\n                 * managed\n                 * AP\n")
        .dir("/sys/class/net", &["lo", "eth0", "wlan0"])
        .file("/sys/class/net/eth0/device/uevent", "DRIVER=e1000e\n")
        .dir("/sys/class/bluetooth", &["hci0"])
        .dir("/sys/bus/usb/devices", &["1-1", "usb1"])
        .file("/sys/bus/usb/devices/1-1/idVendor", "10c4\n")
        .file("/sys/bus/usb/devices/1-1/idProduct", "ea60\n")
        .file("/sys/bus/usb/devices/1-1/product", "CP2102 USB to UART Bridge Controller\n")
        .dir("/sys/bus/pci/devices", &["0000:00:02.0"])
        .file("/sys/bus/pci/devices/0000:00:02.0/class", "0x030000\n")
        .file("/sys/bus/pci/devices/0000:00:02.0/vendor", "0x10de\n");

    let f = collect(&env);
    assert_eq!(f.arch.as_deref(), Some("x86_64"));
    assert_eq!(f.cores, Some(4));
    assert_eq!(f.ram_bytes, Some(8167848 * 1024));
    assert_eq!(f.distro.as_deref(), Some("debian"));
    assert_eq!(f.init.as_deref(), Some("systemd"));
    assert_eq!(f.systemd, Some(true));
    assert_eq!(f.in_container, Some(false));
    assert_eq!(f.is_wsl, Some(false));
    assert_eq!(f.battery, Some(false));
    assert_eq!(f.wifi_ap_capable, Some(true));
    assert_eq!(f.ethernet, Some(true));
    assert_eq!(f.ble, Some(true));
    let lora = f.lora.unwrap();
    assert_eq!(lora.len(), 1);
    assert_eq!(lora[0].vid, "10c4");
    let gpu = f.gpu.unwrap();
    assert_eq!(gpu.len(), 1);
    assert_eq!(gpu[0].vendor, "10de");
    assert!(gpu[0].is_discrete());
}

#[test]
fn lora_requires_product_substring_when_present_in_table() {
    // Same VID:PID but wrong product string → not matched (CP2102 is generic).
    let env = MapEnv::new()
        .dir("/sys/bus/usb/devices", &["1-1"])
        .file("/sys/bus/usb/devices/1-1/idVendor", "10c4\n")
        .file("/sys/bus/usb/devices/1-1/idProduct", "ea60\n")
        .file("/sys/bus/usb/devices/1-1/product", "Some Other Bridge\n");
    let f = collect(&env);
    assert_eq!(f.lora.unwrap().len(), 0);

    // No product file at all: row requires the substring → not matched.
    let env = MapEnv::new()
        .dir("/sys/bus/usb/devices", &["1-1"])
        .file("/sys/bus/usb/devices/1-1/idVendor", "10c4\n")
        .file("/sys/bus/usb/devices/1-1/idProduct", "ea60\n");
    let f = collect(&env);
    assert_eq!(f.lora.unwrap().len(), 0);
}

#[test]
fn lora_wildcard_rows_match_vid_pid() {
    // ESP32-S3 native USB (303a:*, no product requirement).
    let env = MapEnv::new()
        .dir("/sys/bus/usb/devices", &["1-2"])
        .file("/sys/bus/usb/devices/1-2/idVendor", "303a\n")
        .file("/sys/bus/usb/devices/1-2/idProduct", "1001\n")
        .file(
            "/sys/bus/usb/devices/1-2/product",
            "USB JTAG/serial debug unit\n",
        );
    let f = collect(&env);
    assert_eq!(f.lora.unwrap().len(), 1);
}

#[test]
fn wsl_and_container_detection() {
    let env = MapEnv::new()
        .cmd("systemd-detect-virt --container", "wsl\n")
        .file(
            "/proc/version",
            "Linux version 6.6.87.2-microsoft-standard-WSL2\n",
        );
    let f = collect(&env);
    assert_eq!(f.in_container, Some(true));
    assert_eq!(f.is_wsl, Some(true));

    // Fallback without detect-virt: .dockerenv + cgroup markers.
    let env = MapEnv::new()
        .file("/.dockerenv", "")
        .file("/proc/1/cgroup", "0::/docker/abc123\n");
    let f = collect(&env);
    assert_eq!(f.in_container, Some(true));
}

#[test]
fn pi_model_and_battery_and_wifi_absence() {
    let env = MapEnv::new()
        .file(
            "/proc/device-tree/model",
            "Raspberry Pi 5 Model B Rev 1.0\0",
        )
        .dir("/sys/class/power_supply", &["BAT0"])
        .dir("/sys/class/net", &["lo", "wlan0"]);
    let f = collect(&env);
    assert_eq!(
        f.pi_model.as_deref(),
        Some("Raspberry Pi 5 Model B Rev 1.0")
    );
    assert_eq!(f.battery, Some(true));
    assert_eq!(f.wifi_ap_capable, None); // no iw → unknown
    assert_eq!(f.ethernet, Some(false)); // only lo+wlan
}

#[test]
fn distro_families() {
    for (osr, want) in [
        ("ID=ubuntu\nID_LIKE=debian\n", "debian"),
        ("ID=fedora\n", "fedora"),
        ("ID=centos\nID_LIKE=\"rhel fedora\"\n", "fedora"),
        ("ID=arch\n", "arch"),
        ("ID=endeavouros\nID_LIKE=arch\n", "arch"),
        ("ID=alpine\n", "alpine"),
    ] {
        let env = MapEnv::new().file("/etc/os-release", osr);
        assert_eq!(
            collect(&env).distro.as_deref(),
            Some(want),
            "os-release {osr:?}"
        );
    }
}
