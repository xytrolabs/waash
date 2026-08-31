//! Neofetch-style startup screen for WAASH.
//!
//! Shows a stylized Xytro logo alongside system information
//! (OS, host, kernel, uptime, CPU, GPU, memory, disk, etc.),
//! mirroring the look of `neofetch`/`fastfetch` that users
//! get from fish with an alias.

use ansi_term::Colour::*;
use ansi_term::Style;
use std::path::Path;

/// The Xytro "X" emblem, drawn in block characters — gold, like the brand.
/// Matches `xytrologo.png` (a stylized X) so the startup screen looks like a
/// neofetch/fastfetch banner with WAASH's own logo instead of a distro's.
const XYTRO_LOGO: &[&str] = &[
    "    ████      ████  ",
    "    █████    █████  ",
    "    ██████  ██████  ",
    "     ████████████   ",
    "      ██████████    ",
    "       ████████     ",
    "        ██████      ",
    "       ████████     ",
    "      ██████████    ",
    "     ████████████   ",
    "    ██████  ██████  ",
    "    █████    █████  ",
    "    ████      ████  ",
    "                    ",
    "   WHAT AN AMAZING  ",
    "      SHELL         ",
];

/// Gold color used for the Xytro branding.
fn gold() -> ansi_term::Colour {
    Fixed(220)
}

/// Print the full startup screen.
pub fn print_welcome_screen(version: &str, aliases: usize) {
    let logo_lines: Vec<String> = XYTRO_LOGO
        .iter()
        .map(|line| gold().bold().paint(*line).to_string())
        .collect();

    let info_lines = system_info_lines(version, aliases);

    // Render side-by-side: logo on the left, info on the right.
    let width = XYTRO_LOGO.iter().map(|l| l.chars().count()).max().unwrap_or(20);
    let rows = logo_lines.len().max(info_lines.len());

    for i in 0..rows {
        let left = logo_lines.get(i).cloned().unwrap_or_default();
        let left_plain = XYTRO_LOGO.get(i).map(|s| s.to_string()).unwrap_or_default();
        let pad = width.saturating_sub(left_plain.chars().count());
        let right = info_lines.get(i).map(|s| s.as_str()).unwrap_or("");

        println!("{}{} {}", left, " ".repeat(pad), right);
    }

    println!();
}

/// Collect system info lines, right column of the screen.
fn system_info_lines(version: &str, aliases: usize) -> Vec<String> {
    let mut lines: Vec<(String, String)> = Vec::new();

    lines.push(("OS".into(), os_pretty()));
    lines.push(("Host".into(), hostname()));
    lines.push(("Kernel".into(), kernel()));
    lines.push(("Uptime".into(), uptime()));
    let pkgs = packages();
    if !pkgs.is_empty() {
        lines.push(("Packages".into(), pkgs));
    }
    lines.push(("Shell".into(), format!("waash {}", version)));
    lines.push(("Terminal".into(), "waash".into()));
    if let Some(de) = desktop_env() {
        lines.push(("DE".into(), de));
    }
    if let Some(wm) = window_manager() {
        lines.push(("WM".into(), wm));
    }
    lines.push(("CPU".into(), cpu_model()));
    let gpu = gpu_model();
    if !gpu.is_empty() {
        lines.push(("GPU".into(), gpu));
    }
    lines.push(("Memory".into(), memory()));
    let swap = swap_usage();
    if !swap.is_empty() {
        lines.push(("Swap".into(), swap));
    }
    for disk in disks() {
        lines.push(("Disk".into(), disk));
    }
    if let Some(ip) = local_ip() {
        lines.push(("Local IP".into(), ip));
    }
    lines.push(("Locale".into(), locale()));
    if aliases > 0 {
        lines.push(("Aliases".into(), format!("{} loaded", aliases)));
    }

    // Style: label in gold bold, value dimmed white.
    lines
        .into_iter()
        .map(|(label, value)| {
            format!(
                "{} {}",
                gold().bold().paint(label),
                Style::new().dimmed().paint(value)
            )
        })
        .collect()
}

