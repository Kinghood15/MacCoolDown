use anyhow::Result;
use colored::*;
use dialoguer::{theme::ColorfulTheme, Confirm, MultiSelect, Select};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::analyzer::{analyze_processes, AnalyzedProcess};
use crate::display::display_kill_results;
use crate::killer::kill_process;
use crate::maintenance::run_maintenance;
use crate::scanner::{get_cpu_usage, get_memory_info, get_system_load, scan_processes};
use crate::thermal::{get_thermal_status, PowerSource, ThermalStatus};
use crate::whitelist::Whitelist;

pub fn run_interactive() -> Result<()> {
    run_interactive_inner(false)
}

pub fn run_realtime() -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    println!("{}", "Starting realtime mode (Ctrl+C to stop, Enter for menu)...".dimmed());
    std::thread::sleep(Duration::from_secs(1));

    while running.load(Ordering::SeqCst) {
        // Clear screen
        print!("\x1B[2J\x1B[1;1H");
        std::io::stdout().flush()?;

        display_realtime_dashboard()?;

        // Check for Enter key (non-blocking)
        // For simplicity, we'll just auto-refresh every 2 seconds
        // User can Ctrl+C then run cooldown for menu

        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if !running.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    println!();
    println!("  {} Realtime mode stopped.", "INFO".dimmed());
    println!();

    // Return to interactive menu
    run_interactive_inner(false)
}

fn display_realtime_dashboard() -> Result<()> {
    let thermal = get_thermal_status().unwrap_or_default();
    let load = get_system_load();
    let mem = get_memory_info();
    let cpu = get_cpu_usage();
    let whitelist = Whitelist::load()?;

    // Header
    println!("{}", "COOLDOWN REALTIME".cyan().bold());
    println!("{}", "=================".dimmed());
    println!();

    // System stats line 1: Load, CPU, Memory
    let mem_pct = (mem.0 as f64 / mem.1 as f64) * 100.0;
    println!(
        "  {} Load {:.2} | CPU {:.0}% | Mem {}GB/{}GB ({:.0}%)",
        "System:".bold(),
        load.0,
        cpu,
        mem.0,
        mem.1,
        mem_pct
    );

    // System stats line 2: Thermal
    print!("  {} ", "Thermal:".bold());
    print!("{} ", thermal.level.emoji());
    print!("{}", thermal.level.colored_label());

    if let Some(temp) = thermal.cpu_temp {
        let temp_colored = if temp > 90.0 {
            format!("{:.0}°C", temp).red().bold()
        } else if temp > 75.0 {
            format!("{:.0}°C", temp).yellow()
        } else {
            format!("{:.0}°C", temp).green()
        };
        print!(" | CPU {}", temp_colored);
    }

    if let Some(rpm) = thermal.fan_rpm {
        print!(" | Fan {}rpm", rpm);
    }

    // Power source
    print!(
        " | {} {}",
        thermal.power_source.emoji(),
        if let Some(pct) = thermal.battery_percent {
            format!("{}%", pct)
        } else {
            thermal.power_source.label().to_string()
        }
    );
    println!();
    println!();

    // Scan processes
    let processes = scan_processes(20.0);
    let analyzed = analyze_processes(&processes, &whitelist);

    // Get top processes by CPU
    let mut all_sorted: Vec<&AnalyzedProcess> = analyzed.iter().collect();
    all_sorted.sort_by(|a, b| b.info.cpu_percent.partial_cmp(&a.info.cpu_percent).unwrap());

    // Problematic
    let problematic: Vec<&AnalyzedProcess> = analyzed
        .iter()
        .filter(|ap| ap.issue.is_problematic() && !ap.whitelisted && !ap.is_system_app)
        .collect();

    if !problematic.is_empty() {
        let total_cpu: f32 = problematic.iter().map(|ap| ap.info.cpu_percent).sum();
        let total_mem: u64 = problematic.iter().map(|ap| ap.info.memory_mb).sum();
        println!(
            "  {} {} process(es) - {:.0}% CPU, {}MB RAM",
            "⚠ PROBLEMATIC".yellow().bold(),
            problematic.len(),
            total_cpu,
            total_mem
        );
        println!();

        for ap in problematic.iter().take(5) {
            println!(
                "    {} [{:>6}] {:20} {:>5.0}% CPU {:>5}MB {}",
                "•".red(),
                ap.info.pid,
                truncate(&ap.info.name, 20),
                ap.info.cpu_percent,
                ap.info.memory_mb,
                ap.issue.description().red()
            );
        }
        if problematic.len() > 5 {
            println!("    {} ... and {} more", "•".dimmed(), problematic.len() - 5);
        }
        println!();
    }

    // Top processes table
    println!("  {}", "TOP PROCESSES".bold());
    println!(
        "    {:>6}  {:20} {:>7} {:>7} {:>8}  {}",
        "PID".dimmed(),
        "NAME".dimmed(),
        "CPU%".dimmed(),
        "MEM MB".dimmed(),
        "RUNTIME".dimmed(),
        "STATUS".dimmed()
    );
    println!("    {}", "─".repeat(70).dimmed());

    for ap in all_sorted.iter().take(10) {
        let status = if ap.is_system_app {
            "[SYS]".blue()
        } else if ap.whitelisted {
            "[WL]".cyan()
        } else if ap.issue.is_problematic() {
            format!("[{}]", ap.issue.label()).red()
        } else {
            "[OK]".green()
        };

        let cpu_colored = if ap.info.cpu_percent > 100.0 {
            format!("{:>6.0}%", ap.info.cpu_percent).red()
        } else if ap.info.cpu_percent > 50.0 {
            format!("{:>6.0}%", ap.info.cpu_percent).yellow()
        } else {
            format!("{:>6.0}%", ap.info.cpu_percent).normal()
        };

        let mem_colored = if ap.info.memory_mb > 1000 {
            format!("{:>6}", ap.info.memory_mb).yellow()
        } else {
            format!("{:>6}", ap.info.memory_mb).normal()
        };

        println!(
            "    {:>6}  {:20} {} {}  {:>8}  {}",
            ap.info.pid,
            truncate(&ap.info.name, 20),
            cpu_colored,
            mem_colored,
            ap.running_human(),
            status
        );
    }

    println!();
    println!(
        "  {} {} | {} Ctrl+C to stop",
        "Last update:".dimmed(),
        chrono::Local::now().format("%H:%M:%S"),
        "Press".dimmed()
    );

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        format!("{:width$}", s, width = max)
    } else {
        format!("{}…", &s[..max - 1])
    }
}

