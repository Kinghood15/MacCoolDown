use anyhow::Context;
use clap::{Parser, Subcommand};
use colored::*;

mod analyzer;
mod config;
mod display;
mod interactive;
mod killer;
mod maintenance;
mod scanner;
mod thermal;
mod throttle;
mod watcher;
mod whitelist;
mod wrapper;

use analyzer::{analyze_processes, find_old_processes, parse_duration, ProcessIssue};
use config::{display_config, Config};
use display::{
    display_analyzed, display_kill_plan, display_kill_results, display_summary,
    display_system_status, display_whitelist,
};
use killer::kill_process;
use maintenance::run_maintenance;
use scanner::{get_cpu_usage, get_memory_info, get_system_load, scan_all_processes, scan_processes};
use thermal::{display_thermal_status, get_thermal_status, watch_thermal};
use throttle::run_throttle;
use watcher::{run_watch, WatchConfig};
use whitelist::Whitelist;
use wrapper::run_wrap;

#[derive(Parser)]
#[command(name = "mac-cooldown")]
#[command(about = "Keep your MacBook cool by managing CPU-heavy processes")]
#[command(version = "0.4.0")]
#[command(long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
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

    /// Realtime dashboard with auto-refresh (default when no command)
    Live,

    /// Interactive menu mode
    Menu,

    /// Show thermal status (temperature, fan, power)
    Thermal {
        /// Watch mode - continuously monitor thermal status
        #[arg(short, long)]
        watch: bool,

        /// Watch interval in seconds (default 5)
        #[arg(short, long, default_value = "5")]
        interval: u64,
    },

    /// Throttle a process to limit CPU usage
    Throttle {
        /// Process ID to throttle
        pid: u32,

        /// CPU limit percentage (1-99)
        #[arg(short, long, default_value = "50")]
        cpu: u32,

        /// Duration in seconds (omit for unlimited)
        #[arg(short, long)]
        duration: Option<u64>,
    },

    /// Run a command with thermal safety and CPU limiting
    Wrap {
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,

        /// CPU limit percentage (omit for no limit)
        #[arg(short, long)]
        cpu: Option<u32>,

        /// Thermal level limit: moderate, heavy, critical (default: critical)
        #[arg(short, long)]
        thermal: Option<String>,

        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Run maintenance tasks (DNS flush, purge memory, etc.)
    Maintenance {
        /// Flush DNS cache
        #[arg(long)]
        dns: bool,

        /// Free purgeable disk space
        #[arg(long)]
        purgeable: bool,

        /// Clear Time Machine local snapshots
        #[arg(long)]
        timemachine: bool,

        /// Run all maintenance tasks
        #[arg(short, long)]
        all: bool,
    },

    /// Manage configuration
    Config {
        /// Create default configuration file
        #[arg(long)]
        init: bool,

        /// Show current configuration
        #[arg(long)]
        show: bool,

        /// Set a configuration value (format: key=value)
        #[arg(long)]
        set: Option<String>,
    },
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => interactive::run_realtime(),
        Some(Commands::Scan { threshold, json }) => cmd_scan(threshold, json),
        Some(Commands::Clean { dry_run, force }) => cmd_clean(dry_run, force),
        Some(Commands::Kill {
            orphans,
            stuck,
            old,
            pid,
            force,
            dry_run,
        }) => cmd_kill(orphans, stuck, old, pid, force, dry_run),
        Some(Commands::Watch {
            threshold,
            interval,
            auto_clean,
        }) => cmd_watch(threshold, interval, auto_clean),
        Some(Commands::Whitelist { action }) => cmd_whitelist(action),
        Some(Commands::Status) => cmd_status(),
        Some(Commands::Live) => interactive::run_realtime(),
        Some(Commands::Menu) => interactive::run_interactive(),
        Some(Commands::Thermal { watch, interval }) => cmd_thermal(watch, interval),
        Some(Commands::Throttle { pid, cpu, duration }) => cmd_throttle(pid, cpu, duration),
        Some(Commands::Wrap {
            command,
            cpu,
            thermal,
            timeout,
        }) => cmd_wrap(command, cpu, thermal, timeout),
        Some(Commands::Maintenance {
            dns,
            purgeable,
            timemachine,
            all,
        }) => cmd_maintenance(dns, purgeable, timemachine, all),
        Some(Commands::Config { init, show, set }) => cmd_config(init, show, set),
    }
}

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
    let processes = scan_processes(20.0);
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
            println!(
                "  {} Killed PID {} ({})",
                "OK".green().bold(),
                target_pid,
                name.cyan()
            );
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
            println!(
                "  {} Added pattern: {}",
                "OK".green().bold(),
                pattern.cyan()
            );
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
    println!(
        "  {} {:.2} | {:.2} | {:.2}",
        "Load avg (1/5/15m):".bold(),
        load.0,
        load.1,
        load.2
    );
    println!("  {} {:.0}%", "CPU usage:".bold(), cpu);
    println!("  {} {}GB / {}GB", "Memory:".bold(), mem.0, mem.1);

    let all_procs = scan_processes(0.0);
    println!("  {} {}", "Total processes:".bold(), all_procs.len());

    let high_cpu = all_procs.iter().filter(|p| p.cpu_percent >= 50.0).count();
    let top5: Vec<_> = all_procs.iter().take(5).collect();

    println!();
    println!(
        "  {} {} processes >= 50% CPU",
        "High CPU:".bold(),
        high_cpu
    );

    if !top5.is_empty() {
        println!();
        println!("  {}:", "Top processes by CPU".bold());
        for p in top5 {
            println!(
                "    [{:6}] {:25} {:>6.0}%   {}MB",
                p.pid, p.name, p.cpu_percent, p.memory_mb
            );
        }
    }

    let whitelist = Whitelist::load()?;
    println!();
    println!(
        "  {} {} pattern(s)",
        "Whitelist:".bold(),
        whitelist.patterns.len()
    );

    println!();

    Ok(())
}

