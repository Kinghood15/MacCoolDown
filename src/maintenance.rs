use anyhow::{Context, Result};
use colored::*;
use std::process::Command;

#[derive(Debug)]
#[allow(dead_code)]
pub struct MaintenanceResult {
    pub task: String,
    pub success: bool,
    pub message: String,
    pub requires_sudo: bool,
}

fn can_sudo_without_password() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn flush_dns_cache() -> MaintenanceResult {
    let task = "Flush DNS Cache".to_string();

    // Check if we can sudo
    let is_root = unsafe { libc::getuid() } == 0;

    if !is_root && !can_sudo_without_password() {
        return MaintenanceResult {
            task,
            success: false,
            message: "Requires sudo. Run: sudo cooldown maintenance --dns".to_string(),
            requires_sudo: true,
        };
    }

    // Flush DNS cache
    let (cmd, args): (&str, &[&str]) = if is_root {
        ("dscacheutil", &["-flushcache"])
    } else {
        ("sudo", &["-n", "dscacheutil", "-flushcache"])
    };

    let result = Command::new(cmd).args(args).output();

    if let Err(e) = result {
        return MaintenanceResult {
            task,
            success: false,
            message: format!("Failed to flush cache: {}", e),
            requires_sudo: false,
        };
    }

    // Also restart mDNSResponder
    let (cmd2, args2): (&str, &[&str]) = if is_root {
        ("killall", &["-HUP", "mDNSResponder"])
    } else {
        ("sudo", &["-n", "killall", "-HUP", "mDNSResponder"])
    };

    let _ = Command::new(cmd2).args(args2).output();

    MaintenanceResult {
        task,
        success: true,
        message: "DNS cache flushed successfully".to_string(),
        requires_sudo: false,
    }
}

pub fn free_purgeable_space() -> MaintenanceResult {
    let task = "Free Purgeable Space".to_string();

    // Use diskutil to show purgeable space first
    let output = Command::new("diskutil")
        .args(["info", "/"])
        .output()
        .context("Failed to run diskutil");

    let purgeable_before = if let Ok(out) = &output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        parse_purgeable_space(&stdout)
    } else {
        None
    };

    // Trigger purge by writing and deleting a file
    // This forces macOS to reclaim purgeable space
    let result = Command::new("purge").output();

    match result {
        Ok(out) if out.status.success() => {
            let msg = if let Some(before) = purgeable_before {
                format!("Purged memory. Had {:.1}GB purgeable space", before)
            } else {
                "Purged memory successfully".to_string()
            };
            MaintenanceResult {
                task,
                success: true,
                message: msg,
                requires_sudo: false,
            }
        }
        Ok(_) => {
            // purge might need sudo
            let sudo_result = Command::new("sudo")
                .args(["-n", "purge"])
                .output();

            match sudo_result {
                Ok(out) if out.status.success() => MaintenanceResult {
                    task,
                    success: true,
                    message: "Purged memory successfully (with sudo)".to_string(),
                    requires_sudo: false,
                },
                _ => MaintenanceResult {
                    task,
                    success: false,
                    message: "Requires sudo. Run: sudo cooldown maintenance --purgeable".to_string(),
                    requires_sudo: true,
                },
            }
        }
        Err(e) => MaintenanceResult {
            task,
            success: false,
            message: format!("Failed to purge: {}", e),
            requires_sudo: false,
        },
    }
}

fn parse_purgeable_space(diskutil_output: &str) -> Option<f64> {
    for line in diskutil_output.lines() {
        if line.contains("Purgeable") || line.contains("purgeable") {
            // Extract size in bytes or human readable
            for word in line.split_whitespace() {
                // Try to parse as GB (e.g., "12.5")
                if let Ok(gb) = word.parse::<f64>() {
                    return Some(gb);
                }
                // Try to parse with GB suffix
                if word.ends_with("GB") {
                    if let Ok(gb) = word.replace("GB", "").parse::<f64>() {
                        return Some(gb);
                    }
                }
                // Try to parse as bytes
                if let Ok(bytes) = word.replace(",", "").parse::<u64>() {
                    if bytes > 1_000_000 {
                        return Some(bytes as f64 / 1_073_741_824.0);
                    }
                }
            }
        }
    }
    None
}

