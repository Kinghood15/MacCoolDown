use anyhow::Result;
use colored::*;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    terminal,
    ExecutableCommand,
};
use dialoguer::{theme::ColorfulTheme, Confirm, MultiSelect, Select};
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

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
    // Enable raw mode for keyboard input
    terminal::enable_raw_mode()?;

    let result = run_realtime_inner();

    // Always restore terminal
    terminal::disable_raw_mode()?;
    stdout().execute(crossterm::cursor::Show)?;

    result
}

fn run_realtime_inner() -> Result<()> {
    let mut paused = false;
    let mut last_refresh = Instant::now();
    let refresh_interval = Duration::from_secs(5); // 5 seconds to reduce eye strain
    let mut selected_idx: usize = 0;
    let mut last_processes: Vec<ProcessDisplay> = Vec::new();

    loop {
        // Check for keyboard input (non-blocking)
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match handle_key(key, &mut paused, &mut selected_idx, &last_processes)? {
                    KeyAction::Quit => break,
                    KeyAction::Menu => {
                        terminal::disable_raw_mode()?;
                        stdout().execute(crossterm::cursor::Show)?;
                        print!("\x1B[2J\x1B[1;1H");
                        stdout().flush()?;
                        run_interactive_inner()?;
                        terminal::enable_raw_mode()?;
                        last_refresh = Instant::now() - refresh_interval; // Force refresh
                    }
                    KeyAction::KillSelected => {
                        if let Some(proc) = last_processes.get(selected_idx) {
                            terminal::disable_raw_mode()?;
                            kill_process_quick(proc.pid, &proc.name)?;
                            terminal::enable_raw_mode()?;
                            last_refresh = Instant::now() - refresh_interval;
                        }
                    }
                    KeyAction::KillAllProblematic => {
                        let problematic: Vec<_> = last_processes
                            .iter()
                            .filter(|p| p.is_problematic)
                            .collect();
                        if !problematic.is_empty() {
                            terminal::disable_raw_mode()?;
                            kill_problematic_quick(&problematic)?;
                            terminal::enable_raw_mode()?;
                            last_refresh = Instant::now() - refresh_interval;
                        }
                    }
                    KeyAction::Continue => {}
                }
            }
        }

        // Refresh display
        if !paused && last_refresh.elapsed() >= refresh_interval {
            last_processes = display_realtime_dashboard(selected_idx)?;
            last_refresh = Instant::now();
        } else if paused {
            // Show paused indicator
            display_realtime_dashboard_paused(selected_idx, &last_processes)?;
        }
    }

    // Clear and show goodbye
    print!("\x1B[2J\x1B[1;1H");
    stdout().flush()?;
    println!("  {} Goodbye!", "cooldown".cyan().bold());
    println!();

    Ok(())
}

#[derive(Debug, Clone)]
struct ProcessDisplay {
    pid: u32,
    name: String,
    cpu: f32,
    mem: u64,
    runtime: String,
    status: String,
    is_problematic: bool,
    is_system: bool,
}

enum KeyAction {
    Quit,
    Menu,
    KillSelected,
    KillAllProblematic,
    Continue,
}