fn run_interactive_inner(_from_realtime: bool) -> Result<()> {
    // Get thermal status first
    let thermal = get_thermal_status().unwrap_or_default();

    // Adjust threshold based on power source
    let cpu_threshold = get_adaptive_threshold(&thermal);

    println!();
    println!("{}", "COOLDOWN".cyan().bold());
    println!("{}", "========".dimmed());

    // Show system status with thermal inline
    let load = get_system_load();
    let mem = get_memory_info();
    let cpu = get_cpu_usage();

    println!();
    println!(
        "  {} Load {:.2} | CPU {:.0}% | Mem {:.1}GB/{:.0}GB",
        "System:".bold(),
        load.0,
        cpu,
        mem.0,
        mem.1
    );

    // Thermal line with temperature
    print!("  {} {} {}", "Thermal:".bold(), thermal.level.emoji(), thermal.level.colored_label());

    if let Some(temp) = thermal.cpu_temp {
        let temp_colored = if temp > 90.0 {
            format!("{:.0}°C", temp).red().bold()
        } else if temp > 75.0 {
            format!("{:.0}°C", temp).yellow()
        } else {
            format!("{:.0}°C", temp).green()
        };
        print!(" | {}", temp_colored);
    }

    print!(
        " | {} {}",
        thermal.power_source.emoji(),
        if let Some(pct) = thermal.battery_percent {
            format!("{}%", pct)
        } else {
            thermal.power_source.label().to_string()
        }
    );
    println!();

    // Show adaptive threshold hint
    if thermal.power_source == PowerSource::Battery {
        println!(
            "  {} Battery mode - threshold {}%",
            "Mode:".dimmed(),
            cpu_threshold
        );
    }

    // Scan processes
    let whitelist = Whitelist::load()?;
    let processes = scan_processes(cpu_threshold as f32);
    let analyzed = analyze_processes(&processes, &whitelist);

    // Separate by category (excluding system apps from problematic)
    let problematic: Vec<&AnalyzedProcess> = analyzed
        .iter()
        .filter(|ap| ap.issue.is_problematic() && !ap.whitelisted && !ap.is_system_app)
        .collect();

    let system_warnings: Vec<&AnalyzedProcess> = analyzed
        .iter()
        .filter(|ap| ap.issue.is_problematic() && ap.is_system_app)
        .collect();

    let high_cpu: Vec<&AnalyzedProcess> = analyzed
        .iter()
        .filter(|ap| {
            ap.info.cpu_percent >= 50.0
                && !ap.issue.is_problematic()
                && !ap.is_system_app
                && !ap.whitelisted
        })
        .collect();

    println!();

    // No issues
    if problematic.is_empty() && high_cpu.is_empty() {
        println!(
            "  {} No problematic or high-CPU processes found.",
            "OK".green().bold()
        );

        // Show system warnings if any
        if !system_warnings.is_empty() {
            println!();
            println!(
                "  {} {} system app(s) using high CPU (protected):",
                "INFO".blue().bold(),
                system_warnings.len()
            );
            for ap in &system_warnings {
                println!(
                    "    • {} - {:.0}% CPU, {}MB {}",
                    ap.info.name.cyan(),
                    ap.info.cpu_percent,
                    ap.info.memory_mb,
                    "(system)".dimmed()
                );
            }
        }

        println!();
        return show_menu_no_issues(&thermal);
    }

    // Show problematic processes
    if !problematic.is_empty() {
        let total_cpu: f32 = problematic.iter().map(|ap| ap.info.cpu_percent).sum();
        let total_mem: u64 = problematic.iter().map(|ap| ap.info.memory_mb).sum();
        println!(
            "  {} {} problematic ({:.0}% CPU, {}MB RAM):",
            "FOUND".yellow().bold(),
            problematic.len(),
            total_cpu,
            total_mem
        );
        println!();
        for (i, ap) in problematic.iter().enumerate() {
            println!(
                "    {}. [{}] {} - {:.0}% CPU, {}MB - {}",
                i + 1,
                format!("{}", ap.info.pid).dimmed(),
                ap.info.name.cyan(),
                ap.info.cpu_percent,
                ap.info.memory_mb,
                ap.issue.description().red()
            );
        }
        println!();
    }

    // Show high CPU processes
    if !high_cpu.is_empty() {
        let total_cpu: f32 = high_cpu.iter().map(|ap| ap.info.cpu_percent).sum();
        let total_mem: u64 = high_cpu.iter().map(|ap| ap.info.memory_mb).sum();
        println!(
            "  {} {} high-CPU ({:.0}% CPU, {}MB RAM):",
            "INFO".blue().bold(),
            high_cpu.len(),
            total_cpu,
            total_mem
        );
        println!();
        for (i, ap) in high_cpu.iter().enumerate() {
            println!(
                "    {}. [{}] {} - {:.0}% CPU, {}MB",
                i + 1,
                format!("{}", ap.info.pid).dimmed(),
                ap.info.name.cyan(),
                ap.info.cpu_percent,
                ap.info.memory_mb
            );
        }
        println!();
    }

    // Show system warnings
    if !system_warnings.is_empty() {
        println!(
            "  {} {} system app(s) high CPU (protected):",
            "NOTE".dimmed(),
            system_warnings.len()
        );
        for ap in &system_warnings {
            println!(
                "    • {} - {:.0}% CPU, {}MB",
                ap.info.name.dimmed(),
                ap.info.cpu_percent,
                ap.info.memory_mb
            );
        }
        println!();
    }

    // Show menu
    show_menu(&problematic, &high_cpu, &thermal)
}