// ── Info gatherers ──

fn os_pretty() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "Linux".into())
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".into())
        })
}

fn kernel() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn uptime() -> String {
    let secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;

    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;

    match (d, h, m) {
        (0, 0, m) => format!("{} {}", m, if m == 1 { "min" } else { "mins" }),
        (0, h, m) if m == 0 => format!("{} {}", h, if h == 1 { "hour" } else { "hours" }),
        (0, h, m) => format!("{} {}, {} {}", h, if h == 1 { "hour" } else { "hours" }, m, if m == 1 { "min" } else { "mins" }),
        (d, h, _) => format!("{} {}, {} {}", d, if d == 1 { "day" } else { "days" }, h, if h == 1 { "hour" } else { "hours" }),
    }
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn gpu_model() -> String {
    if let Ok(o) = std::process::Command::new("lspci").output() {
        let text = String::from_utf8_lossy(&o.stdout);
        if let Some(line) = text
            .lines()
            .find(|l| l.contains("VGA") || l.contains("3D") || l.contains("Display"))
        {
            // "00:02.0 VGA compatible controller: NVIDIA Corporation AD107 [GeForce RTX 4060]"
            let after = line.splitn(3, ':').nth(2).unwrap_or("").trim();
            // Prefer the human-readable model inside [brackets] when present.
            if let Some(open) = after.find('[') {
                if let Some(close) = after[open..].find(']') {
                    let model = after[open + 1..open + close].trim();
                    if !model.is_empty() {
                        return model.to_string();
                    }
                }
            }
            let cleaned = after.split('[').next().unwrap_or("").trim().to_string();
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
    }
    String::new()
}

fn packages() -> String {
    let mut counts = Vec::new();
    // Flatpak
    if Path::new("/usr/bin/flatpak").exists() {
        if let Ok(o) = std::process::Command::new("flatpak").args(["list", "--app"]).output() {
            let n = String::from_utf8_lossy(&o.stdout).lines().count();
            counts.push(format!("{} (flatpak)", n));
        }
    }
    // Pacman
    if Path::new("/usr/bin/pacman").exists() {
        if let Ok(o) = std::process::Command::new("pacman").arg("-Q").arg("--quiet").output() {
            let n = String::from_utf8_lossy(&o.stdout).lines().count();
            counts.push(format!("{} (pacman)", n));
        }
    }
    // dpkg
    if Path::new("/usr/bin/dpkg").exists() {
        if let Ok(o) = std::process::Command::new("dpkg-query").args(["-f", "${binary:Package}\\n", "-W"]).output() {
            let n = String::from_utf8_lossy(&o.stdout).lines().count();
            counts.push(format!("{} (dpkg)", n));
        }
    }
    // rpm
    if Path::new("/usr/bin/rpm").exists() {
        if let Ok(o) = std::process::Command::new("rpm").args(["-qa"]).output() {
            let n = String::from_utf8_lossy(&o.stdout).lines().count();
            counts.push(format!("{} (rpm)", n));
        }
    }
    counts.join(", ")
}

/// Desktop environment from the environment, e.g. "COSMIC 1.0.0".
fn desktop_env() -> Option<String> {
    std::env::var("XDG_CURRENT_DESKTOP").ok().filter(|s| !s.is_empty())
}

/// Window manager / compositor. On Wayland this is the compositor (e.g.
/// `cosmic-comp (Wayland)`); on X11 fall back to DESKTOP_SESSION or WINDOW_MANAGER.
fn window_manager() -> Option<String> {
    if let Ok(xd) = std::env::var("XDG_SESSION_TYPE") {
        if xd.to_lowercase() == "wayland" {
            if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
                let short = desktop.split_whitespace().next().unwrap_or("").to_lowercase();
                return Some(format!("{} (Wayland)", short));
            }
        }
    }
    for var in ["DESKTOP_SESSION", "WINDOW_MANAGER"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn memory() -> String {
    let (total, avail) = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .map(|c| {
            let total = c
                .lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let avail = c
                .lines()
                .find(|l| l.starts_with("MemAvailable:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            (total, avail)
        })
        .unwrap_or((0, 0));

    if total == 0 {
        return "unknown".into();
    }

    let used = total.saturating_sub(avail);
    let gib = |kb: u64| kb as f64 / 1024.0 / 1024.0;
    let pct = used as f64 / total as f64 * 100.0;
    format!("{:.2} GiB / {:.2} GiB ({:.0}%)", gib(used), gib(total), pct)
}

/// Swap usage from /proc/meminfo, e.g. "2.01 GiB / 46.88 GiB (4%)".
fn swap_usage() -> String {
    let (total, free) = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .map(|c| {
            let total = c
                .lines()
                .find(|l| l.starts_with("SwapTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let free = c
                .lines()
                .find(|l| l.starts_with("SwapFree:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            (total, free)
        })
        .unwrap_or((0, 0));

    if total == 0 {
        return String::new();
    }
    let used = total.saturating_sub(free);
    let gib = |kb: u64| kb as f64 / 1024.0 / 1024.0;
    let pct = used as f64 / total as f64 * 100.0;
    format!("{:.2} GiB / {:.2} GiB ({:.0}%)", gib(used), gib(total), pct)
}

/// Disk usage for the root filesystem plus any mounted removable/media volumes,
/// e.g. "369.88 GiB / 953.37 GiB (39%) - btrfs" and one line per extra mount.
fn disks() -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let Ok(o) = std::process::Command::new("df")
        .args(["-h", "-x", "tmpfs", "-x", "devtmpfs", "-x", "overlay"])
        .output()
    else {
        return out;
    };
    let text = String::from_utf8_lossy(&o.stdout);
    let mut lines = text.lines();
    let _header = lines.next();
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        let fs = parts[0];
        let mount = parts[5];
        // Include root and /run/media/* (and /home), skip others; dedup by device.
        let wanted = mount == "/" || mount == "/home" || mount.starts_with("/run/media/");
        if wanted && seen.insert(fs.to_string()) {
            // parts = [fs, size, used, avail, use%, mount]
            out.push(format!(
                "{} / {} ({}) - {}",
                parts[2], parts[1], parts[4], fs
            ));
        }
    }
    out
}

/// Best-effort local IPv4 address, e.g. "10.0.0.201/24 (wlan0)".
fn local_ip() -> Option<String> {
    if let Ok(o) = std::process::Command::new("ip").args(["-o", "-4", "addr", "show"]).output() {
        let text = String::from_utf8_lossy(&o.stdout);
        // e.g. "2: wlan0    inet 10.0.0.201/24 brd ... scope global dynamic ..."
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let iface = parts[1].trim_end_matches(':');
                let addr = parts[3];
                // Prefer the interface with the default route (wlan0/eth0).
                if iface == "wlan0" || iface == "eth0" || iface == "enp0s3" || iface.starts_with("enp") || iface.starts_with("wlp") {
                    return Some(format!("{} ({})", addr, iface));
                }
            }
        }
    }
    None
}

/// Current locale from the environment, e.g. "en_US.UTF-8".
fn locale() -> String {
    std::env::var("LANG").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "C".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_system_info_lines_no_panic() {
        let lines = system_info_lines("test", 3);
        assert!(!lines.is_empty());
        // Should include OS, Host, Kernel, Shell at minimum
        let joined = lines.join("\n");
        assert!(joined.contains("OS"));
        assert!(joined.contains("Shell"));
    }

    #[test]
    fn test_welcome_screen_renders() {
        // Ensure it doesn't panic; capture output.
        let _ = std::io::stdout().flush();
        print_welcome_screen("0.1.0", 2);
        let _ = std::io::stdout().flush();
    }

    #[test]
    fn test_info_gatherers() {
        assert!(!os_pretty().is_empty());
        assert!(!hostname().is_empty());
        assert!(!kernel().is_empty());
        assert!(!uptime().is_empty());
        assert!(!cpu_model().is_empty());
        assert!(!memory().is_empty());
    }
}

