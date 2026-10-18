// Ported from supabase/edge-runtime ext/os/lib.rs and ext/os/os.js (MIT), pin v1.77.4.

//! Memory figures an edge-runtime worker reports.
//!
//! `Deno.systemMemoryInfo()` is the all-zero object from `ext/os/os.js`.
//! `op_system_memory_info` reads Linux `sysinfo` and is only exposed as
//! `EdgeRuntime.systemMemoryInfo` on the main worker. Spec:
//! `specs/functions/os.md`.

/// JavaScript that installs the user-worker `Deno.systemMemoryInfo`. It runs
/// before the function is imported. This is `ext/os/os.js`, not the op.
pub const SOURCE: &str = r#"
// Deno.systemMemoryInfo of an edge-runtime user worker (ext/os/os.js).
Object.defineProperty(Deno, "systemMemoryInfo", {
  value: () => ({
    total: 0,
    free: 0,
    available: 0,
    buffers: 0,
    cached: 0,
    swapTotal: 0,
    swapFree: 0,
  }),
  enumerable: true,
  writable: true,
  configurable: true,
});
"#;

/// `MemInfo` from `op_system_memory_info`. Counts are bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemInfo {
    /// `totalram * mem_unit` (`MemTotal`).
    pub total: u64,
    /// `freeram * mem_unit` (`MemFree`).
    pub free: u64,
    /// Same as [`Self::free`]. The op does not read `MemAvailable`.
    pub available: u64,
    /// `bufferram * mem_unit` (`Buffers`).
    pub buffers: u64,
    /// Always 0. The op leaves `cached` at its default.
    pub cached: u64,
    /// `totalswap * mem_unit` (`SwapTotal`).
    pub swap_total: u64,
    /// `freeswap * mem_unit` (`SwapFree`).
    pub swap_free: u64,
}

/// Run `op_system_memory_info`.
///
/// Linux reads `/proc/meminfo`, which publishes the same counters as
/// `sysinfo`. Any other host, or a failed read, is `MemInfo::default()`,
/// matching the op's `Some(default)` paths. The value is never absent.
#[must_use]
pub fn system_memory_info() -> MemInfo {
    system_memory_info_from(read_proc_meminfo())
}

fn read_proc_meminfo() -> std::io::Result<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo")
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "sysinfo is only read on linux",
        ))
    }
}

fn system_memory_info_from(read: std::io::Result<String>) -> MemInfo {
    match read {
        Ok(text) => parse_meminfo(&text),
        Err(_) => MemInfo::default(),
    }
}

