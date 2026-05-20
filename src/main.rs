use clap::{Parser, Subcommand};
use colored::*;

mod analyzer;
mod display;
mod killer;
mod scanner;
mod watcher;
mod whitelist;

use analyzer::{analyze_processes, find_old_processes, parse_duration, ProcessIssue};
use display::{
    display_analyzed, display_kill_plan, display_kill_results, display_summary,
    display_system_status, display_whitelist,
};
use killer::kill_process;
use scanner::{get_cpu_usage, get_memory_info, get_system_load, scan_all_processes, scan_processes};
use watcher::{run_watch, WatchConfig};
use whitelist::Whitelist;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "cooldown")]
#[command(about = "Keep your MacBook cool by managing CPU-heavy processes")]
#[command(version = "0.1.0")]
#[command(long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan and display high CPU processes
    Scan {
        /// CPU% threshold to report (default 50)
        #[arg(short, long, default_value = "50")]
        threshold: u32,

        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },

    /// Clean stuck / orphan / old processes (interactive)
    Clean {
        /// Show what would be done without actually killing
        #[arg(short, long)]
        dry_run: bool,

        /// Use SIGKILL instead of SIGTERM
        #[arg(short, long)]
        force: bool,
    },

    /// Kill specific process types
    Kill {
        /// Kill orphan processes (parent = init)
        #[arg(long)]
        orphans: bool,

        /// Kill stuck processes (very high CPU > 1 hour)
        #[arg(long)]
        stuck: bool,

        /// Kill processes older than duration, e.g. 3d, 12h, 30m
        #[arg(long)]
        old: Option<String>,

        /// Kill a specific PID
        #[arg(long)]
        pid: Option<u32>,

        /// Use SIGKILL instead of SIGTERM
        #[arg(short, long)]
        force: bool,

        /// Show what would be done without actually killing
        #[arg(short, long)]
        dry_run: bool,
    },

    /// Watch mode - continuous monitoring at interval
    Watch {
        /// CPU% threshold to report (default 80)
        #[arg(short, long, default_value = "80")]
        threshold: u32,

        /// Polling interval in seconds (default 30)
        #[arg(short, long, default_value = "30")]
        interval: u64,

        /// Automatically kill problematic processes
        #[arg(long)]
        auto_clean: bool,
    },

    /// Manage the process whitelist
    Whitelist {
        #[command(subcommand)]
        action: WhitelistAction,
    },

    /// Show system overview
    Status,
}

