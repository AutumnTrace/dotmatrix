//! Drives the real binary over stdio, the way an MCP client would, and plays
//! a few frames of the bundled demo cartridge.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dotmatrix-mcp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start dotmatrix-mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdin, stdout, next_id: 1 }
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).expect("response is JSON");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], id);
        response
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({ "name": tool, "arguments": arguments }));
        let result = response["result"].clone();
        assert_eq!(result["resultType"], "complete", "{response}");
        result
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn demo_rom() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/roms/tobutobugirl.gb");
    path.canonicalize().unwrap().to_string_lossy().into_owned()
}

fn first_text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap_or_default()
}

#[test]
fn legacy_client_can_play_the_demo_cartridge() {
    let mut c = Client::spawn();
    let init = c.request(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "dotmatrix-mcp");
    c.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let list = c.request("tools/list", json!({}));
    let names: Vec<&str> =
        list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["load_rom", "press_buttons", "run_frames", "screenshot", "read_memory"]);

    let before_load = c.call("run_frames", json!({ "frames": 1 }));
    assert_eq!(before_load["isError"], true);
    assert!(first_text(&before_load).contains("load_rom"));

    let loaded = c.call("load_rom", json!({ "path": demo_rom() }));
    assert_eq!(loaded["isError"], false, "{loaded}");
    assert!(first_text(&loaded).contains("TOBU"));

    let ran = c.call("run_frames", json!({ "frames": 240 }));
    assert!(first_text(&ran).contains("frame 240"), "{ran}");

    let pressed = c.call("press_buttons", json!({ "buttons": ["start"], "screenshot": true }));
    assert_eq!(pressed["isError"], false, "{pressed}");
    assert!(first_text(&pressed).contains("frame 250"), "{pressed}");
    assert_eq!(pressed["content"][1]["type"], "image");

    let shot = c.call("screenshot", json!({ "scale": 1 }));
    let image = &shot["content"][0];
    assert_eq!(image["mimeType"], "image/png");
    // "iVBORw0KGgo" is the base64 form of the PNG signature.
    assert!(image["data"].as_str().unwrap().starts_with("iVBORw0KGgo"));

    // The cartridge header title lives at 0x0134.
    let mem = c.call("read_memory", json!({ "address": "0x0134", "length": 4 }));
    assert!(first_text(&mem).contains("54 4F 42 55"), "{mem}");

    let bad = c.call("press_buttons", json!({ "buttons": ["turbo"] }));
    assert_eq!(bad["isError"], true);
}

#[test]
fn modern_client_discovers_and_gets_version_errors() {
    let mut c = Client::spawn();
    let meta = |v: &str| json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": v } });

    let discover = c.request("server/discover", meta("2026-07-28"));
    let result = &discover["result"];
    assert_eq!(result["resultType"], "complete");
    assert!(result["supportedVersions"].as_array().unwrap().contains(&json!("2026-07-28")));
    assert_eq!(result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "dotmatrix-mcp");

    let list = c.request("tools/list", meta("2026-07-28"));
    assert_eq!(list["result"]["cacheScope"], "public");

    let wrong = c.request("tools/list", meta("2030-01-01"));
    assert_eq!(wrong["error"]["code"], -32022);
    assert_eq!(wrong["error"]["data"]["requested"], "2030-01-01");

    let unknown = c.request("resources/list", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);
}
