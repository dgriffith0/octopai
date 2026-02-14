use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use color_eyre::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};

struct Card {
    id: String,
    title: String,
    description: String,
    full_description: Option<String>,
    tag: String,
    tag_color: Color,
    related: Vec<String>,
}

enum ConfirmAction {
    CloseIssue { number: u64 },
    RemoveWorktree { path: String, branch: String },
}

struct ConfirmModal {
    message: String,
    on_confirm: ConfirmAction,
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    Filtering { query: String },
    CreatingIssue,
    Confirming,
}

#[derive(PartialEq)]
enum Screen {
    RepoSelect,
    Board,
}

#[derive(PartialEq)]
enum RepoSelectPhase {
    Typing,
    Loading,
    Picking,
}

struct RepoSelectState {
    input: String,
    repos: Vec<String>,
    filtered_repos: Vec<String>,
    selected: usize,
    phase: RepoSelectPhase,
    error: Option<String>,
    filter_query: String,
}

impl RepoSelectState {
    fn new() -> Self {
        Self {
            input: String::new(),
            repos: Vec::new(),
            filtered_repos: Vec::new(),
            selected: 0,
            phase: RepoSelectPhase::Typing,
            error: None,
            filter_query: String::new(),
        }
    }

    fn update_filtered(&mut self) {
        if self.filter_query.is_empty() {
            self.filtered_repos = self.repos.clone();
        } else {
            self.filtered_repos = self
                .repos
                .iter()
                .filter(|r| fuzzy_match(&self.filter_query, r))
                .cloned()
                .collect();
        }
        if self.selected >= self.filtered_repos.len() {
            self.selected = if self.filtered_repos.is_empty() {
                0
            } else {
                self.filtered_repos.len() - 1
            };
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Config {
    repo: String,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("roctopai")
        .join("config.json")
}

fn load_config() -> Option<Config> {
    let path = config_path();
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_config(repo: &str) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let config = Config {
        repo: repo.to_string(),
    };
    fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

fn fetch_repos(owner: &str) -> std::result::Result<Vec<String>, String> {
    let output = Command::new("gh")
        .args([
            "repo",
            "list",
            owner,
            "--json",
            "nameWithOwner",
            "--limit",
            "50",
            "-q",
            ".[].nameWithOwner",
        ])
        .output()
        .map_err(|e| format!("Failed to run gh: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh error: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let repos: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    if repos.is_empty() {
        return Err(format!("No repos found for '{}'", owner));
    }

    Ok(repos)
}

fn label_color(name: &str) -> Color {
    match name.to_lowercase().as_str() {
        s if s.contains("bug") => Color::Red,
        s if s.contains("feature") || s.contains("enhancement") => Color::Green,
        s if s.contains("documentation") || s.contains("docs") => Color::Blue,
        s if s.contains("good first issue") || s.contains("help wanted") => Color::Cyan,
        s if s.contains("duplicate") || s.contains("wontfix") || s.contains("invalid") => {
            Color::Gray
        }
        s if s.contains("priority") || s.contains("critical") || s.contains("urgent") => {
            Color::LightRed
        }
        _ => Color::Yellow,
    }
}

fn fetch_issues(repo: &str) -> Vec<Card> {
    let output = Command::new("gh")
        .args([
            "issue",
            "list",
            "--repo",
            repo,
            "--json",
            "number,title,body,labels",
            "--limit",
            "30",
        ])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let issues: Vec<serde_json::Value> = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    issues
        .into_iter()
        .map(|issue| {
            let number = issue["number"].as_u64().unwrap_or(0);
            let title = issue["title"].as_str().unwrap_or("").to_string();
            let body = issue["body"].as_str().unwrap_or("").to_string();
            let full_description = if body.is_empty() {
                None
            } else {
                Some(body.clone())
            };
            let description = if body.len() > 80 {
                format!("{}...", &body[..77])
            } else if body.is_empty() {
                "No description".to_string()
            } else {
                body
            };

            let labels = issue["labels"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| l["name"].as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let (tag, tag_color) = if let Some(first) = labels.first() {
                (first.clone(), label_color(first))
            } else {
                ("open".to_string(), Color::Green)
            };

            Card {
                id: format!("issue-{}", number),
                title: format!("#{} {}", number, title),
                description,
                full_description,
                tag,
                tag_color,
                related: Vec::new(),
            }
        })
        .collect()
}

fn create_issue(repo: &str, title: &str, body: &str) -> std::result::Result<(), String> {
    let output = Command::new("gh")
        .args([
            "issue",
            "create",
            "--repo",
            repo,
            "--title",
            title,
            "--body",
            body,
        ])
        .output()
        .map_err(|e| format!("Failed to run gh: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh error: {}", stderr.trim()));
    }

    Ok(())
}

fn get_repo_name(repo: &str) -> &str {
    repo.split('/').last().unwrap_or(repo)
}

fn fetch_worktrees() -> Vec<Card> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut cards = Vec::new();

    for block in stdout.split("\n\n") {
        let mut path = String::new();
        let mut branch = String::new();
        let mut is_bare = false;

        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = p.to_string();
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = b.to_string();
            } else if line == "bare" {
                is_bare = true;
            }
        }

        if path.is_empty() || is_bare {
            continue;
        }

        let display_name = if branch.is_empty() {
            path.split('/').last().unwrap_or(&path).to_string()
        } else {
            branch.clone()
        };

        let is_main = display_name == "main" || display_name == "master";
        let tag = if is_main { "primary" } else { "branch" };
        let tag_color = if is_main { Color::Green } else { Color::Yellow };

        // Link issue-N worktrees to issue cards
        let related = if let Some(num) = display_name.strip_prefix("issue-") {
            vec![format!("issue-{}", num)]
        } else {
            Vec::new()
        };

        cards.push(Card {
            id: format!("wt-{}", display_name),
            title: display_name,
            description: path,
            full_description: None,
            tag: tag.to_string(),
            tag_color,
            related,
        });
    }

    cards
}

fn close_issue(repo: &str, number: u64) -> std::result::Result<(), String> {
    let output = Command::new("gh")
        .args([
            "issue",
            "close",
            "--repo",
            repo,
            &number.to_string(),
        ])
        .output()
        .map_err(|e| format!("Failed to run gh: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh error: {}", stderr.trim()));
    }

    Ok(())
}

fn remove_worktree(path: &str, branch: &str) -> std::result::Result<(), String> {
    // Kill tmux session if it exists (named after branch)
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", branch])
        .output();

    let output = Command::new("git")
        .args(["worktree", "remove", path])
        .output()
        .map_err(|e| format!("Failed to run git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree remove error: {}", stderr.trim()));
    }

