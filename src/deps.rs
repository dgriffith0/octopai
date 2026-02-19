use std::process::Command;

pub struct Dependency {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    pub recommended: bool,
    pub available: bool,
    pub version: Option<String>,
}

pub fn check_dependencies() -> Vec<Dependency> {
    let mut gh_dep = check_dep(
        "gh",
        "gh",
        "GitHub CLI for issue/PR management (recommended)",
        false,
    );
    gh_dep.recommended = true;
    let mut deps = vec![
        gh_dep,
        check_dep("git", "git", "Version control with worktree support", true),
    ];

    // Terminal multiplexers: at least one recommended; tmux is preferred
    let tmux = check_dep(
        "tmux",
        "tmux",
        "Preferred terminal multiplexer for sessions",
        false,
    );
    let screen = check_dep(
        "screen",
        "screen",
        "Alternative terminal multiplexer (GNU Screen)",
        false,
    );
    let mux_available = tmux.available || screen.available;
    deps.push(Dependency {
        name: "tmux/screen",
        description: "Terminal multiplexer for sessions (tmux preferred)",
        required: false,
        recommended: true,
        available: mux_available,
        version: if tmux.available {
            tmux.version
        } else {
            screen.version
        },
    });

    // Require at least one AI coding assistant (claude or cursor)
    let claude = check_dep(
        "claude",
        "claude",
        "Claude Code CLI for autonomous work",
        false,
    );
    let cursor = check_dep("cursor", "cursor", "Cursor CLI for autonomous work", false);
    let either_available = claude.available || cursor.available;
    deps.push(Dependency {
        name: "claude/cursor",
        description: "AI coding assistant (Claude Code or Cursor)",
        required: true,
        recommended: false,
        available: either_available,
        version: if claude.available {
            claude.version
        } else {
            cursor.version
        },
    });

    deps.push(check_dep(
        "python3",
        "python3",
        "Used by hook script for socket communication",
        true,
    ));
    deps
}

fn check_dep(
    name: &'static str,
    command: &'static str,
    description: &'static str,
    required: bool,
) -> Dependency {
    // tmux and screen use -V instead of --version
    let version_flag = if command == "tmux" || command == "screen" {
        "-V"
    } else {
        "--version"
    };

    let (available, version) = match Command::new(command).arg(version_flag).output() {
        Ok(output) => {
            let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let version_str = if version_str.is_empty() {
                String::from_utf8_lossy(&output.stderr).trim().to_string()
            } else {
                version_str
            };
            // Take just the first line
            let first_line = version_str.lines().next().unwrap_or("").to_string();
            (
                output.status.success(),
                if first_line.is_empty() {
                    None
                } else {
                    Some(first_line)
                },
            )
        }
        Err(_) => (false, None),
    };

    Dependency {
        name,
        description,
        required,
        recommended: false,
        available,
        version,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum PackageManager {
    Brew,
    Apt,
    Dnf,
    Pacman,
    Unknown,
}

pub fn detect_package_manager() -> PackageManager {
    let has = |cmd: &str| {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if has("brew") {
        PackageManager::Brew
    } else if has("apt") {
        PackageManager::Apt
    } else if has("dnf") {
        PackageManager::Dnf
    } else if has("pacman") {
        PackageManager::Pacman
    } else {
        PackageManager::Unknown
    }
}

/// Returns the shell command to install a specific tool, or None if unknown.
pub fn install_command(dep_name: &str, pm: PackageManager) -> Option<String> {
    match dep_name {
        "claude" => Some("npm install -g @anthropic-ai/claude-code".to_string()),
        "cursor" => None, // No reliable single install command
        _ => {
            let pkg = match dep_name {
                "gh" => "gh",
                "git" => "git",
                "tmux" => "tmux",
                "screen" => "screen",
                "python3" => match pm {
                    PackageManager::Brew => "python3",
                    _ => "python3",
                },
                _ => return None,
            };
            match pm {
                PackageManager::Brew => Some(format!("brew install {}", pkg)),
                PackageManager::Apt => Some(format!("sudo apt install -y {}", pkg)),
                PackageManager::Dnf => Some(format!("sudo dnf install -y {}", pkg)),
                PackageManager::Pacman => Some(format!("sudo pacman -S --noconfirm {}", pkg)),
                PackageManager::Unknown => None,
            }
        }
    }
}

/// For compound deps like "tmux/screen", returns the individual choices.
/// First element is the preferred/default choice.
pub fn compound_choices(dep_name: &str) -> Option<Vec<&'static str>> {
    match dep_name {
        "tmux/screen" => Some(vec!["tmux", "screen"]),
        "claude/cursor" => Some(vec!["claude", "cursor"]),
        _ => None,
    }
}

pub fn has_missing_required(deps: &[Dependency]) -> bool {
    deps.iter().any(|d| d.required && !d.available)
}

/// Check if the GitHub CLI (`gh`) is available.
pub fn gh_available() -> bool {
    Command::new("which")
        .arg("gh")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Detect which AI coding assistants are available on the system.
/// Returns `(claude_available, cursor_available)`.
pub fn detect_ai_tools() -> (bool, bool) {
    let claude = Command::new("which")
        .arg("claude")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let cursor = Command::new("which")
        .arg("cursor-agent")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    (claude, cursor)
}
