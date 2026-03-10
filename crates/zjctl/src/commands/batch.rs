//! Batch RPC command

use crate::client;
use crate::output;
use zjctl_proto::methods;

pub fn run(plugin: Option<&str>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let input = std::io::read_to_string(std::io::stdin())?;
    let ops: serde_json::Value = serde_json::from_str(&input)
        .map_err(|e| format!("invalid JSON input: {e}"))?;

    if !ops.is_array() {
        return Err("input must be a JSON array of {\"method\": ..., \"params\": ...} objects".into());
    }

    let params = serde_json::json!({ "ops": ops });
    let result = client::rpc_call(plugin, methods::BATCH, params)?;

    if json {
        output::print_success(result);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn batch_input_must_be_array() {
        let input = r#"{"method": "panes.list"}"#;
        let parsed: serde_json::Value = serde_json::from_str(input).unwrap();
        assert!(!parsed.is_array());
    }

    #[test]
    fn batch_input_array_parses() {
        let input = r#"[{"method":"panes.list","params":{}},{"method":"pane.focus","params":{"selector":"focused"}}]"#;
        let parsed: serde_json::Value = serde_json::from_str(input).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 2);
    }
}