    // Delete the branch
    let _ = Command::new("git")
        .args(["branch", "-D", branch])
        .output();

    Ok(())
}

fn create_worktree_and_session(
    repo: &str,
    number: u64,
    title: &str,
    body: &str,
) -> std::result::Result<(), String> {
    let repo_name = get_repo_name(repo);
    let branch = format!("issue-{}", number);
    let worktree_path = format!("../{}-issue-{}", repo_name, number);

    // Create worktree with new branch
    let output = Command::new("git")
        .args(["worktree", "add", &worktree_path, "-b", &branch])
        .output()
        .map_err(|e| format!("Failed to run git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add error: {}", stderr.trim()));
    }

    // Create tmux session with neovim in the first pane
    let output = Command::new("tmux")
        .args(["new-session", "-d", "-s", &branch, "-c", &worktree_path, "nvim", "."])
        .output()
        .map_err(|e| format!("Failed to create tmux session: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tmux error: {}", stderr.trim()));
    }

    // Split right pane for Claude
    let output = Command::new("tmux")
        .args(["split-window", "-h", "-t", &branch, "-c", &worktree_path])
        .output()
        .map_err(|e| format!("Failed to split tmux pane: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tmux split error: {}", stderr.trim()));
    }

    // Build the Claude prompt
    let prompt = format!(
        "You are working on GitHub issue #{} for the repo {}.\n\nTitle: {}\n\n{}\n\nPlease investigate the codebase and implement a solution for this issue.",
        number,
        repo,
        title,
        if body.is_empty() { "No description provided." } else { body }
    );

    // Send claude command to the right pane (the active one after split)
    let claude_cmd = format!(
        "claude -p '{}' --allowedTools 'Read,Edit,Bash' --max-turns 10",
        prompt.replace('\'', "'\\''")
    );

    let _ = Command::new("tmux")
        .args(["send-keys", "-t", &branch, &claude_cmd, "Enter"])
        .output();

    Ok(())
}

struct IssueModal {
    title: String,
    body: String,
    active_field: usize, // 0 = title, 1 = body
    error: Option<String>,
}

impl IssueModal {
    fn new() -> Self {
        Self {
            title: String::new(),
            body: String::new(),
            active_field: 0,
            error: None,
        }
    }
}

