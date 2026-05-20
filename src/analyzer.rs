use crate::scanner::ProcessInfo;
use crate::whitelist::Whitelist;

// System apps that should never be killed automatically
const SYSTEM_APPS: &[&str] = &[
    "Safari",
    "Finder",
    "Dock",
    "WindowServer",
    "SystemUIServer",
    "loginwindow",
    "launchd",
    "kernel_task",
    "mds",
    "mds_stores",
    "mdworker",
    "Spotlight",
    "coreaudiod",
    "airportd",
    "bluetoothd",
    "configd",
    "diskarbitrationd",
    "diskmanagementd",
    "coreservicesd",
    "securityd",
    "UserEventAgent",
    "cfprefsd",
    "distnoted",
    "trustd",
    "opendirectoryd",
    "nsurlsessiond",
    "CommCenter",
    "AppleIDAuthAgent",
    "IMDPersistenceAgent",
    "CalendarAgent",
    "AddressBookSourceSync",
    "Notes",
    "Mail",
    "Messages",
    "FaceTime",
    "Photos",
    "Music",
    "TV",
    "Podcasts",
    "News",
    "Stocks",
    "Home",
    "Weather",
    "Maps",
    "App Store",
    "System Preferences",
    "System Settings",
    "Activity Monitor",
    "Terminal",
    "iTerm2",
    "Xcode",
    "Simulator",
    "ControlCenter",
    "NotificationCenter",
    "Siri",
];

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessIssue {
    Orphan,
    Stuck,
    Old(u64), // days running
    Normal,
}

impl ProcessIssue {
    pub fn is_problematic(&self) -> bool {
        !matches!(self, ProcessIssue::Normal)
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProcessIssue::Orphan => "ORPHAN",
            ProcessIssue::Stuck => "STUCK",
            ProcessIssue::Old(_) => "OLD",
            ProcessIssue::Normal => "NORMAL",
        }
    }

    pub fn description(&self) -> String {
        match self {
            ProcessIssue::Orphan => "Orphan process (parent=init) with high CPU".to_string(),
            ProcessIssue::Stuck => "High CPU with no progress (stuck)".to_string(),
            ProcessIssue::Old(days) => format!("Running for {} days with high CPU", days),
            ProcessIssue::Normal => "Normal".to_string(),
        }
    }
}

pub struct AnalyzedProcess {
    pub info: ProcessInfo,
    pub issue: ProcessIssue,
    pub running_secs: u64,
    pub whitelisted: bool,
    pub is_system_app: bool,
}

pub fn is_system_app(name: &str) -> bool {
    SYSTEM_APPS.iter().any(|&sys| {
        name.eq_ignore_ascii_case(sys) || name.starts_with(sys)
    })
}

impl AnalyzedProcess {
    pub fn running_human(&self) -> String {
        format_duration(self.running_secs)
    }
}

pub fn analyze_process(proc: &ProcessInfo) -> ProcessIssue {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let running_secs = now.saturating_sub(proc.start_time);
    let running_days = running_secs / 86400;
    let running_hours = running_secs / 3600;

    // Orphan: parent is init (PID 1) and high CPU
    if proc.parent_pid == Some(1) && proc.cpu_percent > 50.0 && running_hours > 1 {
        return ProcessIssue::Orphan;
    }

    // Old: running > 3 days with high CPU
    if running_days >= 3 && proc.cpu_percent > 30.0 {
        return ProcessIssue::Old(running_days);
    }

    // Stuck: high CPU (>200% on multi-core), running > 1 hour
    // sysinfo reports per-CPU so 200% = 2 full cores
    if proc.cpu_percent > 200.0 && running_hours > 1 {
        return ProcessIssue::Stuck;
    }

    ProcessIssue::Normal
}

pub fn analyze_processes(
    processes: &[ProcessInfo],
    whitelist: &Whitelist,
) -> Vec<AnalyzedProcess> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    processes
        .iter()
        .map(|proc| {
            let issue = analyze_process(proc);
            let running_secs = now.saturating_sub(proc.start_time);
            let whitelisted = whitelist.matches(proc);
            let system_app = is_system_app(&proc.name);
            AnalyzedProcess {
                info: proc.clone(),
                issue,
                running_secs,
                whitelisted,
                is_system_app: system_app,
            }
        })
        .collect()
}

pub fn find_old_processes(processes: &[ProcessInfo], min_age: std::time::Duration) -> Vec<&ProcessInfo> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let min_secs = min_age.as_secs();

    processes
        .iter()
        .filter(|p| {
            let running = now.saturating_sub(p.start_time);
            running >= min_secs && p.cpu_percent > 10.0
        })
        .collect()
}

pub fn parse_duration(s: &str) -> anyhow::Result<std::time::Duration> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('d') {
        let days: u64 = num.parse()?;
        Ok(std::time::Duration::from_secs(days * 86400))
    } else if let Some(num) = s.strip_suffix('h') {
        let hours: u64 = num.parse()?;
        Ok(std::time::Duration::from_secs(hours * 3600))
    } else if let Some(num) = s.strip_suffix('m') {
        let mins: u64 = num.parse()?;
        Ok(std::time::Duration::from_secs(mins * 60))
    } else {
        anyhow::bail!("Invalid duration format '{}'. Use: 3d, 12h, 30m", s)
    }
}

pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{}h {}m", h, m)
        } else {
            format!("{}h", h)
        }
    } else {
        let d = secs / 86400;
        let h = (secs % 86400) / 3600;
        if h > 0 {
            format!("{}d {}h", d, h)
        } else {
            format!("{}d", d)
        }
    }
}
