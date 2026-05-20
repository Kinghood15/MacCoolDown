use anyhow::Result;
use colored::*;
use dialoguer::{theme::ColorfulTheme, Select};

use crate::analyzer::{analyze_processes, AnalyzedProcess};
use crate::killer::kill_process;
use crate::maintenance::run_maintenance;
use crate::scanner::{get_cpu_usage, get_memory_info, get_system_load, scan_processes};
use crate::thermal::{get_thermal_status, PowerSource, ThermalStatus};
use crate::whitelist::Whitelist;

pub fn run_interactive() -> Result<()> {
    run_interactive_inner()
}

pub fn run_realtime() -> Result<()> {
    run_interactive_inner()
}

fn run_interactive_inner() -> Result<()> {
    let thermal = get_thermal_status().unwrap_or_default();

    println!();
    println!("{}", "MAC-COOLDOWN".cyan().bold());
    println!("{}", "========".dimmed());

    // System status
    let load = get_system_load();
    let mem = get_memory_info();
    let cpu = get_cpu_usage();

    println!();
    println!(
        "  {} Load {:.2} | CPU {:.0}% | Mem {}GB/{}GB",
        "System:".bold(),
        load.0,
        cpu,
        mem.0,
        mem.1
    );

    // Thermal
    print!("  {} {} {}", "Thermal:".bold(), thermal.level.emoji(), thermal.level.colored_label());
    if let Some(temp) = thermal.cpu_temp {
        print!(" | {}", color_temp(temp));
    }
    print!(
        " | {} {}",
        thermal.power_source.emoji(),
        thermal.battery_percent
            .map(|p| format!("{}%", p))
            .unwrap_or_else(|| thermal.power_source.label().to_string())
    );
    println!();

    // Scan ALL processes to catch idle sessions of important apps
    let whitelist = Whitelist::load()?;
    let all_processes = scan_processes(0.0);
    let analyzed = analyze_processes(&all_processes, &whitelist);

    // Always show ALL instances of these apps (regardless of CPU)
    const ALWAYS_SHOW_ALL: &[&str] = &["claude"];

    fn is_always_show(name: &str) -> bool {
        let name_lower = name.to_lowercase();
        ALWAYS_SHOW_ALL.iter().any(|&app| name_lower.starts_with(app))
    }

    // Filter: CPU >= 5% OR is in always-show list
    let analyzed: Vec<_> = analyzed
        .into_iter()
        .filter(|ap| ap.info.cpu_percent >= 5.0 || is_always_show(&ap.info.name))
        .collect();

    // Sort all by CPU descending
    let mut all_sorted: Vec<&AnalyzedProcess> = analyzed.iter().collect();
    all_sorted.sort_by(|a, b| b.info.cpu_percent.partial_cmp(&a.info.cpu_percent).unwrap());

    // Categorize
    let killable: Vec<&AnalyzedProcess> = all_sorted
        .iter()
        .filter(|ap| !ap.whitelisted && !ap.is_system_app)
        .copied()
        .collect();

    let protected: Vec<&AnalyzedProcess> = all_sorted
        .iter()
        .filter(|ap| ap.is_system_app || ap.whitelisted)
        .copied()
        .collect();

    println!();

    // Categorize KILLABLE processes by heat level
    let hot_killable: Vec<&AnalyzedProcess> = killable.iter()
        .filter(|ap| ap.info.cpu_percent >= 50.0)
        .copied()
        .collect();
    let warm_killable: Vec<&AnalyzedProcess> = killable.iter()
        .filter(|ap| ap.info.cpu_percent >= 20.0 && ap.info.cpu_percent < 50.0)
        .copied()
        .collect();
    let cool_killable: Vec<&AnalyzedProcess> = killable.iter()
        .filter(|ap| ap.info.cpu_percent < 20.0)
        .copied()
        .collect();

    // Show KILLABLE processes by heat level
    println!("  {}", "KILLABLE PROCESSES".bold().underline());

    if !hot_killable.is_empty() {
        let hot_cpu: f32 = hot_killable.iter().map(|ap| ap.info.cpu_percent).sum();
        println!(
            "  {} {} process ({:.0}% CPU) - có thể kill ngay!",
            "🔥 HOT:".red().bold(),
            hot_killable.len(),
            hot_cpu
        );
        for ap in &hot_killable {
            let issue = if ap.issue.is_problematic() {
                format!(" [{}]", ap.issue.label()).red().to_string()
            } else {
                String::new()
            };
            println!(
                "       {} [{}] {} - {:.0}% CPU, {}MB, {}{}",
                "•".red(),
                ap.info.pid.to_string().bold(),
                ap.info.name.red().bold(),
                ap.info.cpu_percent,
                ap.info.memory_mb,
                ap.running_human(),
                issue
            );
        }
    }

    if !warm_killable.is_empty() {
        let warm_cpu: f32 = warm_killable.iter().map(|ap| ap.info.cpu_percent).sum();
        println!(
            "  {} {} process ({:.0}% CPU)",
            "🌡️ WARM:".yellow().bold(),
            warm_killable.len(),
            warm_cpu
        );
        for ap in &warm_killable {
            println!(
                "       {} [{}] {} - {:.0}% CPU, {}MB, {}",
                "•".yellow(),
                ap.info.pid,
                ap.info.name.yellow(),
                ap.info.cpu_percent,
                ap.info.memory_mb,
                ap.running_human()
            );
        }
    }

    if !cool_killable.is_empty() {
        println!(
            "  {} {} process (<20% CPU)",
            "❄️ COOL:".green(),
            cool_killable.len()
        );
        for ap in cool_killable.iter().take(3) {
            println!(
                "       {} [{}] {} - {:.0}% CPU",
                "•".green(),
                ap.info.pid,
                ap.info.name.dimmed(),
                ap.info.cpu_percent
            );
        }
        if cool_killable.len() > 3 {
            println!("       ... +{} more", cool_killable.len() - 3);
        }
    }

    if killable.is_empty() {
        println!("  {} Không có process nào có thể kill", "✓".green());
    }

    println!();

    // Show PROTECTED processes (dev tools, system apps)
    println!("  {}", "PROTECTED (không kill)".blue().bold());
    for ap in protected.iter().take(5) {
        let heat = if ap.info.cpu_percent >= 50.0 {
            "🔥".to_string()
        } else if ap.info.cpu_percent >= 20.0 {
            "🌡️".to_string()
        } else {
            "❄️".to_string()
        };
        println!(
            "       {} [{}] {} - {:.0}% CPU, {}MB, {}",
            heat,
            ap.info.pid,
            ap.info.name.blue(),
            ap.info.cpu_percent,
            ap.info.memory_mb,
            ap.running_human()
        );
    }
    if protected.len() > 5 {
        println!("       ... +{} more", protected.len() - 5);
    }
    println!();

    // Summary line
    println!("  {}", "─".repeat(50));

    show_menu(&killable, &hot_killable, &warm_killable)
}

