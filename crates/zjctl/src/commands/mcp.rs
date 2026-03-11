//! MCP server for remote agent access — per [[ADR-0008]]
//!
//! Exposes 6 tools (pane_launch, pane_send, pane_read, pane_wait, pane_close, panes_list)
//! and read-only resources over stdio using the rmcp crate.

use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{Error as McpError, RoleServer, ServerHandler, tool};
use serde::Deserialize;

use crate::client;
use crate::commands::{pane, panes};
use zjctl_proto::methods;

/// MCP server state — per [[ADR-0008]], wraps existing command functions.
#[derive(Clone)]
pub struct ZjctlMcp {
    plugin: Option<String>,
}

// --- Tool parameter types ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PaneLaunchParams {
    /// Command to run in the new pane (e.g. "zsh", "cargo build")
    #[schemars(description = "Command to run in the new pane")]
    pub command: Option<String>,
    /// Direction to open the pane (right, down)
    #[schemars(description = "Direction: right or down")]
    pub direction: Option<String>,
    /// Name for the new pane
    #[schemars(description = "Name for the new pane")]
    pub name: Option<String>,
    /// Working directory
    #[schemars(description = "Working directory for the pane")]
    pub cwd: Option<String>,
    /// Open as floating pane
    #[schemars(description = "Open as floating pane")]
    pub floating: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PaneSendParams {
    /// Pane selector (e.g. "id:terminal:3", "focused", "title:server")
    #[schemars(description = "Pane selector (e.g. id:terminal:3, focused, title:server)")]
    pub pane: String,
    /// Text to send to the pane's stdin
    #[schemars(description = "Text to send to the pane stdin. Use \\n for Enter.")]
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PaneReadParams {
    /// Pane selector
    #[schemars(description = "Pane selector (e.g. id:terminal:3, focused, title:server)")]
    pub pane: String,
    /// Optional search pattern (plain text or /regex/)
    #[schemars(description = "Search pattern. Plain text for substring, /pattern/ for regex. Omit to get full output.")]
    pub pattern: Option<String>,
    /// Include full scrollback history
    #[schemars(description = "Include full scrollback history (default: false, viewport only)")]
    pub full: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PaneWaitParams {
    /// Pane selector
    #[schemars(description = "Pane selector (e.g. id:terminal:3, focused, title:server)")]
    pub pane: String,
    /// Wait mode: "idle" (output stops changing) or "exit" (process exits)
    #[schemars(description = "Wait mode: idle (output settles) or exit (process exits)")]
    pub mode: String,
    /// Timeout in seconds
    #[schemars(description = "Timeout in seconds (default: 30)")]
    pub timeout: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PaneCloseParams {
    /// Pane selector
    #[schemars(description = "Pane selector (e.g. id:terminal:3, title:server)")]
    pub pane: String,
    /// Force close even if focused
    #[schemars(description = "Force close even if the pane is focused")]
    pub force: Option<bool>,
}

// --- Tool implementations ---

#[tool(tool_box)]
impl ZjctlMcp {
    pub fn new(plugin: Option<String>) -> Self {
        Self { plugin }
    }

    fn plugin(&self) -> Option<&str> {
        self.plugin.as_deref()
    }

    #[tool(description = "Launch a new terminal pane and return its selector. Use this to create panes for running commands.")]
    fn pane_launch(
        &self,
        #[tool(aggr)] params: PaneLaunchParams,
    ) -> Result<CallToolResult, McpError> {
        let command_vec: Vec<String> = params
            .command
            .map(|c| vec![c])
            .unwrap_or_default();

        let options = pane::LaunchOptions {
            direction: params.direction.as_deref(),
            floating: params.floating.unwrap_or(false),
            name: params.name.as_deref(),
            cwd: params.cwd.as_deref(),
            close_on_exit: false,
            in_place: false,
            start_suspended: false,
            command: &command_vec,
        };

        // Use the same launch logic as CLI, but capture the result instead of printing
        let before = panes::list(self.plugin()).map_err(|e| mcp_err(&e))?;
        let focused_tab_index = before.iter().find(|p| p.focused).map(|p| p.tab_index);
        let before_max_terminal_id = before
            .iter()
            .filter_map(|p| parse_terminal_id(&p.id))
            .max()
            .unwrap_or(0);

        // Run the zellij action to create the pane
        run_new_pane_action(&options).map_err(|e| mcp_err(&e))?;

        // Poll for the new pane
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(180);
        let interval = std::time::Duration::from_millis(50);

        let new_pane = loop {
            let after = panes::list(self.plugin()).map_err(|e| mcp_err(&e))?;
            if let Some(p) = find_new_pane(&after, focused_tab_index, &options, before_max_terminal_id) {
                break p;
            }
            if start.elapsed() >= timeout {
                return Ok(CallToolResult {
                    content: vec![Content::text("Error: unable to identify new pane after launch")],
                    is_error: Some(true),
                });
            }
            std::thread::sleep(interval);
        };

        let selector = pane_id_to_selector(&new_pane.id).unwrap_or_else(|| new_pane.id.clone());
        let result = serde_json::json!({
            "selector": selector,
            "pane_id": new_pane.id,
        });
        Ok(CallToolResult {
            content: vec![Content::text(serde_json::to_string_pretty(&result).unwrap())],
            is_error: None,
        })
    }

    #[tool(description = "Send text to a pane's stdin. Use \\n for Enter, \\x03 for Ctrl+C, \\x1b for Escape.")]
    fn pane_send(
        &self,
        #[tool(aggr)] params: PaneSendParams,
    ) -> Result<CallToolResult, McpError> {
        let rpc_params = serde_json::json!({
            "selector": params.pane,
            "all": false,
            "text": params.text,
        });
        client::rpc_call(self.plugin(), methods::PANE_SEND, rpc_params)
            .map_err(|e| mcp_err(&e))?;

        Ok(CallToolResult {
            content: vec![Content::text(format!("Sent to {}", params.pane))],
            is_error: None,
        })
    }

    #[tool(description = "Read pane output. Without pattern: returns full visible content. With pattern: returns matching lines (use /regex/ for regex). Use full=true for scrollback.")]
    fn pane_read(
        &self,
        #[tool(aggr)] params: PaneReadParams,
    ) -> Result<CallToolResult, McpError> {
        let full = params.full.unwrap_or(false);

        if let Some(pattern) = &params.pattern {
            // Grep mode
            let (kind, value) = if pattern.starts_with('/') && pattern.ends_with('/') && pattern.len() > 2 {
                ("regex", &pattern[1..pattern.len() - 1])
            } else {
                ("substring", pattern.as_str())
            };

            let rpc_params = serde_json::json!({
                "selector": params.pane,
                "pattern": { "kind": kind, "value": value },
                "full": full,
            });
            let result = client::rpc_call(self.plugin(), methods::PANE_SEARCH, rpc_params)
                .map_err(|e| mcp_err(&e))?;

            let output = serde_json::to_string_pretty(&result).unwrap();
            Ok(CallToolResult {
                content: vec![Content::text(output)],
                is_error: None,
            })
        } else {
            // Capture mode
            let rpc_params = serde_json::json!({
                "selector": params.pane,
                "full": full,
            });
            let result = client::rpc_call(self.plugin(), methods::PANE_CAPTURE, rpc_params)
                .map_err(|e| mcp_err(&e))?;

            let content = result["content"].as_str().unwrap_or("");
            Ok(CallToolResult {
                content: vec![Content::text(content.to_string())],
                is_error: None,
            })
        }
    }

    #[tool(description = "Wait for a pane. mode=idle: wait until output stops changing. mode=exit: wait until the process exits and return exit code.")]
    fn pane_wait(
        &self,
        #[tool(aggr)] params: PaneWaitParams,
    ) -> Result<CallToolResult, McpError> {
        let timeout = params.timeout.unwrap_or(30.0);

        match params.mode.as_str() {
            "idle" => {
                pane::wait_idle(self.plugin(), &params.pane, 2.0, timeout, false, true, false)
                    .map_err(|e| mcp_err(&e))?;
                Ok(CallToolResult {
                    content: vec![Content::text(format!("{}: output is idle", params.pane))],
                    is_error: None,
                })
            }
            "exit" => {
                pane::wait_exit(self.plugin(), &params.pane, timeout, false)
                    .map_err(|e| mcp_err(&e))?;
                // After wait_exit returns, get the exit status
                let result = client::rpc_call(
                    self.plugin(),
                    methods::PANE_STATUS,
                    serde_json::json!({ "selector": params.pane }),
                ).map_err(|e| mcp_err(&e))?;
                let output = serde_json::to_string_pretty(&result).unwrap();
                Ok(CallToolResult {
                    content: vec![Content::text(output)],
                    is_error: None,
                })
            }
            _ => Ok(CallToolResult {
                content: vec![Content::text("Error: mode must be 'idle' or 'exit'")],
                is_error: Some(true),
            }),
        }
    }

    #[tool(description = "Close a pane. Use force=true if the pane is currently focused.")]
    fn pane_close(
        &self,
        #[tool(aggr)] params: PaneCloseParams,
    ) -> Result<CallToolResult, McpError> {
        let force = params.force.unwrap_or(false);
        pane::close(self.plugin(), &params.pane, force, false)
            .map_err(|e| mcp_err(&e))?;
        Ok(CallToolResult {
            content: vec![Content::text(format!("Closed {}", params.pane))],
            is_error: None,
        })
    }

    #[tool(description = "List all panes in the current Zellij session with their IDs, titles, running status, and tags.")]
    fn panes_list(&self) -> Result<CallToolResult, McpError> {
        let pane_list = panes::list(self.plugin()).map_err(|e| mcp_err(&e))?;
        let output = serde_json::to_string_pretty(&pane_list).unwrap();
        Ok(CallToolResult {
            content: vec![Content::text(output)],
            is_error: None,
        })
    }
}

// --- ServerHandler implementation ---

#[tool(tool_box)]
impl ServerHandler for ZjctlMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "zjctl".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some(
                "zjctl MCP server — control Zellij terminal panes. \
                 Use pane_launch to create panes, pane_send to write, \
                 pane_read to capture output, pane_wait to wait for \
                 completion, pane_close to clean up."
                    .into(),
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: PaginatedRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                RawResource {
                    uri: "zjctl://panes".into(),
                    name: "Pane list".into(),
                    description: Some("All panes in the current Zellij session".into()),
                    mime_type: Some("application/json".into()),
                    size: None,
                }
                .no_annotation(),
                RawResource {
                    uri: "zjctl://tabs".into(),
                    name: "Tab list".into(),
                    description: Some("All tabs in the current Zellij session".into()),
                    mime_type: Some("application/json".into()),
                    size: None,
                }
                .no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: PaginatedRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![
                RawResourceTemplate {
                    uri_template: "zjctl://pane/{pane_id}/output".into(),
                    name: "Pane output".into(),
                    description: Some("Visible content of a specific pane".into()),
                    mime_type: Some("text/plain".into()),
                }
                .no_annotation(),
                RawResourceTemplate {
                    uri_template: "zjctl://pane/{pane_id}/scrollback".into(),
                    name: "Pane scrollback".into(),
                    description: Some("Full scrollback history of a specific pane".into()),
                    mime_type: Some("text/plain".into()),
                }
                .no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;

        if uri == "zjctl://panes" {
            let pane_list = panes::list(self.plugin()).map_err(|e| mcp_err(&e))?;
            let json = serde_json::to_string_pretty(&pane_list).unwrap();
            return Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(json, uri.clone())],
            });
        }

        if uri == "zjctl://tabs" {
            let result = client::rpc_call(
                self.plugin(),
                methods::TABS_LIST,
                serde_json::json!({}),
            ).map_err(|e| mcp_err(&e))?;
            let json = serde_json::to_string_pretty(&result).unwrap();
            return Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(json, uri.clone())],
            });
        }

        // zjctl://pane/{pane_id}/output or zjctl://pane/{pane_id}/scrollback
        if let Some(rest) = uri.strip_prefix("zjctl://pane/") {
            let (pane_id, kind) = if let Some(id) = rest.strip_suffix("/output") {
                (id, false)
            } else if let Some(id) = rest.strip_suffix("/scrollback") {
                (id, true)
            } else {
                return Err(McpError::resource_not_found(
                    format!("unknown resource: {uri}"),
                    None,
                ));
            };

            let selector = format!("id:{pane_id}");
            let rpc_params = serde_json::json!({
                "selector": selector,
                "full": kind,
            });
            let result = client::rpc_call(self.plugin(), methods::PANE_CAPTURE, rpc_params)
                .map_err(|e| mcp_err(&e))?;
            let content = result["content"].as_str().unwrap_or("");

            return Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(content.to_string(), uri.clone())],
            });
        }

        Err(McpError::resource_not_found(
            format!("unknown resource: {uri}"),
            None,
        ))
    }
}

