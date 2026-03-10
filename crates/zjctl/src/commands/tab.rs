//! Tab management commands

use serde::{Deserialize, Serialize};

use crate::client;
use crate::output;
use crate::zellij;
use zjctl_proto::methods;

/// Tab info returned from tabs.list
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TabInfo {
    pub index: usize,
    pub name: String,
    pub active: bool,
}

pub fn ls(plugin: Option<&str>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let tabs = list(plugin)?;
    if json {
        output::print_success(&tabs);
    } else if tabs.is_empty() {
        println!("No tabs found");
    } else {
        println!("{:<8} {:<30} {:<8}", "INDEX", "NAME", "ACTIVE");
        println!("{}", "-".repeat(48));
        for tab in &tabs {
            println!(
                "{:<8} {:<30} {:<8}",
                tab.index,
                tab.name,
                if tab.active { "*" } else { "" }
            );
        }
    }
    Ok(())
}

pub fn list(plugin: Option<&str>) -> Result<Vec<TabInfo>, Box<dyn std::error::Error>> {
    let result = client::rpc_call(plugin, methods::TABS_LIST, serde_json::json!({}))?;
    let tabs: Vec<TabInfo> = serde_json::from_value(result)?;
    Ok(tabs)
}

pub fn new(name: Option<&str>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = zellij::command();
    cmd.args(["action", "new-tab"]);
    if let Some(name) = name {
        cmd.args(["--name", name]);
    }

    let status = cmd
        .status()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if status.success() {
        if json {
            output::print_success(serde_json::json!({
                "created": true,
                "name": name,
            }));
        } else {
            match name {
                Some(n) => println!("tab created: {n}"),
                None => println!("tab created"),
            }
        }
        Ok(())
    } else {
        Err("zellij action new-tab failed".into())
    }
}

pub fn focus(target: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Try as index first, then as name
    if let Ok(index) = target.parse::<usize>() {
        let status = zellij::command()
            .args(["action", "go-to-tab", &(index + 1).to_string()])
            .status()
            .map_err(|err| format!("failed to run zellij: {err}"))?;

        if status.success() {
            if json {
                output::print_success(serde_json::json!({
                    "focused": true,
                    "index": index,
                }));
            }
            return Ok(());
        }
        return Err(format!("failed to focus tab index {index}").into());
    }

    // Name-based: go-to-tab-name
    let status = zellij::command()
        .args(["action", "go-to-tab-name", target])
        .status()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if status.success() {
        if json {
            output::print_success(serde_json::json!({
                "focused": true,
                "name": target,
            }));
        }
        Ok(())
    } else {
        Err(format!("failed to focus tab '{target}'").into())
    }
}

pub fn rename(index: usize, name: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Focus the tab first, then rename
    let status = zellij::command()
        .args(["action", "go-to-tab", &(index + 1).to_string()])
        .status()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if !status.success() {
        return Err(format!("failed to focus tab index {index}").into());
    }

    let status = zellij::command()
        .args(["action", "rename-tab", name])
        .status()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if status.success() {
        if json {
            output::print_success(serde_json::json!({
                "renamed": true,
                "index": index,
                "name": name,
            }));
        } else {
            println!("tab {index} renamed to: {name}");
        }
        Ok(())
    } else {
        Err(format!("failed to rename tab {index}").into())
    }
}

pub fn close(index: usize, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Focus the tab first, then close
    let status = zellij::command()
        .args(["action", "go-to-tab", &(index + 1).to_string()])
        .status()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if !status.success() {
        return Err(format!("failed to focus tab index {index}").into());
    }

    let status = zellij::command()
        .args(["action", "close-tab"])
        .status()
        .map_err(|err| format!("failed to run zellij: {err}"))?;

    if status.success() {
        if json {
            output::print_success(serde_json::json!({
                "closed": true,
                "index": index,
            }));
        } else {
            println!("tab {index} closed");
        }
        Ok(())
    } else {
        Err(format!("failed to close tab {index}").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_info_serializes_to_json() {
        let tab = TabInfo {
            index: 0,
            name: "Tab 1".to_string(),
            active: true,
        };
        let json = serde_json::to_string(&tab).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["index"], 0);
        assert_eq!(parsed["name"], "Tab 1");
        assert_eq!(parsed["active"], true);
    }

    #[test]
    fn tab_info_deserializes_from_rpc() {
        let json =
            r#"[{"index":0,"name":"tab1","active":true},{"index":1,"name":"tab2","active":false}]"#;
        let tabs: Vec<TabInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].name, "tab1");
        assert!(tabs[0].active);
        assert!(!tabs[1].active);
    }
}
