use std::{
    io::Write,
    process::{Command, Stdio},
};

use serde_json::Value;

#[test]
fn stdout_contains_only_mcp_protocol_and_eof_stops_the_bridge() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_airwiki-mcp-bridge"))
        .args(["--client", "chatgpt-desktop"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start bridge");
    let mut stdin = child.stdin.take().expect("bridge stdin");
    stdin
        .write_all(
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",",
                "\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},",
                "\"clientInfo\":{\"name\":\"test-client\",\"version\":\"1\"}}}\n",
                "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\",\"params\":{}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
            )
            .as_bytes(),
        )
        .expect("write MCP requests");
    drop(stdin);

    let output = child.wait_with_output().expect("wait for EOF shutdown");

    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "successful protocol exchange must not log to stderr"
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 protocol output");
    let messages = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("stdout JSON-RPC message"))
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].get("jsonrpc").and_then(Value::as_str),
        Some("2.0")
    );
    assert_eq!(messages[0].get("id").and_then(Value::as_u64), Some(1));
    assert_eq!(
        messages[1].get("jsonrpc").and_then(Value::as_str),
        Some("2.0")
    );
    assert_eq!(messages[1].get("id").and_then(Value::as_u64), Some(2));
}