struct App {
    screen: Screen,
    repo_select: RepoSelectState,
    repo: String,
    issues: Vec<Card>,
    worktrees: Vec<Card>,
    pull_requests: Vec<Card>,
    sessions: Vec<Card>,
    active_section: usize,
    selected_card: [usize; 4],
    mode: Mode,
    issue_modal: Option<IssueModal>,
    confirm_modal: Option<ConfirmModal>,
    status_message: Option<String>,
}

impl App {
    fn new() -> Self {
        Self {
            screen: Screen::RepoSelect,
            repo_select: RepoSelectState::new(),
            repo: String::new(),
            issues: Vec::new(),
            worktrees: Vec::new(),
            pull_requests: Vec::new(),
            active_section: 0,
            selected_card: [0; 4],
            mode: Mode::Normal,
            issue_modal: None,
            confirm_modal: None,
            status_message: None,
            sessions: Vec::new(),
        }
    }
}

impl App {
    fn section_cards(&self, section: usize) -> &[Card] {
        match section {
            0 => &self.issues,
            1 => &self.worktrees,
            2 => &self.pull_requests,
            3 => &self.sessions,
            _ => &[],
        }
    }

    fn section_card_count(&self, section: usize) -> usize {
        self.section_cards(section).len()
    }

    fn clamp_selected(&mut self) {
        let s = self.active_section;
        let count = self.section_card_count(s);
        if count == 0 {
            self.selected_card[s] = 0;
        } else if self.selected_card[s] >= count {
            self.selected_card[s] = count - 1;
        }
    }

    fn move_card_up(&mut self) {
        let s = self.active_section;
        if self.selected_card[s] > 0 {
            self.selected_card[s] -= 1;
        }
    }

    fn move_card_down(&mut self) {
        let s = self.active_section;
        let count = self.section_card_count(s);
        if count > 0 && self.selected_card[s] < count - 1 {
            self.selected_card[s] += 1;
        }
    }

    fn selected_card_related_ids(&self) -> HashSet<String> {
        let cards = self.section_cards(self.active_section);
        let idx = self.selected_card[self.active_section];
        if let Some(card) = cards.get(idx) {
            card.related.iter().cloned().collect()
        } else {
            HashSet::new()
        }
    }

    fn enter_repo_select(&mut self) {
        let owner = if self.repo.contains('/') {
            self.repo.split('/').next().unwrap_or("").to_string()
        } else {
            String::new()
        };
        self.repo_select = RepoSelectState::new();
        self.repo_select.input = owner;
        self.screen = Screen::RepoSelect;
    }
}