fn handle_key(
    key: KeyEvent,
    paused: &mut bool,
    selected_idx: &mut usize,
    processes: &[ProcessDisplay],
) -> Result<KeyAction> {
    match key.code {
        // Quit
        KeyCode::Char('q') | KeyCode::Esc => Ok(KeyAction::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Ok(KeyAction::Quit),

        // Menu
        KeyCode::Char('m') | KeyCode::Enter => Ok(KeyAction::Menu),

        // Pause/Resume
        KeyCode::Char(' ') | KeyCode::Char('p') => {
            *paused = !*paused;
            Ok(KeyAction::Continue)
        }

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => {
            if *selected_idx > 0 {
                *selected_idx -= 1;
            }
            Ok(KeyAction::Continue)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if *selected_idx < processes.len().saturating_sub(1) {
                *selected_idx += 1;
            }
            Ok(KeyAction::Continue)
        }

        // Kill selected
        KeyCode::Char('x') | KeyCode::Delete => Ok(KeyAction::KillSelected),

        // Kill all problematic
        KeyCode::Char('K') => Ok(KeyAction::KillAllProblematic),

        _ => Ok(KeyAction::Continue),
    }
}

fn display_realtime_dashboard(selected_idx: usize) -> Result<Vec<ProcessDisplay>> {
    let thermal = get_thermal_status().unwrap_or_default();
    let load = get_system_load();
    let mem = get_memory_info();
    let cpu = get_cpu_usage();
    let whitelist = Whitelist::load()?;

    // Clear screen
    print!("\x1B[2J\x1B[1;1H");
    stdout().execute(crossterm::cursor::Hide)?;

    // Header
    println!("{}", " COOLDOWN ".on_cyan().black().bold());
    println!();

    // System stats
    let mem_pct = (mem.0 as f64 / mem.1 as f64) * 100.0;
    print!("  ");
    print!("{}", "Load ".dimmed());
    print_colored_load(load.0);
    print!("{}", " │ ".dimmed());
    print!("{}", "CPU ".dimmed());
    print_colored_percent(cpu);
    print!("{}", " │ ".dimmed());
    print!("{}", "Mem ".dimmed());
    print!("{}GB/{}GB ", mem.0, mem.1);
    print_colored_percent(mem_pct as f32);
    println!();

    // Thermal line
    print!("  ");
    print!("{} ", thermal.level.emoji());
    print!("{}", thermal.level.colored_label());
    if let Some(temp) = thermal.cpu_temp {
        print!("{}", " │ ".dimmed());
        print_colored_temp(temp);
    }
    if let Some(rpm) = thermal.fan_rpm {
        print!("{}", " │ ".dimmed());
        print!("{}rpm", rpm);
    }
    print!("{}", " │ ".dimmed());
    print!("{} ", thermal.power_source.emoji());
    if let Some(pct) = thermal.battery_percent {
        print!("{}%", pct);
    } else {
        print!("{}", thermal.power_source.label());
    }
    println!();
    println!();

    // Scan processes
    let processes = scan_processes(20.0);
    let analyzed = analyze_processes(&processes, &whitelist);

    // Build display list
    let mut display_list: Vec<ProcessDisplay> = Vec::new();

    // Sort by CPU
    let mut sorted: Vec<&AnalyzedProcess> = analyzed.iter().collect();
    sorted.sort_by(|a, b| b.info.cpu_percent.partial_cmp(&a.info.cpu_percent).unwrap());

    for ap in sorted.iter().take(12) {
        let status = if ap.is_system_app {
            "SYS".blue().to_string()
        } else if ap.whitelisted {
            "WL".cyan().to_string()
        } else if ap.issue.is_problematic() {
            ap.issue.label().red().to_string()
        } else {
            "OK".green().to_string()
        };

        display_list.push(ProcessDisplay {
            pid: ap.info.pid,
            name: ap.info.name.clone(),
            cpu: ap.info.cpu_percent,
            mem: ap.info.memory_mb,
            runtime: ap.running_human(),
            status,
            is_problematic: ap.issue.is_problematic() && !ap.whitelisted && !ap.is_system_app,
            is_system: ap.is_system_app,
        });
    }

    // Count problematic
    let problematic_count = display_list.iter().filter(|p| p.is_problematic).count();
    let problematic_cpu: f32 = display_list
        .iter()
        .filter(|p| p.is_problematic)
        .map(|p| p.cpu)
        .sum();

    // Table header
    if problematic_count > 0 {
        println!(
            "  {} {} problematic ({:.0}% CPU)",
            "⚠".yellow(),
            problematic_count,
            problematic_cpu
        );
        println!();
    }

    println!(
        "  {}  {:>6}  {:20}  {:>6}  {:>6}  {:>8}  {}",
        " ".dimmed(),
        "PID".dimmed(),
        "NAME".dimmed(),
        "CPU".dimmed(),
        "MEM".dimmed(),
        "TIME".dimmed(),
        "STATUS".dimmed()
    );
    println!("  {}", "─".repeat(68).dimmed());

    for (i, proc) in display_list.iter().enumerate() {
        let selector = if i == selected_idx {
            "▶".cyan().bold()
        } else {
            " ".normal()
        };

        let name_display = truncate(&proc.name, 20);
        let name_colored = if proc.is_problematic {
            name_display.red()
        } else if proc.is_system {
            name_display.blue()
        } else {
            name_display.normal()
        };

        print!("  {}  {:>6}  ", selector, proc.pid);
        print!("{:20}  ", name_colored);
        print_colored_cpu(proc.cpu);
        print!("  ");
        print_colored_mem(proc.mem);
        println!("  {:>8}  [{}]", proc.runtime, proc.status);
    }

    println!();
    println!("  {}", "─".repeat(68).dimmed());

    // Footer with shortcuts
    println!();
    print!("  ");
    print!("{}", "↑↓".cyan());
    print!(" select  ");
    print!("{}", "x".red());
    print!(" kill  ");
    print!("{}", "K".red());
    print!(" kill all  ");
    print!("{}", "m".cyan());
    print!(" menu  ");
    print!("{}", "p".yellow());
    print!(" pause  ");
    print!("{}", "q".dimmed());
    print!(" quit");
    println!();

    // Timestamp
    println!(
        "  {} {}",
        "Updated:".dimmed(),
        chrono::Local::now().format("%H:%M:%S")
    );

    stdout().flush()?;
    Ok(display_list)
}

fn display_realtime_dashboard_paused(
    _selected_idx: usize,
    _processes: &[ProcessDisplay],
) -> Result<()> {
    // Just update the pause indicator without full refresh
    // Move cursor to status line
    print!("\x1B[{};1H", 20); // Approximate line
    print!(
        "  {} {} {}",
        "PAUSED".yellow().bold(),
        "-".dimmed(),
        "Press SPACE to resume".dimmed()
    );
    print!("                    "); // Clear rest of line
    stdout().flush()?;
    Ok(())
}

fn kill_process_quick(pid: u32, name: &str) -> Result<()> {
    println!();
    println!("  {} Kill {} (PID {})? [y/N] ", "?".yellow(), name.cyan(), pid);

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    if input.trim().eq_ignore_ascii_case("y") {
        let result = kill_process(pid, name, false);
        if result.success {
            println!("  {} Killed {}", "OK".green().bold(), name);
        } else {
            println!(
                "  {} Failed: {}",
                "ERR".red().bold(),
                result.error.unwrap_or_default()
            );
        }
    } else {
        println!("  {} Cancelled", "INFO".dimmed());
    }

    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}

fn kill_problematic_quick(processes: &[&ProcessDisplay]) -> Result<()> {
    let total_cpu: f32 = processes.iter().map(|p| p.cpu).sum();

    println!();
    println!(
        "  {} Kill {} problematic process(es)? ({:.0}% CPU) [y/N] ",
        "?".yellow(),
        processes.len(),
        total_cpu
    );

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    if input.trim().eq_ignore_ascii_case("y") {
        for proc in processes {
            let result = kill_process(proc.pid, &proc.name, false);
            if result.success {
                println!("  {} Killed {}", "OK".green().bold(), proc.name);
            } else {
                println!(
                    "  {} Failed {}: {}",
                    "ERR".red().bold(),
                    proc.name,
                    result.error.unwrap_or_default()
                );
            }
        }
    } else {
        println!("  {} Cancelled", "INFO".dimmed());
    }

    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}

// Helper functions for colored output
fn print_colored_load(load: f64) {
    let num_cpus = num_cpus::get() as f64;
    let ratio = load / num_cpus;
    if ratio > 1.0 {
        print!("{}", format!("{:.2}", load).red());
    } else if ratio > 0.7 {
        print!("{}", format!("{:.2}", load).yellow());
    } else {
        print!("{}", format!("{:.2}", load).green());
    }
}

fn print_colored_percent(pct: f32) {
    if pct > 90.0 {
        print!("{}", format!("{:.0}%", pct).red());
    } else if pct > 70.0 {
        print!("{}", format!("{:.0}%", pct).yellow());
    } else {
        print!("{}", format!("{:.0}%", pct).green());
    }
}

fn print_colored_temp(temp: f64) {
    if temp > 90.0 {
        print!("{}", format!("{:.0}°C", temp).red().bold());
    } else if temp > 75.0 {
        print!("{}", format!("{:.0}°C", temp).yellow());
    } else {
        print!("{}", format!("{:.0}°C", temp).green());
    }
}

fn print_colored_cpu(cpu: f32) {
    if cpu > 100.0 {
        print!("{}", format!("{:>5.0}%", cpu).red());
    } else if cpu > 50.0 {
        print!("{}", format!("{:>5.0}%", cpu).yellow());
    } else {
        print!("{}", format!("{:>5.0}%", cpu).normal());
    }
}

fn print_colored_mem(mem: u64) {
    if mem > 2000 {
        print!("{}", format!("{:>5}M", mem).red());
    } else if mem > 500 {
        print!("{}", format!("{:>5}M", mem).yellow());
    } else {
        print!("{}", format!("{:>5}M", mem).normal());
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        format!("{:width$}", s, width = max)
    } else {
        format!("{}…", &s[..max - 1])
    }
}

// ============================================================================
// Interactive Menu Mode
// ============================================================================

fn run_interactive_inner() -> Result<()> {
    let thermal = get_thermal_status().unwrap_or_default();
    let cpu_threshold = get_adaptive_threshold(&thermal);

    println!();
    println!("{}", "COOLDOWN MENU".cyan().bold());
    println!("{}", "=============".dimmed());

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

    let whitelist = Whitelist::load()?;
    let processes = scan_processes(cpu_threshold as f32);
    let analyzed = analyze_processes(&processes, &whitelist);

    let problematic: Vec<&AnalyzedProcess> = analyzed
        .iter()
        .filter(|ap| ap.issue.is_problematic() && !ap.whitelisted && !ap.is_system_app)
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

    if problematic.is_empty() && high_cpu.is_empty() {
        println!("  {} No problematic processes found.", "OK".green().bold());
        println!();
        return show_menu_no_issues();
    }

    if !problematic.is_empty() {
        let total_cpu: f32 = problematic.iter().map(|ap| ap.info.cpu_percent).sum();
        let total_mem: u64 = problematic.iter().map(|ap| ap.info.memory_mb).sum();
        println!(
            "  {} {} problematic ({:.0}% CPU, {}MB):",
            "FOUND".yellow().bold(),
            problematic.len(),
            total_cpu,
            total_mem
        );
        for ap in &problematic {
            println!(
                "    • {} - {:.0}% CPU, {}MB",
                ap.info.name.red(),
                ap.info.cpu_percent,
                ap.info.memory_mb
            );
        }
        println!();
    }

    if !high_cpu.is_empty() {
        println!(
            "  {} {} high-CPU processes:",
            "INFO".blue().bold(),
            high_cpu.len()
        );
        for ap in &high_cpu {
            println!(
                "    • {} - {:.0}% CPU, {}MB",
                ap.info.name.cyan(),
                ap.info.cpu_percent,
                ap.info.memory_mb
            );
        }
        println!();
    }

    show_menu(&problematic, &high_cpu)
}

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

fn show_menu(problematic: &[&AnalyzedProcess], high_cpu: &[&AnalyzedProcess]) -> Result<()> {
    let mut options = vec![];

    if !problematic.is_empty() {
        let total_cpu: f32 = problematic.iter().map(|ap| ap.info.cpu_percent).sum();
        options.push(format!(
            "Kill all problematic ({} proc, {:.0}% CPU)",
            problematic.len(),
            total_cpu
        ));
        options.push("Select processes to kill".to_string());
    }

    if !high_cpu.is_empty() {
        options.push("Select high-CPU to kill".to_string());
    }

    let all: Vec<&AnalyzedProcess> = problematic.iter().chain(high_cpu.iter()).copied().collect();
    if !all.is_empty() {
        options.push("Throttle a process".to_string());
    }

    options.push("Run maintenance".to_string());
    options.push("Back to realtime".to_string());
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
    } else if selected.contains("Select processes") {
        select_and_kill(problematic)?;
        run_interactive_inner()
    } else if selected.contains("Select high-CPU") {
        select_and_kill(high_cpu)?;
        run_interactive_inner()
    } else if selected.contains("Throttle") {
        throttle_interactive(&all)?;
        run_interactive_inner()
    } else if selected.contains("maintenance") {
        run_maintenance_menu()?;
        run_interactive_inner()
    } else if selected.contains("realtime") {
        run_realtime()
    } else {
        println!();
        Ok(())
    }
}

