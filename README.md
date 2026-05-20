# 🧊 Cooldown

**Keep your MacBook cool by managing CPU-heavy processes.**

A fast, Rust-based CLI tool that scans, monitors, and manages resource-hungry processes on macOS. Perfect for developers running multiple heavy workloads.

## Features

- **Smart Scanning** - Detects orphan, stuck, and long-running high-CPU processes
- **Process Table** - View top 15 processes sorted by CPU with status indicators
- **Protected Apps** - Auto-protects system apps, dev tools (claude, node, cargo, etc.)
- **Thermal Monitoring** - CPU temperature, fan speed, thermal level
- **Battery Aware** - Adjusts thresholds based on power source
- **CPU Throttling** - Limit any process to a specific CPU percentage
- **Maintenance Tasks** - DNS flush, memory purge, Time Machine cleanup
- **Interactive Menu** - Easy selection with arrow keys

## Installation

```bash
# Build from source
cargo build --release

# Install to PATH
sudo cp target/release/cooldown /usr/local/bin/cooldown
```

## Usage

### Interactive Mode (Default)

```bash
cooldown
```

```
COOLDOWN
========

  System: Load 3.60 | CPU 34% | Mem 12GB/16GB
  Thermal: ❄️ Nominal | 45°C | 🔌 100%

  ALL PROCESSES (sorted by CPU)
  ──────────────────────────────────────────────────────────────────────
    PID  NAME                      CPU%    MEM      TIME  STATUS
  ──────────────────────────────────────────────────────────────────────
   40632  claude                   101%      1M    4d 5h  [SYS]
   92281  claude                    60%    304M    1d 2h  [SYS]
   77670  com.apple.WebKit.Web…     27%    195M      56m  [OK]
  ──────────────────────────────────────────────────────────────────────

  ⚠ 1 killable problematic (45% CPU)
  ℹ 5 killable | 3 protected

? Action
❯ Kill all problematic (1 proc, 45% CPU)
  Select from 5 killable processes
  Throttle a process
  Run maintenance
  Refresh
  Exit
```

### Commands

```bash
cooldown              # Interactive mode with process table
cooldown status       # Quick system overview
cooldown scan         # Scan high CPU processes
cooldown scan -t 30   # Custom threshold
cooldown clean        # Kill problematic processes
cooldown clean -n     # Dry run (preview)
```

### Kill Processes

```bash
cooldown kill --orphans      # Kill orphan processes
cooldown kill --stuck        # Kill stuck processes  
cooldown kill --old 3d       # Kill processes running > 3 days
cooldown kill --pid 12345    # Kill specific PID
```

### Throttle CPU

```bash
cooldown throttle 12345 --cpu 50       # Limit PID to 50% CPU
cooldown throttle 12345 --cpu 30 -d 60 # Limit for 60 seconds
```

### Wrap Command

Run commands with thermal safety:

```bash
cooldown wrap cargo build              # Monitor thermal
cooldown wrap --cpu 50 npm run build   # Limit to 50% CPU
cooldown wrap --thermal heavy make     # Kill if thermal heavy
cooldown wrap --timeout 300 make all   # Timeout after 5min
```

### Maintenance

```bash
cooldown maintenance --dns         # Flush DNS cache
cooldown maintenance --purgeable   # Free purgeable space
cooldown maintenance --timemachine # Clear Time Machine snapshots
cooldown maintenance --all         # Run all
```

### Whitelist

```bash
cooldown whitelist add "my-process"
cooldown whitelist list
cooldown whitelist remove "my-process"
```

## Process Status

| Status | Description |
|--------|-------------|
| `[ORPHAN]` | Parent PID = 1, high CPU, running > 1h |
| `[STUCK]` | CPU > 200%, running > 1h |
| `[OLD]` | Running >= 3 days with high CPU |
| `[SYS]` | System/dev app (protected) |
| `[WL]` | Whitelisted |
| `[OK]` | Normal |

## Protected Apps

Automatically protected from being flagged as problematic:
- **Dev tools**: claude, code, node, cargo, rustc, python, go, java
- **System**: Safari, Finder, Dock, WindowServer, kernel_task, etc.
- **Apple apps**: Mail, Messages, Photos, Music, Xcode, Terminal, etc.

## Thermal Levels

| Level | Emoji | Meaning |
|-------|-------|---------|
| Nominal | ❄️ | Cool, normal |
| Moderate | 🌡️ | Slightly warm |
| Heavy | 🔥 | Hot, consider action |
| Critical | 🚨 | Very hot, immediate action |

## Configuration

Config file: `~/Library/Application Support/cooldown/config.toml`

```bash
cooldown config --init   # Create default config
cooldown config --show   # Show current config
cooldown config --set cpu_threshold=30
```

## Requirements

- macOS 10.15+
- Rust 1.70+ (for building)

## License

MIT