// --- Entrypoint ---

pub async fn run(plugin: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let server = ZjctlMcp::new(plugin.map(|s| s.to_string()));
    let service = rmcp::ServiceExt::serve(server, rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

// --- Helpers ---

fn mcp_err(e: &dyn std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

/// Reuse pane launch helpers from the pane module (duplicated here to avoid
/// making private functions public just for MCP).
fn run_new_pane_action(options: &pane::LaunchOptions<'_>) -> Result<(), Box<dyn std::error::Error>> {
    use crate::zellij;
    let mut cmd = zellij::command();
    if options.command.is_empty() {
        cmd.args(["action", "new-pane"]);
    } else {
        cmd.arg("run");
    }
    if let Some(direction) = options.direction {
        cmd.args(["--direction", direction]);
    }
    if options.floating {
        cmd.arg("--floating");
    }
    if let Some(name) = options.name {
        cmd.args(["--name", name]);
    }
    if let Some(cwd) = options.cwd {
        cmd.args(["--cwd", cwd]);
    }
    if options.close_on_exit {
        cmd.arg("--close-on-exit");
    }
    if options.in_place {
        cmd.arg("--in-place");
    }
    if options.start_suspended {
        cmd.arg("--start-suspended");
    }
    if !options.command.is_empty() {
        cmd.arg("--").args(options.command);
    }
    let status = cmd.status().map_err(|e| format!("failed to run zellij: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("zellij action new-pane failed: {status:?}").into())
    }
}

fn find_new_pane(
    panes: &[panes::PaneInfo],
    focused_tab_index: Option<usize>,
    options: &pane::LaunchOptions<'_>,
    before_max_terminal_id: u32,
) -> Option<panes::PaneInfo> {
    let mut candidates: Vec<panes::PaneInfo> = panes
        .iter()
        .filter(|p| p.pane_type == "terminal" && !p.suppressed)
        .filter(|p| parse_terminal_id(&p.id).is_some_and(|id| id > before_max_terminal_id))
        .cloned()
        .collect();

    if options.floating {
        candidates.retain(|p| p.floating);
    }
    if let Some(tab) = focused_tab_index {
        candidates.retain(|p| p.tab_index == tab);
    }
    if let Some(name) = options.name {
        candidates.retain(|p| p.title.contains(name));
    }
    candidates.sort_by_key(|p| parse_terminal_id(&p.id).unwrap_or(0));
    candidates.pop()
}

fn parse_terminal_id(id: &str) -> Option<u32> {
    let mut parts = id.split(':');
    let pane_type = parts.next()?;
    let numeric = parts.next()?;
    if parts.next().is_some() || pane_type != "terminal" {
        return None;
    }
    numeric.parse().ok()
}

fn pane_id_to_selector(id: &str) -> Option<String> {
    let mut parts = id.split(':');
    let pane_type = parts.next()?;
    let numeric = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    match pane_type {
        "terminal" | "plugin" => Some(format!("id:{pane_type}:{numeric}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_pane(id: &str) -> panes::PaneInfo {
        panes::PaneInfo {
            id: id.to_string(),
            pane_type: "terminal".to_string(),
            title: String::new(),
            command: None,
            tab_index: 0,
            tab_name: "tab".to_string(),
            focused: false,
            floating: false,
            suppressed: false,
            rows: 24,
            cols: 80,
            exit_status: None,
            tags: HashMap::new(),
        }
    }

    fn default_options<'a>() -> pane::LaunchOptions<'a> {
        pane::LaunchOptions {
            direction: None,
            floating: false,
            name: None,
            cwd: None,
            close_on_exit: false,
            in_place: false,
            start_suspended: false,
            command: &[],
        }
    }

    // --- parse_terminal_id ---

    #[test]
    fn parse_terminal_id_valid() {
        assert_eq!(parse_terminal_id("terminal:3"), Some(3));
        assert_eq!(parse_terminal_id("terminal:0"), Some(0));
    }

    #[test]
    fn parse_terminal_id_invalid() {
        assert_eq!(parse_terminal_id("plugin:3"), None);
        assert_eq!(parse_terminal_id("terminal:3:extra"), None);
    }

    #[test]
    fn parse_terminal_id_non_numeric() {
        assert_eq!(parse_terminal_id("terminal:abc"), None);
    }

    // --- pane_id_to_selector ---

    #[test]
    fn pane_id_to_selector_terminal() {
        assert_eq!(
            pane_id_to_selector("terminal:3"),
            Some("id:terminal:3".to_string())
        );
    }

    #[test]
    fn pane_id_to_selector_plugin() {
        assert_eq!(
            pane_id_to_selector("plugin:7"),
            Some("id:plugin:7".to_string())
        );
    }

    #[test]
    fn pane_id_to_selector_rejects_invalid() {
        assert_eq!(pane_id_to_selector("invalid"), None);
        assert_eq!(pane_id_to_selector("terminal:1:extra"), None);
        assert_eq!(pane_id_to_selector("other:5"), None);
    }

    // --- find_new_pane ---

    #[test]
    fn find_new_pane_returns_none_when_no_panes_exceed_baseline() {
        let panes = vec![make_pane("terminal:1"), make_pane("terminal:2")];
        let opts = default_options();
        assert!(find_new_pane(&panes, None, &opts, 5).is_none());
    }

    #[test]
    fn find_new_pane_ignores_plugin_panes() {
        let mut p = make_pane("plugin:10");
        p.pane_type = "plugin".to_string();
        let panes = vec![p];
        let opts = default_options();
        assert!(find_new_pane(&panes, None, &opts, 0).is_none());
    }

    #[test]
    fn find_new_pane_ignores_suppressed() {
        let mut p = make_pane("terminal:10");
        p.suppressed = true;
        let panes = vec![p];
        let opts = default_options();
        assert!(find_new_pane(&panes, None, &opts, 0).is_none());
    }

    #[test]
    fn find_new_pane_floating_filter() {
        let mut p1 = make_pane("terminal:10");
        p1.floating = true;
        let mut p2 = make_pane("terminal:11");
        p2.floating = false;

        let mut opts = default_options();
        opts.floating = true;
        let result = find_new_pane(&[p1, p2], None, &opts, 0);
        assert_eq!(result.unwrap().id, "terminal:10");
    }

    #[test]
    fn find_new_pane_tab_filter() {
        let mut p1 = make_pane("terminal:10");
        p1.tab_index = 0;
        let mut p2 = make_pane("terminal:11");
        p2.tab_index = 1;

        let opts = default_options();
        let result = find_new_pane(&[p1, p2], Some(1), &opts, 0);
        assert_eq!(result.unwrap().id, "terminal:11");
    }

    #[test]
    fn find_new_pane_name_filter() {
        let mut p1 = make_pane("terminal:10");
        p1.title = "build-output".to_string();
        let mut p2 = make_pane("terminal:11");
        p2.title = "server".to_string();

        let cmd = vec![];
        let opts = pane::LaunchOptions {
            name: Some("build"),
            command: &cmd,
            ..default_options()
        };
        let result = find_new_pane(&[p1, p2], None, &opts, 0);
        assert_eq!(result.unwrap().id, "terminal:10");
    }

    #[test]
    fn find_new_pane_returns_highest_id() {
        let panes = vec![
            make_pane("terminal:5"),
            make_pane("terminal:9"),
            make_pane("terminal:7"),
        ];
        let opts = default_options();
        let result = find_new_pane(&panes, None, &opts, 0);
        assert_eq!(result.unwrap().id, "terminal:9");
    }

    #[test]
    fn find_new_pane_no_tab_filter_when_none() {
        let mut p1 = make_pane("terminal:10");
        p1.tab_index = 0;
        let mut p2 = make_pane("terminal:11");
        p2.tab_index = 1;

        let opts = default_options();
        // focused_tab_index=None → no tab filtering, returns highest id
        let result = find_new_pane(&[p1, p2], None, &opts, 0);
        assert_eq!(result.unwrap().id, "terminal:11");
    }

    // --- plugin accessor ---

    #[test]
    fn new_with_plugin_returns_some() {
        let s = ZjctlMcp::new(Some("/tmp/x.wasm".to_string()));
        assert_eq!(s.plugin(), Some("/tmp/x.wasm"));
    }

    #[test]
    fn new_without_plugin_returns_none() {
        let s = ZjctlMcp::new(None);
        assert_eq!(s.plugin(), None);
    }
}
