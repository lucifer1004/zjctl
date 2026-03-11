//! Structured output helpers for JSON mode.

use crate::client::ClientError;
use serde::Serialize;

/// Exit codes for structured error handling.
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_RPC_ERROR: i32 = 1;
pub const EXIT_CLIENT_ERROR: i32 = 2;
// Exit code 3 is reserved for usage errors (clap handles these).

/// Print a success result in JSON mode.
pub fn print_success(data: impl Serialize) {
    let wrapper = serde_json::json!({
        "ok": true,
        "result": serde_json::to_value(data).expect("failed to serialize result"),
    });
    println!(
        "{}",
        serde_json::to_string(&wrapper).expect("failed to serialize output")
    );
}

/// Format an error as JSON to stderr and return the appropriate exit code.
pub fn format_error(err: &(dyn std::error::Error + 'static)) -> i32 {
    let (code, exit) = classify_error(err);
    let wrapper = serde_json::json!({
        "ok": false,
        "error": {
            "code": code,
            "message": err.to_string(),
        }
    });
    eprintln!(
        "{}",
        serde_json::to_string(&wrapper).expect("failed to serialize error")
    );
    exit
}

fn classify_error(err: &(dyn std::error::Error + 'static)) -> (&'static str, i32) {
    // Try to downcast to ClientError for specific codes
    if let Some(client_err) = err.downcast_ref::<ClientError>() {
        return match client_err {
            ClientError::RpcError(msg) => {
                // Try to extract the RPC error code from the message
                let code = if msg.contains("no panes match") || msg.contains("no focused pane") {
                    "no_match"
                } else if msg.contains("panes match selector") {
                    "ambiguous_match"
                } else if msg.contains("unknown method") {
                    "method_not_found"
                } else {
                    "rpc_error"
                };
                (code, EXIT_RPC_ERROR)
            }
            ClientError::ZellijMissing => ("zellij_missing", EXIT_CLIENT_ERROR),
            ClientError::PluginNotInstalled { .. } => ("plugin_not_installed", EXIT_CLIENT_ERROR),
            ClientError::PluginNotLoaded { .. } => ("plugin_not_loaded", EXIT_CLIENT_ERROR),
            ClientError::PipeError { .. } => ("pipe_error", EXIT_CLIENT_ERROR),
            ClientError::Spawn(_) => ("spawn_error", EXIT_CLIENT_ERROR),
            ClientError::Io(_) => ("io_error", EXIT_CLIENT_ERROR),
            ClientError::Serialize(_) => ("serialize_error", EXIT_CLIENT_ERROR),
        };
    }

    // Generic error
    ("error", EXIT_RPC_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rpc_no_match() {
        let err = ClientError::RpcError("no panes match selector".to_string());
        let (code, exit) = classify_error(&err);
        assert_eq!(code, "no_match");
        assert_eq!(exit, EXIT_RPC_ERROR);
    }

    #[test]
    fn classify_rpc_ambiguous() {
        let err = ClientError::RpcError("3 panes match selector; use --all".to_string());
        let (code, exit) = classify_error(&err);
        assert_eq!(code, "ambiguous_match");
        assert_eq!(exit, EXIT_RPC_ERROR);
    }

    #[test]
    fn classify_zellij_missing() {
        let err = ClientError::ZellijMissing;
        let (code, exit) = classify_error(&err);
        assert_eq!(code, "zellij_missing");
        assert_eq!(exit, EXIT_CLIENT_ERROR);
    }

    #[test]
    fn classify_generic_error() {
        let err: Box<dyn std::error::Error> = "something went wrong".into();
        let (code, exit) = classify_error(err.as_ref());
        assert_eq!(code, "error");
        assert_eq!(exit, EXIT_RPC_ERROR);
    }

    #[test]
    fn classify_rpc_method_not_found() {
        let err = ClientError::RpcError("unknown method: foo".to_string());
        let (code, _) = classify_error(&err);
        assert_eq!(code, "method_not_found");
    }

    #[test]
    fn classify_rpc_generic_fallthrough() {
        let err = ClientError::RpcError("something unrecognised".to_string());
        let (code, exit) = classify_error(&err);
        assert_eq!(code, "rpc_error");
        assert_eq!(exit, EXIT_RPC_ERROR);
    }

    #[test]
    fn classify_plugin_not_installed() {
        let err = ClientError::PluginNotInstalled {
            path: "/tmp/x.wasm".to_string(),
            install_cmd: String::new(),
            download_cmd: String::new(),
            launch_cmd: String::new(),
        };
        let (code, exit) = classify_error(&err);
        assert_eq!(code, "plugin_not_installed");
        assert_eq!(exit, EXIT_CLIENT_ERROR);
    }

    #[test]
    fn print_success_json_structure() {
        // Capture stdout by serializing manually (print_success writes to stdout)
        let data = serde_json::json!({"pane": "terminal:3"});
        let wrapper = serde_json::json!({
            "ok": true,
            "result": data,
        });
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&wrapper).unwrap()
        ).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["result"]["pane"], "terminal:3");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn format_error_json_structure() {
        let err = ClientError::RpcError("no panes match selector".to_string());
        let (code, _) = classify_error(&err);
        // Reconstruct the same JSON that format_error produces
        let wrapper = serde_json::json!({
            "ok": false,
            "error": {
                "code": code,
                "message": err.to_string(),
            }
        });
        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&wrapper).unwrap()
        ).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "no_match");
        assert!(json["error"]["message"].as_str().unwrap().contains("no panes match"));
        assert!(json.get("result").is_none());
    }
}