#[allow(dead_code)]
fn get_adaptive_threshold(thermal: &ThermalStatus) -> u32 {
    let base: u32 = match thermal.power_source {
        PowerSource::Battery => 30,
        PowerSource::AC => 50,
        PowerSource::Unknown => 40,
    };
    if thermal.level.is_dangerous() {
        base.saturating_sub(10)
    } else {
        base
    }
}

fn color_temp(temp: f64) -> ColoredString {
    if temp > 90.0 {
        format!("{:.0}°C", temp).red().bold()
    } else if temp > 75.0 {
        format!("{:.0}°C", temp).yellow()
    } else {
        format!("{:.0}°C", temp).green()
    }
}

#[allow(dead_code)]
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        format!("{:width$}", s, width = max)
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{}…", truncated)
    }
}

#[allow(dead_code)]
fn group_by_name(processes: &[&AnalyzedProcess]) -> Vec<(String, usize, f32)> {
    use std::collections::HashMap;
    let mut groups: HashMap<String, (usize, f32)> = HashMap::new();

    for ap in processes {
        let entry = groups.entry(ap.info.name.clone()).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += ap.info.cpu_percent;
    }

    let mut result: Vec<(String, usize, f32)> = groups
        .into_iter()
        .map(|(name, (count, cpu))| (name, count, cpu))
        .collect();

    // Sort by CPU descending
    result.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    result
}

#[allow(dead_code)]
fn format_process_list(processes: &[&AnalyzedProcess]) -> String {
    if processes.is_empty() {
        return "none".to_string();
    }

    // Show each process with PID (top 5)
    let items: Vec<String> = processes
        .iter()
        .take(5)
        .map(|ap| {
            format!("{}[{}] ({:.0}%)", ap.info.name, ap.info.pid, ap.info.cpu_percent)
        })
        .collect();

    let mut result = items.join(", ");
    if processes.len() > 5 {
        result.push_str(&format!(" +{} more", processes.len() - 5));
    }
    result
}

