use std::process::Command;

use chrono::Local;
use regex::Regex;
use serde_json::Value;

// ── GPU snapshot ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct GpuSnapshot {
    id: u32,
    name: String,
    gfx_ver: String,
    temp: f64,
    power: f64,
    power_cap: f64,
    gpu_pct: f64,
    vram_total: u64,
    vram_used: u64,
}

// ── Data collection ─────────────────────────────────────────────────────────

fn run_cmd(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn parse_float(s: &str) -> f64 {
    let re = Regex::new(r"([\d.]+)").unwrap();
    re.captures(s)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0.0)
}

fn get_driver_version() -> String {
    let out = run_cmd("rocm-smi", &["--showdriver"]);
    let re = Regex::new(r"Driver version:\s*(\S+)").unwrap();
    re.captures(&out)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "N/A".to_string())
}

fn get_rocm_version() -> String {
    if let Ok(content) = std::fs::read_to_string("/opt/rocm/.info/version") {
        let v = content.trim().split('-').next().unwrap_or("N/A");
        if v.starts_with(|c: char| c.is_ascii_digit()) {
            return v.to_string();
        }
    }
    "N/A".to_string()
}

fn get_gpu_data() -> Vec<GpuSnapshot> {
    let mut gpus: Vec<GpuSnapshot> = Vec::new();

    let concise = run_cmd("rocm-smi", &[]);
    for line in concise.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 15 {
            continue;
        }
        if let Ok(id) = parts[0].parse::<u32>() {
            gpus.push(GpuSnapshot {
                id,
                temp: parse_float(parts[4]),
                power: parse_float(parts[5]),
                power_cap: parse_float(parts[13]),
                gpu_pct: if parts.len() > 15 {
                    parse_float(parts[15])
                } else {
                    0.0
                },
                ..Default::default()
            });
        }
    }

    let vram_json = run_cmd("rocm-smi", &["--showmeminfo", "vram", "--json"]);
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&vram_json) {
        for (i, gpu) in gpus.iter_mut().enumerate() {
            let key = format!("card{}", i);
            if let Some(card) = map.get(&key) {
                if let Some(total) = card.get("VRAM Total Memory (B)") {
                    gpu.vram_total = total
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| total.as_u64())
                        .unwrap_or(0);
                }
                if let Some(used) = card.get("VRAM Total Used Memory (B)") {
                    gpu.vram_used = used
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| used.as_u64())
                        .unwrap_or(0);
                }
            }
        }
    }

    let prod = run_cmd("rocm-smi", &["--showproductname"]);
    let re_name = Regex::new(r"GPU\[(\d+)\]\s*:\s*Card Series:\s*(.*)").unwrap();
    for cap in re_name.captures_iter(&prod) {
        if let (Some(idx), Some(name)) = (cap.get(1), cap.get(2)) {
            if let Ok(i) = idx.as_str().parse::<usize>() {
                if i < gpus.len() {
                    gpus[i].name = name.as_str().trim().to_string();
                }
            }
        }
    }

    let hw = run_cmd("rocm-smi", &["--showhw"]);
    for line in hw.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 10 {
            if let Ok(id) = parts[0].parse::<usize>() {
                if id < gpus.len() {
                    gpus[id].gfx_ver = parts[4].to_string();
                }
            }
        }
    }

    gpus
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn bytes_to_gib(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0 * 1024.0)
}

fn bytes_to_mib(b: u64) -> u64 {
    b / (1024 * 1024)
}

// ── ANSI colors ─────────────────────────────────────────────────────────────

mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const RED: &str = "\x1b[91m";
    pub const GREEN: &str = "\x1b[92m";
    pub const YELLOW: &str = "\x1b[93m";
    pub const CYAN: &str = "\x1b[96m";
    pub const WHITE: &str = "\x1b[97m";
}

fn ansi_temp(val: f64) -> &'static str {
    if val >= 90.0 { ansi::RED }
    else if val >= 75.0 { ansi::YELLOW }
    else if val >= 50.0 { ansi::WHITE }
    else { ansi::GREEN }
}

fn ansi_ratio(ratio: f64) -> &'static str {
    if ratio >= 0.9 { ansi::RED }
    else if ratio >= 0.7 { ansi::YELLOW }
    else if ratio > 0.0 { ansi::GREEN }
    else { ansi::DIM }
}

/// Visible length of a string (strips ANSI escapes)
fn vlen(s: &str) -> usize {
    let re = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").len()
}

fn rpad(s: &str, w: usize) -> String {
    let vis = vlen(s);
    if vis < w {
        format!("{}{}", s, " ".repeat(w - vis))
    } else {
        s.to_string()
    }
}