fn cmd_thermal(watch: bool, interval: u64) -> anyhow::Result<()> {
    if watch {
        watch_thermal(interval)
    } else {
        match get_thermal_status() {
            Ok(status) => {
                display_thermal_status(&status);
                Ok(())
            }
            Err(e) => {
                println!();
                println!(
                    "  {} Could not get thermal status: {}",
                    "WARN".yellow().bold(),
                    e
                );
                println!(
                    "  {} Try running with sudo: {}",
                    "TIP:".dimmed(),
                    "sudo mac-cooldown thermal".cyan()
                );
                println!();
                Ok(())
            }
        }
    }
}

fn cmd_throttle(pid: u32, cpu: u32, duration: Option<u64>) -> anyhow::Result<()> {
    run_throttle(pid, cpu, duration)
}

fn cmd_wrap(
    command: Vec<String>,
    cpu: Option<u32>,
    thermal: Option<String>,
    timeout: Option<u64>,
) -> anyhow::Result<()> {
    run_wrap(command, cpu, thermal, timeout)
}

fn cmd_maintenance(dns: bool, purgeable: bool, timemachine: bool, all: bool) -> anyhow::Result<()> {
    run_maintenance(dns, purgeable, timemachine, all)
}

fn cmd_config(init: bool, show: bool, set: Option<String>) -> anyhow::Result<()> {
    if init {
        let exists = Config::exists()?;
        if exists {
            println!("  {} Configuration file already exists.", "WARN".yellow().bold());
            if let Ok(path) = Config::config_path() {
                println!("  {} {}", "Path:".dimmed(), path.display());
            }
            return Ok(());
        }
        let path = Config::init()?;
        println!(
            "  {} Created configuration file at: {}",
            "OK".green().bold(),
            path.display()
        );
        return Ok(());
    }

    if show {
        let config = Config::load()?;
        display_config(&config);
        return Ok(());
    }

    if let Some(kv) = set {
        let parts: Vec<&str> = kv.splitn(2, '=').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid format. Use: --set key=value");
        }

        let key = parts[0].trim();
        let value = parts[1].trim();

        let mut config = Config::load()?;

        match key {
            "cpu_threshold" => {
                config.cpu_threshold = value.parse().context("Invalid number")?;
            }
            "watch_interval" => {
                config.watch_interval = value.parse().context("Invalid number")?;
            }
            "auto_clean" => {
                config.auto_clean = value.parse().context("Invalid boolean")?;
            }
            "thermal_limit" => {
                config.thermal_limit = value.to_string();
            }
            "wrap_cpu_limit" => {
                config.wrap_cpu_limit = value.parse().context("Invalid number")?;
            }
            _ => {
                anyhow::bail!("Unknown config key: {}", key);
            }
        }

        config.save()?;
        println!(
            "  {} Set {} = {}",
            "OK".green().bold(),
            key.cyan(),
            value.green()
        );
        return Ok(());
    }

    println!();
    println!(
        "  {}",
        "Use --init to create config, --show to display, or --set key=value".yellow()
    );
    println!();
    Ok(())
}