#[allow(dead_code)]
fn format_breakdown(groups: &[(String, usize, f32)]) -> String {
    if groups.is_empty() {
        return "none".to_string();
    }

    // Show top 5 process names
    let items: Vec<String> = groups
        .iter()
        .take(5)
        .map(|(name, count, cpu)| {
            if *count > 1 {
                format!("{}x{} ({:.0}%)", name, count, cpu)
            } else {
                format!("{} ({:.0}%)", name, cpu)
            }
        })
        .collect();

    let mut result = items.join(", ");
    if groups.len() > 5 {
        result.push_str(&format!(" +{} more", groups.len() - 5));
    }
    result
}

fn show_menu(killable: &[&AnalyzedProcess], hot_killable: &[&AnalyzedProcess], warm_killable: &[&AnalyzedProcess]) -> Result<()> {
    let mut options = vec![];

    // Option to kill HOT processes (>50% CPU)
    if !hot_killable.is_empty() {
        let total_cpu: f32 = hot_killable.iter().map(|ap| ap.info.cpu_percent).sum();
        options.push(format!(
            "🔥 Kill {} HOT ({:.0}% CPU)",
            hot_killable.len(),
            total_cpu
        ));
    }

    // Option to kill WARM processes (20-50% CPU)
    if !warm_killable.is_empty() {
        let total_cpu: f32 = warm_killable.iter().map(|ap| ap.info.cpu_percent).sum();
        options.push(format!(
            "🌡️ Kill {} WARM ({:.0}% CPU)",
            warm_killable.len(),
            total_cpu
        ));
    }

    if !killable.is_empty() {
        options.push(format!("📋 Select from {} killable processes", killable.len()));
    }

    if !killable.is_empty() {
        options.push("⏱️ Throttle a process".to_string());
    }

    options.push("🔧 Run maintenance".to_string());
    options.push("🔄 Refresh".to_string());
    options.push("❌ Exit".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Action")
        .items(&options)
        .default(0)
        .interact()?;

    let selected = &options[selection];

    if selected.contains("Kill HOT") {
        kill_all(&hot_killable)?;
        run_interactive_inner()
    } else if selected.contains("Kill WARM") {
        kill_all(&warm_killable)?;
        run_interactive_inner()
    } else if selected.contains("Select from") {
        select_and_kill(killable)?;
        run_interactive_inner()
    } else if selected.contains("Throttle") {
        throttle_interactive(killable)?;
        run_interactive_inner()
    } else if selected.contains("maintenance") {
        run_maintenance_menu()?;
        run_interactive_inner()
    } else if selected.contains("Refresh") {
        run_interactive_inner()
    } else {
        println!();
        Ok(())
    }
}

#[allow(dead_code)]
fn show_menu_no_issues() -> Result<()> {
    let options = vec!["Run maintenance", "Refresh", "Exit"];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What would you like to do?")
        .items(&options)
        .default(0)
        .interact()?;

    match selection {
        0 => {
            run_maintenance_menu()?;
            run_interactive_inner()
        }
        1 => run_interactive_inner(),
        _ => {
            println!();
            Ok(())
        }
    }
}

fn kill_all(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        println!("  {} No problematic processes to kill.", "INFO".dimmed());
        println!();
        return Ok(());
    }

    let total_cpu: f32 = processes.iter().map(|ap| ap.info.cpu_percent).sum();
    let total_mem: u64 = processes.iter().map(|ap| ap.info.memory_mb).sum();

    println!();
    println!("  {} Processes to kill:", "TARGET".yellow().bold());
    for ap in processes {
        println!(
            "    • {} (PID {}) - {:.0}% CPU, {}MB",
            ap.info.name, ap.info.pid, ap.info.cpu_percent, ap.info.memory_mb
        );
    }
    println!();

    let kill_options = vec![
        format!("Kill all {} (SIGTERM)", processes.len()),
        format!("Force kill all {} (SIGKILL)", processes.len()),
        "← Cancel".to_string(),
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Free {:.0}% CPU, {}MB RAM", total_cpu, total_mem))
        .items(&kill_options)
        .default(0)
        .interact()?;

    let force = match selection {
        0 => false,
        1 => true,
        _ => {
            println!("  {} Cancelled.", "INFO".dimmed());
            println!();
            return Ok(());
        }
    };

    println!();
    let mut killed = 0;
    let mut failed = 0;

    for ap in processes {
        let result = kill_process(ap.info.pid, &ap.info.name, force);
        if result.success {
            println!(
                "  {} {} (PID {})",
                "KILLED".green().bold(),
                ap.info.name,
                ap.info.pid
            );
            killed += 1;
        } else {
            println!(
                "  {} {} - {}",
                "FAILED".red().bold(),
                ap.info.name,
                result.error.unwrap_or_default()
            );
            failed += 1;
        }
    }

    println!();
    println!(
        "  {} {} killed, {} failed",
        "DONE".cyan().bold(),
        killed,
        failed
    );

    std::thread::sleep(std::time::Duration::from_millis(500));
    println!();
    Ok(())
}

fn select_and_kill(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        println!("  {} No killable processes.", "INFO".dimmed());
        println!();
        return Ok(());
    }

    loop {
        let items: Vec<String> = processes
            .iter()
            .map(|ap| {
                format!(
                    "[{}] {} - {:.0}% CPU, {}MB",
                    ap.info.pid, ap.info.name, ap.info.cpu_percent, ap.info.memory_mb
                )
            })
            .collect();

        let mut items_with_options = items.clone();
        items_with_options.push("← Back to main menu".to_string());

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select process to kill")
            .items(&items_with_options)
            .default(0)
            .interact()?;

        // Back option
        if selection == items.len() {
            return Ok(());
        }

        let ap = processes[selection];
        println!();
        println!(
            "  {} {} (PID {})",
            "TARGET:".yellow().bold(),
            ap.info.name.yellow(),
            ap.info.pid
        );
        println!(
            "  {} {:.0}% CPU, {}MB RAM, running {}",
            "INFO:".dimmed(),
            ap.info.cpu_percent,
            ap.info.memory_mb,
            ap.running_human()
        );
        println!();

        let kill_options = vec![
            "Kill (SIGTERM - graceful)",
            "Force Kill (SIGKILL - immediate)",
            "← Cancel",
        ];

        let kill_selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("How to kill?")
            .items(&kill_options)
            .default(0)
            .interact()?;

        match kill_selection {
            0 => {
                // SIGTERM
                let result = kill_process(ap.info.pid, &ap.info.name, false);
                if result.success {
                    println!("  {} Sent SIGTERM to {}", "OK".green().bold(), ap.info.name);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    // Check if process still exists
                    if process_still_running(ap.info.pid) {
                        println!("  {} Process still running. Try Force Kill?", "WARN".yellow());
                    } else {
                        println!("  {} Process terminated.", "OK".green());
                    }
                } else {
                    println!(
                        "  {} Failed: {}",
                        "ERR".red().bold(),
                        result.error.unwrap_or_default()
                    );
                }
            }
            1 => {
                // SIGKILL
                let result = kill_process(ap.info.pid, &ap.info.name, true);
                if result.success {
                    println!("  {} Force killed {}", "OK".green().bold(), ap.info.name);
                } else {
                    println!(
                        "  {} Failed: {}",
                        "ERR".red().bold(),
                        result.error.unwrap_or_default()
                    );
                }
            }
            _ => {
                println!("  {} Cancelled.", "INFO".dimmed());
            }
        }

        println!();
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Ask if want to kill more using Select instead of Confirm
        let next_options = vec![
            "Kill another process",
            "← Back to main menu",
        ];

        let next_selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("What next?")
            .items(&next_options)
            .default(1)
            .interact()?;

        if next_selection == 1 {
            return Ok(());
        }
        println!();
    }
}