fn lpad(s: &str, w: usize) -> String {
    let vis = vlen(s);
    if vis < w {
        format!("{}{}", " ".repeat(w - vis), s)
    } else {
        s.to_string()
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let gpus = get_gpu_data();
    let driver = get_driver_version();
    let rocm = get_rocm_version();
    let timestamp = Local::now().format("%a %b %d %H:%M:%S %Y").to_string();
    let w: usize = 79; // inner width

    let gpu_name = if !gpus.is_empty() { gpus[0].name.as_str() } else { "AMD GPU" };
    let gfx_ver = if !gpus.is_empty() { gpus[0].gfx_ver.as_str() } else { "" };

    // Header
    println!("{}╔{}╗{}", ansi::CYAN, "═".repeat(w), ansi::RESET);

    let title = format!("{}{}pretty-rocm-smi{}", ansi::BOLD, ansi::CYAN, ansi::RESET);
    let ts = format!("{}{}{}", ansi::DIM, timestamp, ansi::RESET);
    let pad = w - 2 - "pretty-rocm-smi".len() - timestamp.len();
    println!("{}║{} {}{}{} {}║{}", ansi::CYAN, ansi::RESET, title, " ".repeat(pad), ts, ansi::CYAN, ansi::RESET);

    let info = format!(
        "{}{}{} {}({}){} {}Driver: {}{}{}  {}ROCm: {}{}{}",
        ansi::BOLD, gpu_name, ansi::RESET,
        ansi::DIM, gfx_ver, ansi::RESET,
        ansi::DIM, ansi::RESET, ansi::BOLD, driver,
        ansi::DIM, ansi::RESET, ansi::BOLD, rocm,
    );
    let info_plain = format!("{} ({}) Driver: {}  ROCm: {}", gpu_name, gfx_ver, driver, rocm);
    let info_pad = w.saturating_sub(1 + info_plain.len());
    println!("{}║{} {}{}{}{}║{}", ansi::CYAN, ansi::RESET, info, ansi::RESET, " ".repeat(info_pad), ansi::CYAN, ansi::RESET);

    println!("{}╠{}╣{}", ansi::CYAN, "═".repeat(w), ansi::RESET);

    // Column headers
    let hdr = format!(
        " {}{}GPU   Temp    Power Usage         VRAM Usage           GPU%{}",
        ansi::BOLD, ansi::WHITE, ansi::RESET
    );
    println!("{}║{}{}{}║{}", ansi::CYAN, ansi::RESET, rpad(&hdr, w), ansi::CYAN, ansi::RESET);
    println!("{}╟{}╢{}", ansi::CYAN, "─".repeat(w), ansi::RESET);

    // GPU rows
    for gpu in &gpus {
        let id_s = format!("{}{}{:>3}{}", ansi::BOLD, ansi::WHITE, gpu.id, ansi::RESET);

        let tc = ansi_temp(gpu.temp);
        let temp_s = format!("{}{:.0}°C{}", tc, gpu.temp, ansi::RESET);

        let pwr_ratio = if gpu.power_cap > 0.0 { gpu.power / gpu.power_cap } else { 0.0 };
        let pc = ansi_ratio(pwr_ratio);
        let power_s = format!(
            "{}{:.0}W{} {}/ {:.0}W{}",
            pc, gpu.power, ansi::RESET, ansi::DIM, gpu.power_cap, ansi::RESET
        );

        let vram_used_gib = bytes_to_gib(gpu.vram_used);
        let vram_total_gib = bytes_to_gib(gpu.vram_total);
        let vr = if gpu.vram_total > 0 { gpu.vram_used as f64 / gpu.vram_total as f64 } else { 0.0 };
        let vc = ansi_ratio(vr);
        let used_disp = if bytes_to_mib(gpu.vram_used) < 1024 {
            format!("{}MiB", bytes_to_mib(gpu.vram_used))
        } else {
            format!("{:.1}GiB", vram_used_gib)
        };
        let vram_s = format!(
            "{}{}{} {}/ {:.0}GiB{}",
            vc, used_disp, ansi::RESET, ansi::DIM, vram_total_gib, ansi::RESET
        );

        let uc = ansi_ratio(gpu.gpu_pct / 100.0);
        let bold = if gpu.gpu_pct >= 90.0 { ansi::BOLD } else { "" };
        let util_s = format!("{}{}{:.0}%{}", uc, bold, gpu.gpu_pct, ansi::RESET);

        let row = format!(
            " {}   {}   {}   {}   {}",
            lpad(&id_s, 3),
            lpad(&temp_s, 5),
            rpad(&power_s, 19),
            rpad(&vram_s, 19),
            lpad(&util_s, 4),
        );

        println!("{}║{}{}{}║{}", ansi::CYAN, ansi::RESET, rpad(&row, w), ansi::CYAN, ansi::RESET);
    }

    println!("{}╚{}╝{}", ansi::CYAN, "═".repeat(w), ansi::RESET);

    // Summary
    let total_vram: u64 = gpus.iter().map(|g| g.vram_total).sum();
    let used_vram: u64 = gpus.iter().map(|g| g.vram_used).sum();
    let total_power: f64 = gpus.iter().map(|g| g.power).sum();
    let total_cap: f64 = gpus.iter().map(|g| g.power_cap).sum();
    let temps: Vec<f64> = gpus.iter().map(|g| g.temp).collect();
    let avg_temp = if !temps.is_empty() { temps.iter().sum::<f64>() / temps.len() as f64 } else { 0.0 };

    let tc = ansi_temp(avg_temp);
    let pr = if total_cap > 0.0 { total_power / total_cap } else { 0.0 };
    let pc = ansi_ratio(pr);

    println!(
        " {}Total:{} {}{}{} GPUs  {}│{}  VRAM: {:.1}/{:.0} GiB  {}│{}  Power: {}{:.0}W{}{}/{:.0}W  {}│{}  Avg Temp: {}{:.0}°C{}",
        ansi::DIM, ansi::RESET,
        ansi::BOLD, gpus.len(), ansi::RESET,
        ansi::DIM, ansi::RESET,
        bytes_to_gib(used_vram), bytes_to_gib(total_vram),
        ansi::DIM, ansi::RESET,
        pc, total_power, ansi::RESET, ansi::DIM, total_cap,
        ansi::DIM, ansi::RESET,
        tc, avg_temp, ansi::RESET,
    );
    println!();
}
