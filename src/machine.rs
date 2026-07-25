use std::process::Command;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectedMachine {
    pub total_unified_gb: f64,
    pub chip: Option<String>,
    pub platform: &'static str,
}

fn cmd_out(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Detect the physical memory of the machine this process runs on.
/// macOS first (sysctl), then Linux (/proc/meminfo). None elsewhere.
pub fn detect() -> Option<DetectedMachine> {
    if let Some(mem) = cmd_out("sysctl", &["-n", "hw.memsize"]) {
        if let Ok(bytes) = mem.trim().parse::<f64>() {
            if bytes > 0.0 {
                let chip = cmd_out("sysctl", &["-n", "machdep.cpu.brand_string"])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                return Some(DetectedMachine {
                    total_unified_gb: round1(bytes / (1u64 << 30) as f64),
                    chip,
                    platform: "macos",
                });
            }
        }
    }
    if let Ok(info) = std::fs::read_to_string("/proc/meminfo") {
        for line in info.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                if let Ok(kb) = rest.trim().trim_end_matches("kB").trim().parse::<f64>() {
                    if kb > 0.0 {
                        let chip = std::fs::read_to_string("/proc/cpuinfo")
                            .ok()
                            .and_then(|c| {
                                c.lines()
                                    .find(|l| l.starts_with("model name"))
                                    .and_then(|l| l.split(':').nth(1))
                                    .map(|s| s.trim().to_string())
                            })
                            .filter(|s| !s.is_empty());
                        return Some(DetectedMachine {
                            total_unified_gb: round1(kb / 1048576.0),
                            chip,
                            platform: "linux",
                        });
                    }
                }
            }
        }
    }
    None
}
