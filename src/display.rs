use colored::*;
use tabled::{settings::Style, Table, Tabled};

use crate::analyzer::{AnalyzedProcess, ProcessIssue};

#[derive(Tabled)]
pub struct ProcessRow {
    #[tabled(rename = "PID")]
    pub pid: u32,
    #[tabled(rename = "Process")]
    pub name: String,
    #[tabled(rename = "CPU%")]
    pub cpu: String,
    #[tabled(rename = "MEM MB")]
    pub mem: String,
    #[tabled(rename = "Running")]
    pub running: String,
    #[tabled(rename = "Status")]
    pub status: String,
}

impl ProcessRow {
    pub fn from_analyzed(ap: &AnalyzedProcess) -> Self {
        let status_str = match &ap.issue {
            ProcessIssue::Orphan => "[ORPHAN]".to_string(),
            ProcessIssue::Stuck => "[STUCK]".to_string(),
            ProcessIssue::Old(d) => format!("[OLD {}d]", d),
            ProcessIssue::Normal => "[OK]".to_string(),
        };

        let whitelist_tag = if ap.whitelisted { " [WL]" } else { "" };

        ProcessRow {
            pid: ap.info.pid,
            name: ap.info.name.clone(),
            cpu: format!("{:.0}%", ap.info.cpu_percent),
            mem: format!("{}", ap.info.memory_mb),
            running: ap.running_human(),
            status: format!("{}{}", status_str, whitelist_tag),
        }
    }
}

#[allow(dead_code)]
pub fn display_processes(rows: Vec<ProcessRow>, title: &str) {
    println!();
    println!("{}", title.red().bold());

    if rows.is_empty() {
        println!("  {}", "No processes found.".dimmed());
        return;
    }

    let mut table = Table::new(&rows);
    table.with(Style::modern());
    println!("{}", table);
}

pub fn display_analyzed(processes: &[AnalyzedProcess], threshold: u32) {
    println!();
    println!(
        "{}",
        format!("HIGH CPU PROCESSES (>= {}%)", threshold)
            .red()
            .bold()
    );

    if processes.is_empty() {
        println!(
            "  {}",
            "No high-CPU processes found. MacBook is cool!".green()
        );
        return;
    }

    let rows: Vec<ProcessRow> = processes.iter().map(ProcessRow::from_analyzed).collect();

    let mut table = Table::new(&rows);
    table.with(Style::modern());
    println!("{}", table);
}

pub fn display_system_status(load: (f64, f64, f64), mem: (u64, u64), cpu_pct: f32) {
    println!();
    let load_color = if load.0 > 8.0 {
        format!("{:.2}", load.0).red().bold()
    } else if load.0 > 4.0 {
        format!("{:.2}", load.0).yellow().bold()
    } else {
        format!("{:.2}", load.0).green().bold()
    };

    let cpu_color = if cpu_pct > 80.0 {
        format!("{:.0}%", cpu_pct).red()
    } else if cpu_pct > 50.0 {
        format!("{:.0}%", cpu_pct).yellow()
    } else {
        format!("{:.0}%", cpu_pct).green()
    };

    let mem_color = if mem.0 >= mem.1 {
        format!("{}GB/{}GB", mem.0, mem.1).red()
    } else if mem.0 as f32 / mem.1 as f32 > 0.85 {
        format!("{}GB/{}GB", mem.0, mem.1).yellow()
    } else {
        format!("{}GB/{}GB", mem.0, mem.1).green()
    };

    println!(
        "  {} Load {} | CPU {} | Memory {}",
        "System:".bold(),
        load_color,
        cpu_color,
        mem_color
    );
}

pub fn display_summary(processes: &[AnalyzedProcess]) {
    let problematic: Vec<&AnalyzedProcess> = processes
        .iter()
        .filter(|p| p.issue.is_problematic() && !p.whitelisted)
        .collect();

    if problematic.is_empty() {
        println!();
        println!("  {}", "No problematic processes found.".green());
        return;
    }

    let total_wasted_cpu: f32 = problematic.iter().map(|p| p.info.cpu_percent).sum();

    println!();
    println!(
        "  {} Found {} problematic processes ({:.0}% CPU wasted)",
        "Hint:".yellow().bold(),
        problematic.len().to_string().red().bold(),
        total_wasted_cpu
    );
    println!(
        "     Run {} to fix",
        "`cooldown clean`".cyan().bold()
    );
}

pub fn display_kill_plan(
    to_kill: &[(u32, String, String)],
    whitelisted: &[(u32, String, String)],
    dry_run: bool,
) {
    if dry_run {
        println!();
        println!(
            "{}",
            "DRY RUN - No processes will be killed".yellow().bold()
        );
    }

    if to_kill.is_empty() {
        println!();
        println!("  {}", "Nothing to kill.".green());
    } else {
        println!();
        println!("  {}:", "Would kill".red().bold());
        for (pid, name, reason) in to_kill {
            println!(
                "    {} PID {} ({}) - {}",
                "x".red(),
                pid.to_string().bold(),
                name.cyan(),
                reason.dimmed()
            );
        }
    }

    if !whitelisted.is_empty() {
        println!();
        println!("  {}:", "Skipping (whitelisted)".dimmed());
        for (pid, name, pattern) in whitelisted {
            println!(
                "    {} PID {} ({}) - matches \"{}\"",
                "-".dimmed(),
                pid,
                name.dimmed(),
                pattern.dimmed()
            );
        }
    }

    if dry_run && !to_kill.is_empty() {
        println!();
        println!(
            "  Run without {} to execute.",
            "--dry-run".cyan()
        );
    }
}

pub fn display_kill_results(results: &[crate::killer::KillResult], freed_cpu: f32, new_load: f64) {
    println!();
    for r in results {
        if r.success {
            println!(
                "  {} Killed PID {} ({})",
                "OK".green().bold(),
                r.pid.to_string().bold(),
                r.name.cyan()
            );
        } else {
            println!(
                "  {} Failed PID {} ({}): {}",
                "ERR".red().bold(),
                r.pid,
                r.name.cyan(),
                r.error.as_deref().unwrap_or("unknown error").red()
            );
        }
    }

    let success_count = results.iter().filter(|r| r.success).count();
    if success_count > 0 {
        println!();
        println!(
            "  {} Freed {:.0}% CPU | New load: {:.2}",
            "Done!".green().bold(),
            freed_cpu,
            new_load
        );
    }
}

pub fn display_whitelist(patterns: &[String]) {
    println!();
    println!("{}", "Whitelisted Patterns".bold());
    println!("{}", "-------------------".dimmed());

    if patterns.is_empty() {
        println!("  {}", "(empty)".dimmed());
    } else {
        for (i, p) in patterns.iter().enumerate() {
            println!("  {}. {}", i + 1, p.cyan());
        }
    }
    println!();
    println!(
        "  Config: {}",
        crate::whitelist::Whitelist::config_path()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "unknown".to_string())
            .dimmed()
    );
}

pub fn display_watch_header(threshold: u32, interval: u64) {
    println!();
    println!("{}", "COOLDOWN WATCH MODE".cyan().bold());
    println!(
        "  Threshold: {}% | Interval: {}s | Ctrl+C to stop",
        threshold, interval
    );
    println!("{}", "-".repeat(60).dimmed());
}

pub fn display_watch_tick(tick: u64) {
    let ts = chrono::Local::now().format("%H:%M:%S");
    println!();
    println!(
        "{} {} [tick #{}]",
        ">>>".cyan().bold(),
        ts.to_string().dimmed(),
        tick
    );
}