#[derive(Subcommand)]
enum WhitelistAction {
    /// Add a pattern to the whitelist
    Add {
        /// Pattern string (substring match, supports * wildcard)
        pattern: String,
    },
    /// Remove a pattern from the whitelist
    Remove {
        /// Pattern to remove
        pattern: String,
    },
    /// List all whitelisted patterns
    List,
    /// Remove all whitelist patterns
    Clear,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Scan { threshold, json } => cmd_scan(threshold, json),
        Commands::Clean { dry_run, force } => cmd_clean(dry_run, force),
        Commands::Kill {
            orphans,
            stuck,
            old,
            pid,
            force,
            dry_run,
        } => cmd_kill(orphans, stuck, old, pid, force, dry_run),
        Commands::Watch {
            threshold,
            interval,
            auto_clean,
        } => cmd_watch(threshold, interval, auto_clean),
        Commands::Whitelist { action } => cmd_whitelist(action),
        Commands::Status => cmd_status(),
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn cmd_scan(threshold: u32, json_output: bool) -> anyhow::Result<()> {
    let whitelist = Whitelist::load()?;
    let processes = scan_processes(threshold as f32);
    let analyzed = analyze_processes(&processes, &whitelist);

    if json_output {
        let output: Vec<serde_json::Value> = analyzed
            .iter()
            .map(|ap| {
                serde_json::json!({
                    "pid": ap.info.pid,
                    "name": ap.info.name,
                    "cmd": ap.info.cmd,
                    "cpu_percent": ap.info.cpu_percent,
                    "memory_mb": ap.info.memory_mb,
                    "parent_pid": ap.info.parent_pid,
                    "start_time": ap.info.start_time,
                    "running_secs": ap.running_secs,
                    "issue": ap.issue.label(),
                    "whitelisted": ap.whitelisted,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    display_analyzed(&analyzed, threshold);

    let load = get_system_load();
    let mem = get_memory_info();
    let cpu = get_cpu_usage();
    display_system_status(load, mem, cpu);

    display_summary(&analyzed);
    println!();

    Ok(())
}

fn cmd_clean(dry_run: bool, force: bool) -> anyhow::Result<()> {
    let whitelist = Whitelist::load()?;
    let processes = scan_processes(20.0); // lower threshold for clean
    let analyzed = analyze_processes(&processes, &whitelist);

    let to_kill: Vec<(u32, String, String)> = analyzed
        .iter()
        .filter(|ap| ap.issue.is_problematic() && !ap.whitelisted)
        .map(|ap| {
            let reason = ap.issue.description();
            (ap.info.pid, ap.info.name.clone(), reason)
        })
        .collect();

    let skipped: Vec<(u32, String, String)> = analyzed
        .iter()
        .filter(|ap| ap.whitelisted && ap.issue.is_problematic())
        .map(|ap| {
            let pattern = whitelist
                .patterns
                .iter()
                .find(|pat| {
                    ap.info.name.to_lowercase().contains(&pat.to_lowercase())
                        || ap.info.cmd.to_lowercase().contains(&pat.to_lowercase())
                })
                .cloned()
                .unwrap_or_else(|| "whitelist".to_string());
            (ap.info.pid, ap.info.name.clone(), pattern)
        })
        .collect();

    display_kill_plan(&to_kill, &skipped, dry_run);

    if dry_run || to_kill.is_empty() {
        return Ok(());
    }

    // Capture total CPU before killing
    let freed_cpu: f32 = to_kill
        .iter()
        .filter_map(|(pid, _, _)| {
            analyzed
                .iter()
                .find(|ap| ap.info.pid == *pid)
                .map(|ap| ap.info.cpu_percent)
        })
        .sum();

    let results: Vec<killer::KillResult> = to_kill
        .iter()
        .map(|(pid, name, _)| kill_process(*pid, name, force))
        .collect();

    // Get new load after kills
    std::thread::sleep(std::time::Duration::from_millis(500));
    let (new_load, _, _) = get_system_load();

    display_kill_results(&results, freed_cpu, new_load);
    println!();

    Ok(())
}

fn cmd_kill(
    orphans: bool,
    stuck: bool,
    old: Option<String>,
    pid: Option<u32>,
    force: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    let whitelist = Whitelist::load()?;

    // If specific PID
    if let Some(target_pid) = pid {
        let all = scan_all_processes();
        let name = all
            .iter()
            .find(|p| p.pid == target_pid)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("PID{}", target_pid));

        if dry_run {
            println!();
            println!(
                "  {} Would kill PID {} ({})",
                "DRY RUN:".yellow().bold(),
                target_pid,
                name.cyan()
            );
            return Ok(());
        }

        let result = kill_process(target_pid, &name, force);
        if result.success {
            println!("  {} Killed PID {} ({})", "OK".green().bold(), target_pid, name.cyan());
        } else {
            println!(
                "  {} Failed: {}",
                "ERR".red().bold(),
                result.error.as_deref().unwrap_or("unknown").red()
            );
        }
        return Ok(());
    }

    if !orphans && !stuck && old.is_none() {
        println!(
            "  {}",
            "Specify at least one of: --orphans, --stuck, --old <duration>, --pid <PID>".yellow()
        );
        return Ok(());
    }

    let processes = scan_processes(10.0);
    let analyzed = analyze_processes(&processes, &whitelist);

    let mut targets: Vec<(u32, String, String)> = Vec::new();
    let mut skipped: Vec<(u32, String, String)> = Vec::new();

    for ap in &analyzed {
        let matches = (orphans && ap.issue == ProcessIssue::Orphan)
            || (stuck && ap.issue == ProcessIssue::Stuck)
            || (old.is_some() && matches!(&ap.issue, ProcessIssue::Old(_)));

        if !matches {
            continue;
        }

        if ap.whitelisted {
            let pattern = whitelist
                .patterns
                .iter()
                .find(|pat| {
                    ap.info.name.to_lowercase().contains(&pat.to_lowercase())
                        || ap.info.cmd.to_lowercase().contains(&pat.to_lowercase())
                })
                .cloned()
                .unwrap_or_else(|| "whitelist".to_string());
            skipped.push((ap.info.pid, ap.info.name.clone(), pattern));
        } else {
            targets.push((ap.info.pid, ap.info.name.clone(), ap.issue.description()));
        }
    }

    // Handle --old with custom duration threshold
    if let Some(ref duration_str) = old {
        let min_age = parse_duration(duration_str)?;
        let old_procs = find_old_processes(&processes, min_age);
        for p in old_procs {
            if whitelist.matches(p) {
                continue;
            }
            if !targets.iter().any(|(pid, _, _)| *pid == p.pid) {
                targets.push((
                    p.pid,
                    p.name.clone(),
                    format!("Running > {}", duration_str),
                ));
            }
        }
    }

    display_kill_plan(&targets, &skipped, dry_run);

    if dry_run || targets.is_empty() {
        return Ok(());
    }

    let freed_cpu: f32 = targets
        .iter()
        .filter_map(|(pid, _, _)| {
            analyzed
                .iter()
                .find(|ap| ap.info.pid == *pid)
                .map(|ap| ap.info.cpu_percent)
        })
        .sum();

    let results: Vec<killer::KillResult> = targets
        .iter()
        .map(|(pid, name, _)| kill_process(*pid, name, force))
        .collect();

    std::thread::sleep(std::time::Duration::from_millis(500));
    let (new_load, _, _) = get_system_load();

    display_kill_results(&results, freed_cpu, new_load);
    println!();

    Ok(())
}

fn cmd_watch(threshold: u32, interval: u64, auto_clean: bool) -> anyhow::Result<()> {
    run_watch(WatchConfig {
        threshold,
        interval_secs: interval,
        auto_clean,
    })
}

fn cmd_whitelist(action: WhitelistAction) -> anyhow::Result<()> {
    match action {
        WhitelistAction::Add { pattern } => {
            let mut wl = Whitelist::load()?;
            wl.add(pattern.clone());
            wl.save()?;
            println!("  {} Added pattern: {}", "OK".green().bold(), pattern.cyan());
        }
        WhitelistAction::Remove { pattern } => {
            let mut wl = Whitelist::load()?;
            let removed = wl.remove(&pattern);
            wl.save()?;
            if removed {
                println!(
                    "  {} Removed pattern: {}",
                    "OK".green().bold(),
                    pattern.cyan()
                );
            } else {
                println!(
                    "  {} Pattern not found: {}",
                    "WARN".yellow().bold(),
                    pattern.yellow()
                );
            }
        }
        WhitelistAction::List => {
            let wl = Whitelist::load()?;
            display_whitelist(&wl.patterns);
        }
        WhitelistAction::Clear => {
            let mut wl = Whitelist::load()?;
            let count = wl.patterns.len();
            wl.clear();
            wl.save()?;
            println!(
                "  {} Cleared {} pattern(s).",
                "OK".green().bold(),
                count
            );
        }
    }
    Ok(())
}

fn cmd_status() -> anyhow::Result<()> {
    println!();
    println!("{}", "SYSTEM STATUS".cyan().bold());
    println!("{}", "=============".dimmed());

    let load = get_system_load();
    let mem = get_memory_info();
    let cpu = get_cpu_usage();

    display_system_status(load, mem, cpu);

    println!();
    println!("  {} {:.2} | {:.2} | {:.2}", "Load avg (1/5/15m):".bold(), load.0, load.1, load.2);
    println!("  {} {:.0}%", "CPU usage:".bold(), cpu);
    println!("  {} {}GB / {}GB", "Memory:".bold(), mem.0, mem.1);

    let all_procs = scan_processes(0.0);
    println!("  {} {}", "Total processes:".bold(), all_procs.len());

    let high_cpu = all_procs.iter().filter(|p| p.cpu_percent >= 50.0).count();
    let top5: Vec<_> = all_procs.iter().take(5).collect();

    println!();
    println!("  {} {} processes >= 50% CPU", "High CPU:".bold(), high_cpu);

    if !top5.is_empty() {
        println!();
        println!("  {}:", "Top processes by CPU".bold());
        for p in top5 {
            println!(
                "    [{:6}] {:25} {:>6.0}%   {}MB",
                p.pid,
                p.name,
                p.cpu_percent,
                p.memory_mb
            );
        }
    }

    let whitelist = Whitelist::load()?;
    println!();
    println!("  {} {} pattern(s)", "Whitelist:".bold(), whitelist.patterns.len());

    println!();

    Ok(())
}