fn show_menu_no_issues() -> Result<()> {
    let options = vec!["Run maintenance", "Back to realtime", "Exit"];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Action")
        .items(&options)
        .default(0)
        .interact()?;

    match selection {
        0 => {
            run_maintenance_menu()?;
            run_interactive_inner()
        }
        1 => run_realtime(),
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

    let confirm = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Kill {} process(es)? ({:.0}% CPU)", processes.len(), total_cpu))
        .default(false)
        .interact()?;

    if !confirm {
        println!("  {} Cancelled.", "INFO".dimmed());
        return Ok(());
    }

    for ap in processes {
        let result = kill_process(ap.info.pid, &ap.info.name, false);
        if result.success {
            println!("  {} Killed {}", "OK".green().bold(), ap.info.name);
        }
    }

    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}

fn select_and_kill(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        return Ok(());
    }

    let items: Vec<String> = processes
        .iter()
        .map(|ap| format!("[{}] {} - {:.0}%", ap.info.pid, ap.info.name, ap.info.cpu_percent))
        .collect();

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select (Space to toggle)")
        .items(&items)
        .interact()?;

    for idx in selections {
        let ap = processes[idx];
        let result = kill_process(ap.info.pid, &ap.info.name, false);
        if result.success {
            println!("  {} Killed {}", "OK".green().bold(), ap.info.name);
        }
    }

    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}

fn throttle_interactive(processes: &[&AnalyzedProcess]) -> Result<()> {
    if processes.is_empty() {
        return Ok(());
    }

    let items: Vec<String> = processes
        .iter()
        .map(|ap| format!("[{}] {} - {:.0}%", ap.info.pid, ap.info.name, ap.info.cpu_percent))
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Throttle to 50% CPU")
        .items(&items)
        .interact()?;

    let ap = processes[selection];
    println!();
    println!(
        "  {} Throttling {} (Ctrl+C to stop)",
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
        "Clear Time Machine",
        "Run all",
        "Back",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Maintenance")
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