fn process_still_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::kill;
        use nix::unistd::Pid as NixPid;
        use nix::errno::Errno;
        match kill(NixPid::from_raw(pid as i32), None) {
            Ok(()) => true,
            Err(Errno::ESRCH) => false,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn throttle_interactive(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        println!("  {} No processes to throttle.", "INFO".dimmed());
        return Ok(());
    }

    let items: Vec<String> = processes
        .iter()
        .map(|ap| {
            format!(
                "[{}] {} - {:.0}% CPU → 50%",
                ap.info.pid, ap.info.name, ap.info.cpu_percent
            )
        })
        .collect();

    let mut items_with_back = items.clone();
    items_with_back.push("← Back".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select process to throttle to 50% CPU")
        .items(&items_with_back)
        .interact()?;

    if selection == items.len() {
        return Ok(());
    }

    let ap = processes[selection];
    println!();
    println!(
        "  {} Throttling {} to 50% CPU (Ctrl+C to stop)",
        "THROTTLE".cyan().bold(),
        ap.info.name.cyan()
    );

    crate::throttle::run_throttle(ap.info.pid, 50, None)?;
    Ok(())
}

fn run_maintenance_menu() -> Result<()> {
    let options = vec![
        "Flush DNS cache",
        "Free purgeable space",
        "Clear Time Machine snapshots",
        "Run all",
        "← Back",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select maintenance task")
        .items(&options)
        .interact()?;

    println!();

    match selection {
        0 => run_maintenance(true, false, false, false)?,
        1 => run_maintenance(false, true, false, false)?,
        2 => run_maintenance(false, false, true, false)?,
        3 => run_maintenance(false, false, false, true)?,
        _ => {}
    }

    Ok(())
}
