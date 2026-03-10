//! Plugin state management - tracks panes and tabs from Zellij events

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zellij_tile::prelude::*;

/// Plugin state tracking panes and tabs
#[derive(Default)]
pub struct PluginState {
    /// All known panes, keyed by a unique string ID
    pub panes: HashMap<String, PaneEntry>,
    /// Tab information
    pub tabs: Vec<TabEntry>,
    /// Focused pane of the current client (if known)
    pub current_client_pane_id: Option<PaneId>,
    /// Per-pane tags, keyed by pane ID string. Stored separately so tags
    /// survive the clear+rebuild cycle in `update_panes`.
    pub tags: HashMap<String, HashMap<String, String>>,
}

/// Information about a single pane
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneEntry {
    /// Numeric pane ID
    pub numeric_id: u32,
    /// Whether this is a plugin pane (vs terminal)
    pub is_plugin: bool,
    /// Pane title
    pub title: String,
    /// Command running in pane (for terminals)
    pub command: Option<String>,
    /// Tab index this pane belongs to
    pub tab_index: usize,
    /// Tab name
    pub tab_name: String,
    /// Whether this pane is focused
    pub focused: bool,
    /// Whether this is a floating pane
    pub floating: bool,
    /// Whether this pane is suppressed
    pub suppressed: bool,
    /// Pane content rows (terminal size)
    pub rows: usize,
    /// Pane content columns (terminal size)
    pub cols: usize,
    /// Whether the pane's process has exited
    pub exited: bool,
    /// Exit status of the pane's process (None if still running)
    pub exit_status: Option<i32>,
}

impl PaneEntry {
    /// Get string ID for this pane
    pub fn id_string(&self) -> String {
        if self.is_plugin {
            format!("plugin:{}", self.numeric_id)
        } else {
            format!("terminal:{}", self.numeric_id)
        }
    }

    /// Get the Zellij PaneId for this pane
    pub fn pane_id(&self) -> PaneId {
        if self.is_plugin {
            PaneId::Plugin(self.numeric_id)
        } else {
            PaneId::Terminal(self.numeric_id)
        }
    }
}

/// Information about a tab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabEntry {
    pub index: usize,
    pub name: String,
    pub active: bool,
}

impl PluginState {
    /// Update pane state from PaneUpdate event
    pub fn update_panes(&mut self, manifest: PaneManifest) {
        self.panes.clear();

        for (tab_index, panes) in manifest.panes {
            let tab_name = self
                .tabs
                .get(tab_index)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| format!("Tab {}", tab_index));

            for pane in panes {
                let entry = PaneEntry {
                    numeric_id: pane.id,
                    is_plugin: pane.is_plugin,
                    title: pane.title.clone(),
                    command: pane.terminal_command.clone(),
                    tab_index,
                    tab_name: tab_name.clone(),
                    focused: pane.is_focused,
                    floating: pane.is_floating,
                    suppressed: pane.is_suppressed,
                    rows: pane.pane_content_rows,
                    cols: pane.pane_content_columns,
                    exited: pane.exited,
                    exit_status: pane.exit_status,
                };
                let key = entry.id_string();
                self.panes.insert(key, entry);
            }
        }
    }

    /// Update tab state from TabUpdate event
    pub fn update_tabs(&mut self, tabs: Vec<TabInfo>) {
        let max_position = tabs.iter().map(|t| t.position).max().unwrap_or(0);
        let mut entries: Vec<Option<TabEntry>> = vec![None; max_position.saturating_add(1)];

        for tab in tabs {
            entries[tab.position] = Some(TabEntry {
                index: tab.position,
                name: tab.name,
                active: tab.active,
            });
        }

        self.tabs = entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                entry.unwrap_or(TabEntry {
                    index,
                    name: format!("Tab {}", index),
                    active: false,
                })
            })
            .collect();
    }

    pub fn update_clients(&mut self, clients: Vec<ClientInfo>) {
        if clients.is_empty() {
            self.current_client_pane_id = None;
            return;
        }

        // Prefer Zellij's "current client" marker when it exists, otherwise fall back to a
        // deterministic client (helps in single-client sessions where is_current_client can be
        // false for all entries when queried from a background plugin).
        self.current_client_pane_id = clients
            .iter()
            .find(|c| c.is_current_client)
            .or_else(|| clients.iter().min_by_key(|c| c.client_id))
            .map(|c| c.pane_id);
    }

    pub fn active_tab_index(&self) -> Option<usize> {
        self.tabs.iter().find(|t| t.active).map(|t| t.index)
    }

    /// Set a tag on a pane
    pub fn set_tag(&mut self, pane_id: &str, key: String, value: String) {
        self.tags
            .entry(pane_id.to_string())
            .or_default()
            .insert(key, value);
    }

    /// Remove a tag from a pane
    pub fn remove_tag(&mut self, pane_id: &str, key: &str) -> bool {
        if let Some(tags) = self.tags.get_mut(pane_id) {
            let removed = tags.remove(key).is_some();
            if tags.is_empty() {
                self.tags.remove(pane_id);
            }
            removed
        } else {
            false
        }
    }

    /// Get tags for a pane
    pub fn get_tags(&self, pane_id: &str) -> HashMap<String, String> {
        self.tags.get(pane_id).cloned().unwrap_or_default()
    }

    /// List all panes for the panes.list command
    pub fn list_panes(&self, focused_id: Option<&str>) -> Vec<PaneListItem> {
        self.panes
            .values()
            .map(|p| {
                let id = p.id_string();
                let tags = self.get_tags(&id);
                PaneListItem {
                    focused: focused_id == Some(id.as_str()),
                    id,
                    pane_type: if p.is_plugin { "plugin" } else { "terminal" }.to_string(),
                    title: p.title.clone(),
                    command: p.command.clone(),
                    tab_index: p.tab_index,
                    tab_name: p.tab_name.clone(),
                    floating: p.floating,
                    suppressed: p.suppressed,
                    rows: p.rows,
                    cols: p.cols,
                    exit_status: p.exit_status,
                    tags,
                }
            })
            .collect()
    }
}

/// Pane info for list response — implements [[RFC-0001:C-PANES-LIST]]
#[derive(Debug, Serialize, Deserialize)]
pub struct PaneListItem {
    pub id: String,
    pub pane_type: String,
    pub title: String,
    pub command: Option<String>,
    pub tab_index: usize,
    pub tab_name: String,
    pub focused: bool,
    pub floating: bool,
    pub suppressed: bool,
    pub rows: usize,
    pub cols: usize,
    pub exit_status: Option<i32>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
}
