use anyhow::Result;
use colored::*;
use dialoguer::{theme::ColorfulTheme, Confirm, MultiSelect, Select};

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
    println!("{}", "COOLDOWN".cyan().bold());
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

    // Scan with low threshold to catch everything
    let whitelist = Whitelist::load()?;
    let processes = scan_processes(10.0); // Scan từ 10% CPU
    let analyzed = analyze_processes(&processes, &whitelist);

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

    // Show ALL processes table
    println!("  {} (sorted by CPU)", "ALL PROCESSES".bold());
    println!("  {}", "─".repeat(70));
    println!(
        "  {:>6}  {:22}  {:>6}  {:>6}  {:>8}  {}",
        "PID", "NAME", "CPU%", "MEM", "TIME", "STATUS"
    );
    println!("  {}", "─".repeat(70));

    for ap in all_sorted.iter().take(15) {
        let status = if ap.whitelisted {
            "[WL]".cyan()
        } else if ap.is_system_app {
            "[SYS]".blue()
        } else if ap.issue.is_problematic() {
            format!("[{}]", ap.issue.label()).red()
        } else {
            "[OK]".green()
        };

        let name = truncate(&ap.info.name, 22);
        let name_colored = if !ap.whitelisted && !ap.is_system_app && ap.issue.is_problematic() {
            name.red()
        } else if ap.is_system_app {
            name.blue()
        } else if ap.whitelisted {
            name.cyan()
        } else {
            name.normal()
        };

        let cpu_str = if ap.info.cpu_percent > 100.0 {
            format!("{:>5.0}%", ap.info.cpu_percent).red()
        } else if ap.info.cpu_percent > 50.0 {
            format!("{:>5.0}%", ap.info.cpu_percent).yellow()
        } else {
            format!("{:>5.0}%", ap.info.cpu_percent).normal()
        };

        let mem_str = if ap.info.memory_mb > 1000 {
            format!("{:>5}M", ap.info.memory_mb).yellow()
        } else {
            format!("{:>5}M", ap.info.memory_mb).normal()
        };

        println!(
            "  {:>6}  {}  {}  {}  {:>8}  {}",
            ap.info.pid,
            name_colored,
            cpu_str,
            mem_str,
            ap.running_human(),
            status
        );
    }

    if all_sorted.len() > 15 {
        println!("  {} ... and {} more", "".dimmed(), all_sorted.len() - 15);
    }

    println!("  {}", "─".repeat(70));
    println!();

    // Summary
    let killable_problematic: Vec<&AnalyzedProcess> = killable
        .iter()
        .filter(|ap| ap.issue.is_problematic())
        .copied()
        .collect();

    if !killable_problematic.is_empty() {
        println!(
            "  {} {} killable problematic ({:.0}% CPU)",
            "⚠".yellow(),
            killable_problematic.len(),
            killable_problematic.iter().map(|ap| ap.info.cpu_percent).sum::<f32>()
        );
    }

    println!(
        "  {} {} killable | {} protected",
        "ℹ".blue(),
        killable.len(),
        protected.len()
    );
    println!();

    show_menu(&killable, &killable_problematic)
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        format!("{:width$}", s, width = max)
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{}…", truncated)
    }
}

fn show_menu(killable: &[&AnalyzedProcess], problematic: &[&AnalyzedProcess]) -> Result<()> {
    let mut options = vec![];

    if !problematic.is_empty() {
        let total_cpu: f32 = problematic.iter().map(|ap| ap.info.cpu_percent).sum();
        options.push(format!(
            "Kill all problematic ({} proc, {:.0}% CPU)",
            problematic.len(),
            total_cpu
        ));
    }

    if !killable.is_empty() {
        options.push(format!("Select from {} killable processes", killable.len()));
    }

    if !killable.is_empty() {
        options.push("Throttle a process".to_string());
    }

    options.push("Run maintenance".to_string());
    options.push("Refresh".to_string());
    options.push("Exit".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Action")
        .items(&options)
        .default(0)
        .interact()?;

    let selected = &options[selection];

    if selected.contains("Kill all") {
        kill_all(problematic)?;
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
        return Ok(());
    }

    let total_cpu: f32 = processes.iter().map(|ap| ap.info.cpu_percent).sum();
    let total_mem: u64 = processes.iter().map(|ap| ap.info.memory_mb).sum();

    let confirm = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Kill {} process(es)? (free {:.0}% CPU, {}MB RAM)",
            processes.len(),
            total_cpu,
            total_mem
        ))
        .default(false)
        .interact()?;

    if !confirm {
        println!("  {} Cancelled.", "INFO".dimmed());
        println!();
        return Ok(());
    }

    for ap in processes {
        let result = kill_process(ap.info.pid, &ap.info.name, false);
        if result.success {
            println!("  {} Killed {}", "OK".green().bold(), ap.info.name);
        } else {
            println!(
                "  {} Failed {}: {}",
                "ERR".red().bold(),
                ap.info.name,
                result.error.unwrap_or_default()
            );
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(500));
    println!();
    Ok(())
}

fn select_and_kill(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        return Ok(());
    }

    let items: Vec<String> = processes
        .iter()
        .map(|ap| {
            format!(
                "[{}] {} - {:.0}% CPU, {}MB",
                ap.info.pid, ap.info.name, ap.info.cpu_percent, ap.info.memory_mb
            )
        })
        .collect();

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select processes (Space to toggle, Enter to confirm)")
        .items(&items)
        .interact()?;

    if selections.is_empty() {
        println!("  {} No processes selected.", "INFO".dimmed());
        println!();
        return Ok(());
    }

    for idx in selections {
        let ap = processes[idx];
        let result = kill_process(ap.info.pid, &ap.info.name, false);
        if result.success {
            println!("  {} Killed {}", "OK".green().bold(), ap.info.name);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(500));
    println!();
    Ok(())
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