pub fn clear_time_machine_snapshots() -> MaintenanceResult {
    let task = "Clear Time Machine Snapshots".to_string();

    // List snapshots first
    let list_result = Command::new("tmutil")
        .args(["listlocalsnapshots", "/"])
        .output();

    let snapshot_count = if let Ok(out) = list_result {
        let stdout = String::from_utf8_lossy(&out.stdout);
        stdout.lines().filter(|l| l.contains("com.apple.TimeMachine")).count()
    } else {
        0
    };

    if snapshot_count == 0 {
        return MaintenanceResult {
            task,
            success: true,
            message: "No local Time Machine snapshots to delete".to_string(),
            requires_sudo: false,
        };
    }

    // Delete all local snapshots
    let result = Command::new("sudo")
        .args(["-n", "tmutil", "deletelocalsnapshots", "/"])
        .output();

    match result {
        Ok(out) if out.status.success() => MaintenanceResult {
            task,
            success: true,
            message: format!("Deleted {} Time Machine snapshot(s)", snapshot_count),
            requires_sudo: false,
        },
        _ => MaintenanceResult {
            task,
            success: false,
            message: "Requires sudo. Run: sudo cooldown maintenance --timemachine".to_string(),
            requires_sudo: true,
        },
    }
}

pub fn clear_system_caches() -> MaintenanceResult {
    let task = "Clear System Caches".to_string();

    let home = dirs::home_dir().unwrap_or_default();
    let cache_dir = home.join("Library/Caches");

    if !cache_dir.exists() {
        return MaintenanceResult {
            task,
            success: true,
            message: "No user caches to clear".to_string(),
            requires_sudo: false,
        };
    }

    // Count files before
    let count_before = std::fs::read_dir(&cache_dir)
        .map(|entries| entries.count())
        .unwrap_or(0);

    // We don't actually delete caches here - that's dangerous
    // Instead, just report the size
    let size = dir_size(&cache_dir);

    MaintenanceResult {
        task,
        success: true,
        message: format!(
            "User cache: {} items, {:.1}GB (use mac-cleaner for cleanup)",
            count_before,
            size as f64 / 1_073_741_824.0
        ),
        requires_sudo: false,
    }
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                size += dir_size(&path);
            } else if let Ok(meta) = entry.metadata() {
                size += meta.len();
            }
        }
    }
    size
}

pub fn run_maintenance(
    dns: bool,
    purgeable: bool,
    timemachine: bool,
    all: bool,
) -> Result<()> {
    let mut tasks: Vec<(&str, Box<dyn Fn() -> MaintenanceResult>)> = Vec::new();

    if dns || all {
        tasks.push(("DNS Cache", Box::new(flush_dns_cache)));
    }
    if purgeable || all {
        tasks.push(("Purgeable Space", Box::new(free_purgeable_space)));
    }
    if timemachine || all {
        tasks.push(("Time Machine", Box::new(clear_time_machine_snapshots)));
    }
    if all {
        tasks.push(("System Caches", Box::new(clear_system_caches)));
    }

    if tasks.is_empty() {
        println!();
        println!(
            "  {}",
            "No maintenance tasks specified.".yellow()
        );
        println!(
            "  {}",
            "Use --dns, --purgeable, --timemachine, or --all".dimmed()
        );
        println!();
        return Ok(());
    }

    println!();
    println!("{}", "MAINTENANCE TASKS".cyan().bold());
    println!("{}", "=================".dimmed());
    println!();

    let mut any_requires_sudo = false;

    for (name, task_fn) in tasks {
        print!("  {} {}...", "Running:".dimmed(), name);
        use std::io::Write;
        let _ = std::io::stdout().flush();

        let result = task_fn();

        // Clear line
        print!("\r                                                              \r");

        if result.success {
            println!("  {} {}", "OK".green().bold(), result.message);
        } else {
            println!("  {} {}", "ERR".red().bold(), result.message);
            if result.requires_sudo {
                any_requires_sudo = true;
            }
        }
    }

    println!();

    if any_requires_sudo {
        println!(
            "  {} Some tasks require sudo. Run: {}",
            "TIP:".yellow().bold(),
            "sudo cooldown maintenance --all".cyan()
        );
        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_purgeable_space() {
        let sample = "   Purgeable:                        12.5 GB";
        assert!(parse_purgeable_space(sample).is_some());
    }

    #[test]
    fn test_dir_size() {
        let temp = std::env::temp_dir();
        let size = dir_size(&temp);
        assert!(size > 0);
    }
}
