use std::fs;
use std::path::PathBuf;

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

use crate::models::Card;

#[derive(Serialize, Deserialize, Clone)]
pub struct LocalIssue {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub state: String, // "open" or "closed"
}

#[derive(Serialize, Deserialize, Default)]
struct LocalIssueStore {
    next_id: u64,
    issues: Vec<LocalIssue>,
}

fn store_path(repo: &str) -> PathBuf {
    let slug = repo.replace('/', "-");
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("octopai")
        .join("local_issues")
        .join(format!("{}.json", slug))
}

fn load_store(repo: &str) -> LocalIssueStore {
    let path = store_path(repo);
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return LocalIssueStore::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_store(repo: &str, store: &LocalIssueStore) -> Result<(), String> {
    let path = store_path(repo);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    let json =
        serde_json::to_string_pretty(store).map_err(|e| format!("Failed to serialize: {}", e))?;
    fs::write(path, json).map_err(|e| format!("Failed to write: {}", e))
}

pub fn fetch_local_issues(repo: &str, state_filter: &str) -> Vec<Card> {
    let store = load_store(repo);
    store
        .issues
        .iter()
        .filter(|issue| issue.state == state_filter)
        .map(|issue| {
            let description = if issue.body.len() > 80 {
                format!("{}...", &issue.body[..77])
            } else if issue.body.is_empty() {
                "No description".to_string()
            } else {
                issue.body.clone()
            };
            let full_description = if issue.body.is_empty() {
                None
            } else {
                Some(issue.body.clone())
            };
            Card {
                id: format!("local-{}", issue.id),
                title: format!("L-{} {}", issue.id, issue.title),
                description,
                full_description,
                tag: "local".to_string(),
                tag_color: Color::Cyan,
                related: Vec::new(),
                url: None,
                pr_number: None,
                is_draft: None,
                is_merged: None,
                head_branch: None,
                is_local: true,
            }
        })
        .collect()
}

pub fn create_local_issue(repo: &str, title: &str, body: &str) -> Result<u64, String> {
    let mut store = load_store(repo);
    store.next_id += 1;
    let id = store.next_id;
    store.issues.push(LocalIssue {
        id,
        title: title.to_string(),
        body: body.to_string(),
        state: "open".to_string(),
    });
    save_store(repo, &store)?;
    Ok(id)
}

pub fn edit_local_issue(repo: &str, id: u64, title: &str, body: &str) -> Result<(), String> {
    let mut store = load_store(repo);
    if let Some(issue) = store.issues.iter_mut().find(|i| i.id == id) {
        issue.title = title.to_string();
        issue.body = body.to_string();
        save_store(repo, &store)
    } else {
        Err(format!("Local issue L-{} not found", id))
    }
}

pub fn close_local_issue(repo: &str, id: u64) -> Result<(), String> {
    let mut store = load_store(repo);
    if let Some(issue) = store.issues.iter_mut().find(|i| i.id == id) {
        issue.state = "closed".to_string();
        save_store(repo, &store)
    } else {
        Err(format!("Local issue L-{} not found", id))
    }
}

pub fn fetch_local_issue(repo: &str, id: u64) -> Result<(String, String), String> {
    let store = load_store(repo);
    if let Some(issue) = store.issues.iter().find(|i| i.id == id) {
        Ok((issue.title.clone(), issue.body.clone()))
    } else {
        Err(format!("Local issue L-{} not found", id))
    }
}