/// Map a `/proc/meminfo` snapshot onto [`MemInfo`].
///
/// Kibibyte counters become bytes (`value * 1024`, saturating). Unknown
/// lines and non-numeric values are skipped. `available` is then set to
/// `free`, and `cached` to 0.
fn parse_meminfo(text: &str) -> MemInfo {
    let mut info = MemInfo::default();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else {
            continue;
        };
        let Some(raw) = parts.next() else {
            continue;
        };
        let Ok(kibibytes) = raw.parse::<u64>() else {
            continue;
        };
        let bytes = kibibytes.saturating_mul(1024);
        match key {
            "MemTotal:" => info.total = bytes,
            "MemFree:" => info.free = bytes,
            "Buffers:" => info.buffers = bytes,
            "SwapTotal:" => info.swap_total = bytes,
            "SwapFree:" => info.swap_free = bytes,
            _ => {}
        }
    }
    info.available = info.free;
    info.cached = 0;
    info
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const SAMPLE: &str = "\
MemTotal:       2048 kB
MemFree:        1024 kB
MemAvailable:   1500 kB
Buffers:         256 kB
Cached:          512 kB
SwapTotal:      4096 kB
SwapFree:       3072 kB
";

    fn kib(value: u64) -> u64 {
        value * 1024
    }

    fn run_node(script: &str) -> String {
        static CALLS: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "megabase-os-{}-{nanos}-{}",
            std::process::id(),
            CALLS.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run.mjs");
        std::fs::write(&path, script).unwrap();
        let output = Command::new("node")
            .arg(&path)
            .output()
            .unwrap_or_else(|error| {
                panic!(
                    "node failed to start ({error}); the systemMemoryInfo test needs node on PATH"
                )
            });
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            output.status.success(),
            "node exited {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        );
        stdout
    }

    #[test]
    fn parse_uses_sysinfo_fields_and_drops_mem_available_and_cached() {
        let info = parse_meminfo(SAMPLE);
        assert_eq!(
            info,
            MemInfo {
                total: kib(2048),
                free: kib(1024),
                available: kib(1024),
                buffers: kib(256),
                cached: 0,
                swap_total: kib(4096),
                swap_free: kib(3072),
            }
        );
        assert_ne!(info.available, kib(1500));
        assert_eq!(
            parse_meminfo("MemTotal: 2 kB\n"),
            MemInfo {
                total: kib(2),
                available: 0,
                ..MemInfo::default()
            }
        );
        assert_eq!(
            parse_meminfo("nope\nMemTotal: wat kB\n\n"),
            MemInfo::default()
        );
        let huge = format!("MemTotal: {} kB\nMemFree: 1 kB\n", u64::MAX);
        let saturated = parse_meminfo(&huge);
        assert_eq!(saturated.total, u64::MAX);
        assert_eq!(saturated.free, kib(1));
        assert_eq!(saturated.available, saturated.free);
    }

    #[test]
    fn a_failed_read_is_the_default_struct() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "no meminfo");
        assert_eq!(system_memory_info_from(Err(error)), MemInfo::default());
        assert_eq!(
            system_memory_info_from(Ok(SAMPLE.to_owned())),
            parse_meminfo(SAMPLE)
        );
    }

    #[test]
    fn the_live_read_follows_the_op_invariants() {
        let info = system_memory_info();
        assert_eq!(info.available, info.free);
        assert_eq!(info.cached, 0);
        #[cfg(target_os = "linux")]
        {
            let text = std::fs::read_to_string("/proc/meminfo").unwrap();
            let parsed = parse_meminfo(&text);
            assert_eq!(system_memory_info_from(Ok(text)), parsed);
            assert!(parsed.total > 0, "MemTotal was zero");
            assert!(info.total > 0, "live MemTotal was zero");
        }
    }

    #[test]
    fn deno_system_memory_info_runs_as_a_fresh_zero_object() {
        let script = format!(
            r#"
            globalThis.Deno = {{}};
            {SOURCE}
            const first = Deno.systemMemoryInfo();
            const second = Deno.systemMemoryInfo();
            first.total = 9;
            const desc = Object.getOwnPropertyDescriptor(Deno, "systemMemoryInfo");
            process.stdout.write(JSON.stringify({{
              first,
              second,
              distinct: first !== second,
              enumerable: desc.enumerable,
              writable: desc.writable,
              configurable: desc.configurable,
            }}));
            "#
        );
        let value: serde_json::Value = serde_json::from_str(&run_node(&script)).unwrap();
        assert_eq!(value["first"]["total"], 9);
        assert_eq!(value["first"]["free"], 0);
        for key in [
            "total",
            "free",
            "available",
            "buffers",
            "cached",
            "swapTotal",
            "swapFree",
        ] {
            assert_eq!(value["second"][key], 0, "{key}");
        }
        assert_eq!(value["distinct"], true);
        assert_eq!(value["enumerable"], true);
        assert_eq!(value["writable"], true);
        assert_eq!(value["configurable"], true);
    }

    #[test]
    fn the_worker_module_installs_system_memory_info_before_import() {
        let module = crate::bootstrap::module();
        let shim = module.find(SOURCE).unwrap();
        assert!(shim < module.find("await import(").unwrap());
    }
}