fn get_adaptive_threshold(thermal: &ThermalStatus) -> u32 {
    // Lower threshold when on battery to be more aggressive about saving power
    let base: u32 = match thermal.power_source {
        PowerSource::Battery => 30,
        PowerSource::AC => 50,
        PowerSource::Unknown => 40,
    };

    // Further lower if thermal is high
    if thermal.level.is_dangerous() {
        base.saturating_sub(10)
    } else {
        base
    }
}

fn show_menu(
    problematic: &[&AnalyzedProcess],
    high_cpu: &[&AnalyzedProcess],
    _thermal: &ThermalStatus,
) -> Result<()> {
    let mut options = vec![];

    if !problematic.is_empty() {
        let total_cpu: f32 = problematic.iter().map(|ap| ap.info.cpu_percent).sum();
        let total_mem: u64 = problematic.iter().map(|ap| ap.info.memory_mb).sum();
        options.push(format!(
            "🔴 Kill all problematic ({} proc, {:.0}% CPU, {}MB)",
            problematic.len(),
            total_cpu,
            total_mem
        ));
        options.push("🎯 Select processes to kill".to_string());
    }

    if !high_cpu.is_empty() {
        options.push("📋 Select high-CPU processes to kill".to_string());
    }

    // Combine all for throttle
    let all_killable: Vec<&AnalyzedProcess> = problematic
        .iter()
        .chain(high_cpu.iter())
        .copied()
        .collect();

    if !all_killable.is_empty() {
        options.push("⏱️  Quick throttle (limit CPU)".to_string());
    }

    options.push("📊 Realtime monitor".to_string());
    options.push("🔧 Run maintenance".to_string());
    options.push("🔄 Refresh".to_string());
    options.push("❌ Exit".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What would you like to do?")
        .items(&options)
        .default(0)
        .interact()?;

    let selected = &options[selection];

    if selected.contains("Kill all problematic") {
        kill_all_problematic(problematic)
    } else if selected.contains("Select processes to kill") {
        select_and_kill(problematic)
    } else if selected.contains("Select high-CPU") {
        select_and_kill(high_cpu)
    } else if selected.contains("Quick throttle") {
        quick_throttle(&all_killable)
    } else if selected.contains("Realtime") {
        run_realtime()
    } else if selected.contains("Run maintenance") {
        run_maintenance_interactive()
    } else if selected.contains("Refresh") {
        run_interactive()
    } else {
        println!();
        Ok(())
    }
}

fn show_menu_no_issues(_thermal: &ThermalStatus) -> Result<()> {
    let options = vec![
        "📊 Realtime monitor",
        "🔧 Run maintenance",
        "🔄 Refresh",
        "❌ Exit",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What would you like to do?")
        .items(&options)
        .default(0)
        .interact()?;

    match selection {
        0 => run_realtime(),
        1 => run_maintenance_interactive(),
        2 => run_interactive(),
        _ => {
            println!();
            Ok(())
        }
    }
}

fn kill_all_problematic(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        return run_interactive();
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
        return run_interactive();
    }

    let mut results = vec![];

    for ap in processes {
        results.push(kill_process(ap.info.pid, &ap.info.name, false));
    }

    std::thread::sleep(std::time::Duration::from_millis(500));
    let (new_load, _, _) = get_system_load();

    display_kill_results(&results, total_cpu, new_load);
    println!();

    // Return to menu
    run_interactive()
}

fn select_and_kill(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        return run_interactive();
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
        .with_prompt("Select processes (Space to select, Enter to confirm)")
        .items(&items)
        .interact()?;

    if selections.is_empty() {
        println!("  {} No processes selected.", "INFO".dimmed());
        println!();
        return run_interactive();
    }

    let total_cpu: f32 = selections
        .iter()
        .map(|&idx| processes[idx].info.cpu_percent)
        .sum();

    let mut results = vec![];

    for idx in selections {
        let ap = processes[idx];
        results.push(kill_process(ap.info.pid, &ap.info.name, false));
    }

    std::thread::sleep(std::time::Duration::from_millis(500));
    let (new_load, _, _) = get_system_load();

    display_kill_results(&results, total_cpu, new_load);
    println!();

    run_interactive()
}

fn quick_throttle(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        println!("  {} No processes to throttle.", "INFO".dimmed());
        return run_interactive();
    }

    // Build items with CPU limit options inline
    let items: Vec<String> = processes
        .iter()
        .map(|ap| {
            format!(
                "[{}] {} ({:.0}% → 50% CPU, {}MB)",
                ap.info.pid, ap.info.name, ap.info.cpu_percent, ap.info.memory_mb
            )
        })
        .collect();

    let mut items_with_cancel = items.clone();
    items_with_cancel.push("← Back".to_string());

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select process to throttle to 50% CPU")
        .items(&items_with_cancel)
        .interact()?;

    if selection == items.len() {
        return run_interactive();
    }

    let ap = processes[selection];

    println!();
    println!(
        "  {} Throttling {} to 50% CPU (Ctrl+C to stop)",
        "THROTTLE".cyan().bold(),
        ap.info.name.cyan()
    );
    println!();

    crate::throttle::run_throttle(ap.info.pid, 50, None)?;

    run_interactive()
}

fn run_maintenance_interactive() -> Result<()> {
    let options = vec![
        "🌐 Flush DNS cache",
        "💾 Free purgeable space",
        "⏰ Clear Time Machine snapshots",
        "🚀 Run all",
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

    run_interactive()
}
