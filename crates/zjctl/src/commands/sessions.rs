//! Session management commands

use crate::output;
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Serialize)]
struct SessionInfo {
    name: String,
}

pub fn ls(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let sessions = list_sessions()?;

    if json {
        output::print_success(&sessions);
    } else if sessions.is_empty() {
        println!("No active sessions");
    } else {
        println!("{:<30}", "SESSION");
        println!("{}", "-".repeat(30));
        for session in &sessions {
            println!("{:<30}", session.name);
        }
    }

    Ok(())
}

pub fn create(name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    // `zellij --session <name>` with detach creates a new session.
    // We use `zellij attach --create <name>` which creates if it doesn't exist,
    // combined with a force-detach approach using the `zellij kill-session` pattern.
    // The simplest portable approach: use `zellij attach --create` in the background.
    let status = Command::new("zellij")
        .args(["attach", name, "--create", "--force-run-commands"])
        .env("ZELLIJ_AUTO_EXIT", "true")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if status.success() {
        if json {
            output::print_success(serde_json::json!({
                "session": name,
                "created": true,
            }));
        } else {
            println!("session created: {name}");
        }
        Ok(())
    } else {
        Err(format!("failed to create session '{name}'").into())
    }
}

pub fn kill(name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("zellij")
        .args(["kill-session", name])
        .output()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if output.status.success() {
        if json {
            output::print_success(serde_json::json!({
                "session": name,
                "killed": true,
            }));
        } else {
            println!("session killed: {name}");
        }
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("failed to kill session '{}': {}", name, stderr.trim()).into())
    }
}

fn list_sessions() -> Result<Vec<SessionInfo>, Box<dyn std::error::Error>> {
    let output = Command::new("zellij")
        .args(["list-sessions", "--short"])
        .output()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Zellij returns non-zero when no sessions exist
        if stderr.contains("No active") || stderr.trim().is_empty() {
            return Ok(Vec::new());
        }
        return Err(format!("zellij list-sessions failed: {}", stderr.trim()).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sessions: Vec<SessionInfo> = stdout
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            // `zellij list-sessions --short` outputs just session names, one per line.
            // Without --short, it may include metadata — we take the first word.
            let name = line.split_whitespace().next().unwrap_or(line);
            SessionInfo {
                name: name.to_string(),
            }
        })
        .collect();

    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_list_output() {
        // Simulating what list_sessions does with raw output
        let output = "my-session\nwork-session\ntest\n";
        let sessions: Vec<SessionInfo> = output
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(|line| {
                let name = line.split_whitespace().next().unwrap_or(line);
                SessionInfo {
                    name: name.to_string(),
                }
            })
            .collect();

        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].name, "my-session");
        assert_eq!(sessions[1].name, "work-session");
        assert_eq!(sessions[2].name, "test");
    }

    #[test]
    fn parse_empty_session_list() {
        let output = "";
        let sessions: Vec<SessionInfo> = output
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(|line| SessionInfo {
                name: line.to_string(),
            })
            .collect();

        assert!(sessions.is_empty());
    }

    #[test]
    fn session_info_serializes_to_json() {
        let session = SessionInfo {
            name: "test-session".to_string(),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("test-session"));
    }
}