fn main() -> Result<()> {
    color_eyre::install()?;
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new();

    // Load saved config
    if let Some(config) = load_config() {
        if !config.repo.is_empty() {
            app.repo = config.repo.clone();
            app.issues = fetch_issues(&config.repo);
            app.worktrees = fetch_worktrees();
            app.selected_card = [0; 4];
            app.screen = Screen::Board;
        }
    }

    loop {
        terminal.draw(|frame| match app.screen {
            Screen::RepoSelect => ui_repo_select(frame, &app.repo_select),
            Screen::Board => ui(frame, &app),
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match app.screen {
                Screen::RepoSelect => {
                    match app.repo_select.phase {
                        RepoSelectPhase::Typing => match key.code {
                            KeyCode::Esc => {
                                if app.repo.is_empty() {
                                    break; // quit if no board to return to
                                } else {
                                    app.screen = Screen::Board;
                                }
                            }
                            KeyCode::Enter => {
                                let owner = app.repo_select.input.trim().to_string();
                                if owner.is_empty() {
                                    app.repo_select.error =
                                        Some("Please enter an org or user name".into());
                                } else {
                                    app.repo_select.error = None;
                                    app.repo_select.phase = RepoSelectPhase::Loading;
                                    // We need to redraw to show loading state, then fetch
                                    terminal.draw(|frame| {
                                        ui_repo_select(frame, &app.repo_select)
                                    })?;

                                    match fetch_repos(&owner) {
                                        Ok(repos) => {
                                            app.repo_select.repos = repos;
                                            app.repo_select.filter_query.clear();
                                            app.repo_select.update_filtered();
                                            app.repo_select.selected = 0;
                                            app.repo_select.phase = RepoSelectPhase::Picking;
                                        }
                                        Err(e) => {
                                            app.repo_select.error = Some(e);
                                            app.repo_select.phase = RepoSelectPhase::Typing;
                                        }
                                    }
                                }
                            }
                            KeyCode::Backspace => {
                                app.repo_select.input.pop();
                            }
                            KeyCode::Char(c) => {
                                app.repo_select.input.push(c);
                            }
                            _ => {}
                        },
                        RepoSelectPhase::Loading => {
                            // No input during loading
                        }
                        RepoSelectPhase::Picking => match key.code {
                            KeyCode::Esc => {
                                app.repo_select.phase = RepoSelectPhase::Typing;
                                app.repo_select.filter_query.clear();
                            }
                            KeyCode::Enter => {
                                if let Some(repo) =
                                    app.repo_select.filtered_repos.get(app.repo_select.selected)
                                {
                                    let repo = repo.clone();
                                    let _ = save_config(&repo);
                                    app.issues = fetch_issues(&repo);
                                    app.worktrees = fetch_worktrees();
                                    app.selected_card = [0; 4];
                                    app.repo = repo;
                                    app.screen = Screen::Board;
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if app.repo_select.selected > 0 {
                                    app.repo_select.selected -= 1;
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if !app.repo_select.filtered_repos.is_empty()
                                    && app.repo_select.selected
                                        < app.repo_select.filtered_repos.len() - 1
                                {
                                    app.repo_select.selected += 1;
                                }
                            }
                            KeyCode::Char('/') => {
                                // Toggle filter — if already filtering, this adds '/' to query
                                // Start fresh filter
                                app.repo_select.filter_query.clear();
                                app.repo_select.update_filtered();
                            }
                            KeyCode::Backspace => {
                                app.repo_select.filter_query.pop();
                                app.repo_select.update_filtered();
                            }
                            KeyCode::Char(c) => {
                                if c != '/' {
                                    app.repo_select.filter_query.push(c);
                                    app.repo_select.update_filtered();
                                }
                            }
                            _ => {}
                        },
                    }
                }
                Screen::Board => {
                    match &mut app.mode {
                        Mode::Filtering { query } => match key.code {
                            KeyCode::Esc => {
                                app.mode = Mode::Normal;
                            }
                            KeyCode::Backspace => {
                                query.pop();
                                app.clamp_selected();
                            }
                            KeyCode::Up => {
                                app.move_card_up();
                            }
                            KeyCode::Down => {
                                app.move_card_down();
                            }
                            KeyCode::Char(c) => {
                                query.push(c);
                                app.clamp_selected();
                            }
                            _ => {}
                        },
                        Mode::Normal => {
                            // Clear status message on any keypress
                            app.status_message = None;
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => break,
                                KeyCode::Enter => {
                                    app.enter_repo_select();
                                }
                                KeyCode::Tab => {
                                    app.active_section = (app.active_section + 1) % 4;
                                }
                                KeyCode::BackTab => {
                                    app.active_section = (app.active_section + 3) % 4;
                                }
                                KeyCode::Char('/') => {
                                    app.mode = Mode::Filtering {
                                        query: String::new(),
                                    };
                                }
                                KeyCode::Char('n') if app.active_section == 0 => {
                                    app.mode = Mode::CreatingIssue;
                                    app.issue_modal = Some(IssueModal::new());
                                }
                                KeyCode::Char('w') if app.active_section == 0 => {
                                    if let Some(card) = app.issues.get(app.selected_card[0]) {
                                        // Extract issue number from id "issue-N"
                                        if let Some(num_str) = card.id.strip_prefix("issue-") {
                                            if let Ok(number) = num_str.parse::<u64>() {
                                                let title = card.title.clone();
                                                let body = card
                                                    .full_description
                                                    .clone()
                                                    .unwrap_or_default();
                                                let repo = app.repo.clone();
                                                match create_worktree_and_session(
                                                    &repo, number, &title, &body,
                                                ) {
                                                    Ok(()) => {
                                                        app.worktrees = fetch_worktrees();
                                                        app.clamp_selected();
                                                        app.status_message = Some(format!(
                                                            "Created worktree and session for issue #{}",
                                                            number
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        app.status_message =
                                                            Some(format!("Error: {}", e));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('d') if app.active_section == 0 => {
                                    if let Some(card) = app.issues.get(app.selected_card[0]) {
                                        if let Some(num_str) = card.id.strip_prefix("issue-") {
                                            if let Ok(number) = num_str.parse::<u64>() {
                                                app.confirm_modal = Some(ConfirmModal {
                                                    message: format!(
                                                        "Close issue #{}?\n\n{}",
                                                        number, card.title
                                                    ),
                                                    on_confirm: ConfirmAction::CloseIssue {
                                                        number,
                                                    },
                                                });
                                                app.mode = Mode::Confirming;
                                            }
                                        }
                                    }
                                }
                                KeyCode::Char('d') if app.active_section == 1 => {
                                    if let Some(card) = app.worktrees.get(app.selected_card[1]) {
                                        let branch = card.title.clone();
                                        if branch == "main" || branch == "master" {
                                            app.status_message = Some(
                                                "Cannot remove main/master worktree".to_string(),
                                            );
                                        } else {
                                            let path = card.description.clone();
                                            app.confirm_modal = Some(ConfirmModal {
                                                message: format!(
                                                    "Remove worktree '{}'?\n\nPath: {}\nThis will also delete the branch and kill any tmux session.",
                                                    branch, path
                                                ),
                                                on_confirm: ConfirmAction::RemoveWorktree {
                                                    path,
                                                    branch,
                                                },
                                            });
                                            app.mode = Mode::Confirming;
                                        }
                                    }
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    app.move_card_up();
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    app.move_card_down();
                                }
                                _ => {}
                            }
                        }
                        Mode::Confirming => match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(modal) = app.confirm_modal.take() {
                                    match modal.on_confirm {
                                        ConfirmAction::CloseIssue { number } => {
                                            let repo = app.repo.clone();
                                            match close_issue(&repo, number) {
                                                Ok(()) => {
                                                    app.issues = fetch_issues(&repo);
                                                    app.clamp_selected();
                                                    app.status_message = Some(format!(
                                                        "Closed issue #{}",
                                                        number
                                                    ));
                                                }
                                                Err(e) => {
                                                    app.status_message =
                                                        Some(format!("Error: {}", e));
                                                }
                                            }
                                        }
                                        ConfirmAction::RemoveWorktree { path, branch } => {
                                            match remove_worktree(&path, &branch) {
                                                Ok(()) => {
                                                    app.worktrees = fetch_worktrees();
                                                    app.clamp_selected();
                                                    app.status_message = Some(format!(
                                                        "Removed worktree '{}'",
                                                        branch
                                                    ));
                                                }
                                                Err(e) => {
                                                    app.status_message =
                                                        Some(format!("Error: {}", e));
                                                }
                                            }
                                        }
                                    }
                                }
                                app.mode = Mode::Normal;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                app.confirm_modal = None;
                                app.mode = Mode::Normal;
                            }
                            _ => {}
                        },
                        Mode::CreatingIssue => {
                            if let Some(modal) = &mut app.issue_modal {
                                match key.code {
                                    KeyCode::Esc => {
                                        app.issue_modal = None;
                                        app.mode = Mode::Normal;
                                    }
                                    KeyCode::Tab => {
                                        modal.active_field = if modal.active_field == 0 {
                                            1
                                        } else {
                                            0
                                        };
                                    }
                                    KeyCode::Enter if modal.active_field == 0 => {
                                        modal.active_field = 1;
                                    }
                                    KeyCode::Char('s')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        let title = modal.title.trim().to_string();
                                        if title.is_empty() {
                                            modal.error =
                                                Some("Title cannot be empty".to_string());
                                        } else {
                                            let body = modal.body.clone();
                                            match create_issue(&app.repo, &title, &body) {
                                                Ok(()) => {
                                                    app.issues = fetch_issues(&app.repo);
                                                    app.clamp_selected();
                                                    app.issue_modal = None;
                                                    app.mode = Mode::Normal;
                                                }
                                                Err(e) => {
                                                    modal.error = Some(e);
                                                }
                                            }
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if modal.active_field == 0 {
                                            modal.title.pop();
                                        } else {
                                            modal.body.pop();
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        if modal.active_field == 0 {
                                            modal.title.push(c);
                                        } else {
                                            modal.body.push(c);
                                        }
                                    }
                                    KeyCode::Enter if modal.active_field == 1 => {
                                        modal.body.push('\n');
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn fuzzy_match(query: &str, target: &str) -> bool {
    let target_lower = target.to_lowercase();
    let mut target_chars = target_lower.chars();
    for qc in query.to_lowercase().chars() {
        loop {
            match target_chars.next() {
                Some(tc) if tc == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

fn card_matches(card: &Card, query: &str) -> bool {
    fuzzy_match(query, &card.title) || fuzzy_match(query, &card.description)
}

fn ui_repo_select(frame: &mut Frame, state: &RepoSelectState) {
    let area = frame.area();

    // Center the content vertically
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Min(0),
            Constraint::Percentage(30),
        ])
        .split(area);

    // Center horizontally
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(vertical[1]);

    let center = horizontal[1];

    match state.phase {
        RepoSelectPhase::Typing => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(2),
                    Constraint::Min(0),
                ])
                .split(center);

            // Title
            let title = Paragraph::new(Line::from(vec![Span::styled(
                "Enter GitHub user or org:",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]))
            .block(Block::default());
            frame.render_widget(title, chunks[0]);

            // Input field
            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White))
                .title(" Owner ");
            let input_text = Paragraph::new(Line::from(vec![
                Span::styled(
                    &state.input,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("_", Style::default().fg(Color::Cyan)),
            ]))
            .block(input_block);
            frame.render_widget(input_text, chunks[1]);

            // Error message
            if let Some(err) = &state.error {
                let err_text = Paragraph::new(Line::from(vec![Span::styled(
                    err.as_str(),
                    Style::default().fg(Color::Red),
                )]));
                frame.render_widget(err_text, chunks[2]);
            }

            // Hint
            let hint = Paragraph::new(Line::from(vec![Span::styled(
                "Press Enter to fetch repos, Esc to go back",
                Style::default().fg(Color::DarkGray),
            )]));
            frame.render_widget(hint, chunks[3]);
        }
        RepoSelectPhase::Loading => {
            let loading = Paragraph::new(Line::from(vec![Span::styled(
                "Fetching repositories...",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]))
            .block(Block::default());
            frame.render_widget(loading, center);
        }
        RepoSelectPhase::Picking => {
            let max_visible = (center.height.saturating_sub(5)) as usize; // reserve space for header + filter
            let list_height = if max_visible > 0 { max_visible } else { 1 };

            let mut constraints = vec![
                Constraint::Length(1), // title
                Constraint::Length(1), // filter line
                Constraint::Length(1), // separator
            ];
            for _ in 0..list_height.min(state.filtered_repos.len()) {
                constraints.push(Constraint::Length(1));
            }
            constraints.push(Constraint::Min(0)); // hint at bottom

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(center);

            // Title
            let title = Paragraph::new(Line::from(vec![
                Span::styled(
                    "Select a repository",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ({} repos)", state.filtered_repos.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            frame.render_widget(title, chunks[0]);

            // Filter line
            let filter_line = if state.filter_query.is_empty() {
                Paragraph::new(Line::from(vec![Span::styled(
                    "Type to filter...",
                    Style::default().fg(Color::DarkGray),
                )]))
            } else {
                Paragraph::new(Line::from(vec![
                    Span::styled("/ ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        &state.filter_query,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("_", Style::default().fg(Color::Cyan)),
                ]))
            };
            frame.render_widget(filter_line, chunks[1]);

            // Separator
            let sep = Paragraph::new(Line::from(vec![Span::styled(
                "─".repeat(center.width as usize),
                Style::default().fg(Color::DarkGray),
            )]));
            frame.render_widget(sep, chunks[2]);

            // Scrolled repo list
            let scroll_offset = if state.selected >= list_height {
                state.selected - list_height + 1
            } else {
                0
            };

            let visible_count = list_height.min(state.filtered_repos.len());
            for i in 0..visible_count {
                let repo_idx = scroll_offset + i;
                if repo_idx >= state.filtered_repos.len() {
                    break;
                }
                let is_selected = repo_idx == state.selected;
                let repo_name = &state.filtered_repos[repo_idx];
                let line = if is_selected {
                    Line::from(vec![
                        Span::styled(" > ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            repo_name.as_str(),
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled("   ", Style::default()),
                        Span::styled(repo_name.as_str(), Style::default().fg(Color::Gray)),
                    ])
                };
                frame.render_widget(Paragraph::new(line), chunks[3 + i]);
            }

            // Hint at bottom
            let hint_idx = 3 + visible_count;
            if hint_idx < chunks.len() {
                let hint = Paragraph::new(Line::from(vec![Span::styled(
                    "↑/↓ navigate  Enter select  Esc back",
                    Style::default().fg(Color::DarkGray),
                )]));
                frame.render_widget(hint, chunks[hint_idx]);
            }
        }
    }
}

fn ui(frame: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    // Top bar — selected repository
    let repo_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Repository ");
    let repo_text = Paragraph::new(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            &app.repo,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  (Enter to change)",
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(repo_block);
    frame.render_widget(repo_text, outer[0]);

    // Four columns
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(outer[1]);

    let section_data: [(&str, Color, &[Card]); 4] = [
        (" Issues ", Color::Red, &app.issues),
        (" Worktrees ", Color::Yellow, &app.worktrees),
        (" Pull Requests ", Color::Magenta, &app.pull_requests),
        (" Sessions ", Color::Blue, &app.sessions),
    ];

    let filter_query = match &app.mode {
        Mode::Filtering { query } => Some(query.as_str()),
        _ => None,
    };

    let related_ids = app.selected_card_related_ids();

    for (i, (title, color, cards)) in section_data.iter().enumerate() {
        let is_active = i == app.active_section;
        let query = if is_active { filter_query } else { None };
        let selected = if is_active {
            Some(app.selected_card[i])
        } else {
            None
        };
        render_column(
            frame,
            columns[i],
            title,
            *color,
            cards,
            is_active,
            query,
            selected,
            &related_ids,
        );
    }

    // Bottom legend bar
    let key_style = Style::default()
        .fg(Color::White)
        .bg(Color::Rgb(60, 60, 60))
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::Gray);
    let key_accent = Style::default()
        .fg(Color::Black)
        .bg(Color::Green)
        .add_modifier(Modifier::BOLD);

    let mut legend_spans: Vec<Span> = Vec::new();

    // Status message prefix
    if let Some(msg) = &app.status_message {
        legend_spans.push(Span::styled(
            format!(" {} ", msg),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        legend_spans.push(Span::styled(" | ", desc_style));
    }

    let mode_spans: Vec<Span> = match &app.mode {
        Mode::Normal => {
            let mut spans = vec![
                Span::styled(" q/Esc ", key_style),
                Span::styled(" Quit ", desc_style),
                Span::styled(" Tab/S-Tab ", key_style),
                Span::styled(" Switch column ", desc_style),
                Span::styled(" ↑/↓ ", key_style),
                Span::styled(" Navigate ", desc_style),
                Span::styled(" / ", key_style),
                Span::styled(" Filter ", desc_style),
                Span::styled(" Enter ", key_style),
                Span::styled(" Change repo ", desc_style),
            ];
            if app.active_section == 0 {
                spans.push(Span::styled(" n ", key_accent));
                spans.push(Span::styled(" New issue ", desc_style));
                spans.push(Span::styled(" w ", key_accent));
                spans.push(Span::styled(" Worktree+Claude ", desc_style));
                spans.push(Span::styled(" d ", key_style));
                spans.push(Span::styled(" Close issue ", desc_style));
            }
            if app.active_section == 1 {
                spans.push(Span::styled(" d ", key_style));
                spans.push(Span::styled(" Remove worktree ", desc_style));
            }
            spans
        }
        Mode::Filtering { .. } => vec![
            Span::styled(" Esc ", key_style),
            Span::styled(" Clear filter ", desc_style),
            Span::styled(" ↑/↓ ", key_style),
            Span::styled(" Navigate ", desc_style),
        ],
        Mode::CreatingIssue => vec![
            Span::styled(" Esc ", key_style),
            Span::styled(" Cancel ", desc_style),
            Span::styled(" Tab ", key_style),
            Span::styled(" Switch field ", desc_style),
            Span::styled(" Ctrl+S ", key_accent),
            Span::styled(" Submit ", desc_style),
        ],
        Mode::Confirming => vec![
            Span::styled(" y ", key_accent),
            Span::styled(" Confirm ", desc_style),
            Span::styled(" n/Esc ", key_style),
            Span::styled(" Cancel ", desc_style),
        ],
    };
    legend_spans.extend(mode_spans);

    let legend = Paragraph::new(Line::from(legend_spans));
    frame.render_widget(legend, outer[2]);

    // Render issue modal overlay if open
    if let Some(modal) = &app.issue_modal {
        ui_issue_modal(frame, modal);
    }

    // Render confirm modal overlay if open
    if let Some(modal) = &app.confirm_modal {
        ui_confirm_modal(frame, modal);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn ui_issue_modal(frame: &mut Frame, modal: &IssueModal) {
    let area = centered_rect(50, 50, frame.area());

    frame.render_widget(Clear, area);

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" New Issue ")
        .title_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .padding(Padding::new(1, 1, 1, 0));
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    // Layout: title field (3), body field (remaining), error (1), hint (1)
    let has_error = modal.error.is_some();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title input
            Constraint::Min(3),   // body input
            Constraint::Length(if has_error { 1 } else { 0 }), // error
            Constraint::Length(1), // hint
        ])
        .split(inner);

    // Title field
    let title_border_style = if modal.active_field == 0 {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let title_block = Block::default()
        .borders(Borders::ALL)
        .border_style(title_border_style)
        .title(" Title ");
    let title_text = Paragraph::new(Line::from(vec![
        Span::styled(&modal.title, Style::default().fg(Color::White)),
        if modal.active_field == 0 {
            Span::styled("_", Style::default().fg(Color::Cyan))
        } else {
            Span::raw("")
        },
    ]))
    .block(title_block);
    frame.render_widget(title_text, chunks[0]);

    // Body field
    let body_border_style = if modal.active_field == 1 {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let body_block = Block::default()
        .borders(Borders::ALL)
        .border_style(body_border_style)
        .title(" Body ");
    let mut body_text = modal.body.clone();
    if modal.active_field == 1 {
        body_text.push('_');
    }
    let body_paragraph = Paragraph::new(body_text)
        .style(Style::default().fg(Color::White))
        .block(body_block);
    frame.render_widget(body_paragraph, chunks[1]);

    // Error
    if let Some(err) = &modal.error {
        let err_text = Paragraph::new(Line::from(vec![Span::styled(
            err.as_str(),
            Style::default().fg(Color::Red),
        )]));
        frame.render_widget(err_text, chunks[2]);
    }

    // Hint
    let hint = Paragraph::new(Line::from(vec![Span::styled(
        "Tab: switch field | Ctrl+S: submit | Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )]));
    frame.render_widget(hint, chunks[3]);
}

fn ui_confirm_modal(frame: &mut Frame, modal: &ConfirmModal) {
    let area = centered_rect(50, 20, frame.area());

    frame.render_widget(Clear, area);

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Confirm ")
        .title_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )
        .padding(Padding::new(1, 1, 1, 0));
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let message = Paragraph::new(modal.message.as_str())
        .style(Style::default().fg(Color::White));
    frame.render_widget(message, chunks[0]);

    let hint = Paragraph::new(Line::from(vec![
        Span::styled("y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" confirm  ", Style::default().fg(Color::DarkGray)),
        Span::styled("n", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(hint, chunks[1]);
}

fn render_column(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    color: Color,
    cards: &[Card],
    is_active: bool,
    filter_query: Option<&str>,
    selected: Option<usize>,
    related_ids: &HashSet<String>,
) {
    let border_style = if is_active {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    let col_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title)
        .title_style(if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        })
        .padding(Padding::new(1, 1, 1, 0));
    let inner = col_block.inner(area);
    frame.render_widget(col_block, area);

    // Determine content area — if filtering, reserve a line for the search input
    let (cards_area, filter_area) = if let Some(_) = filter_query {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(inner);
        (split[1], Some(split[0]))
    } else {
        (inner, None)
    };

    // Render filter input if active
    if let (Some(area), Some(query)) = (filter_area, filter_query) {
        let input = Paragraph::new(Line::from(vec![
            Span::styled("/ ", Style::default().fg(Color::Cyan)),
            Span::styled(
                query,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("_", Style::default().fg(Color::Cyan)),
        ]));
        frame.render_widget(input, area);
    }

    // Filter cards
    let visible_cards: Vec<&Card> = if let Some(query) = filter_query {
        if query.is_empty() {
            cards.iter().collect()
        } else {
            cards.iter().filter(|c| card_matches(c, query)).collect()
        }
    } else {
        cards.iter().collect()
    };

    let card_height = 4u16;
    let mut constraints: Vec<Constraint> = visible_cards
        .iter()
        .map(|_| Constraint::Length(card_height))
        .collect();
    constraints.push(Constraint::Min(0));

    let slots = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(cards_area);

    for (i, card) in visible_cards.iter().enumerate() {
        let is_selected = selected.is_some_and(|s| s == i);
        let is_related = !is_selected && related_ids.contains(&card.id);
        render_card(frame, slots[i], card, is_selected, is_related);
    }
}

fn render_card(frame: &mut Frame, area: Rect, card: &Card, is_selected: bool, is_related: bool) {
    let border_style = if is_selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if is_related {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let card_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = card_block.inner(area);
    frame.render_widget(card_block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let lines = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    // Title line with tag
    let tag = Span::styled(
        format!(" {} ", card.tag),
        Style::default().fg(Color::Black).bg(card.tag_color),
    );
    let title = Span::styled(
        format!(" {}", card.title),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(Paragraph::new(Line::from(vec![tag, title])), lines[0]);

    // Description
    let desc = Paragraph::new(Span::styled(
        &card.description,
        Style::default().fg(Color::Gray),
    ));
    frame.render_widget(desc, lines[1]);
}
