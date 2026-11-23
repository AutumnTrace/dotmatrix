//! dotmatrix-mcp: a Model Context Protocol server on stdio.
//!
//! It gives an agent a Game Boy: load a ROM, hold buttons, run frames, look
//! at the screen (PNG image content) and peek at memory. JSON-RPC is
//! hand-rolled on top of serde_json, one message per line.
//!
//! The server is dual-era: modern clients (protocol 2026-07-28) can probe
//! with `server/discover` and send their version in each request's `_meta`;
//! legacy clients (2025-11-25 and earlier) get the `initialize` handshake.

use std::io::{BufRead, Write};

use dotmatrix_core::png::{self, palette};
use dotmatrix_core::{Button, GameBoy, HEIGHT, WIDTH};
use serde_json::{Value, json};

const MODERN_VERSIONS: &[&str] = &["2026-07-28"];
const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const INSTRUCTIONS: &str = "dotmatrix emulates an original Game Boy (DMG). Call load_rom with a path \
to a .gb file, then alternate press_buttons / run_frames with screenshot to see what happened. \
The console runs at about 59.7 frames per second, so 60 frames is one second of game time. \
Most games ignore presses shorter than 2-3 frames; the default hold of 6 frames is a safe tap.";

struct Session {
    gb: Option<GameBoy>,
    rom_name: String,
}

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
}

/// A tool failure is reported inside a successful result with `isError`,
/// so the model can read it and recover.
type ToolResult = Result<Vec<Value>, String>;

fn server_info() -> Value {
    json!({ "name": "dotmatrix-mcp", "title": "dotmatrix Game Boy emulator", "version": env!("CARGO_PKG_VERSION") })
}

fn tools() -> Value {
    let buttons = json!(["a", "b", "start", "select", "up", "down", "left", "right"]);
    json!([
        {
            "name": "load_rom",
            "title": "Load ROM",
            "description": "Load a Game Boy (DMG) ROM from a file path and power on. Replaces any running game.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Path to a .gb file" } },
                "required": ["path"]
            }
        },
        {
            "name": "press_buttons",
            "title": "Press buttons",
            "description": "Hold one or more buttons for some frames, release them, then let the game run a few more frames.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "buttons": { "type": "array", "items": { "type": "string", "enum": buttons }, "minItems": 1 },
                    "hold_frames": { "type": "integer", "minimum": 1, "maximum": 600, "default": 6 },
                    "release_frames": { "type": "integer", "minimum": 0, "maximum": 600, "default": 4,
                        "description": "Frames to run after releasing" },
                    "screenshot": { "type": "boolean", "default": false,
                        "description": "Attach a screenshot taken after the release frames" }
                },
                "required": ["buttons"]
            }
        },
        {
            "name": "run_frames",
            "title": "Run frames",
            "description": "Advance emulation by a number of video frames (60 is about one second) with no buttons held.",
            "inputSchema": {
                "type": "object",
                "properties": { "frames": { "type": "integer", "minimum": 1, "maximum": 36000 } },
                "required": ["frames"]
            }
        },
        {
            "name": "screenshot",
            "title": "Screenshot",
            "description": "Return the current 160x144 screen as a PNG image.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scale": { "type": "integer", "minimum": 1, "maximum": 4, "default": 2 },
                    "palette": { "type": "string", "enum": ["gray", "green"], "default": "gray" }
                }
            }
        },
        {
            "name": "read_memory",
            "title": "Read memory",
            "description": "Read bytes from the 16-bit address space without side effects (VRAM and OAM are always visible). Returns a hex dump.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": ["integer", "string"], "description": "Start address, e.g. 49152 or \"0xC000\"" },
                    "length": { "type": "integer", "minimum": 1, "maximum": 1024, "default": 16 }
                },
                "required": ["address"]
            }
        }
    ])
}

impl Session {
    fn gb(&mut self) -> Result<&mut GameBoy, String> {
        self.gb.as_mut().ok_or_else(|| "no ROM loaded yet: call load_rom first".to_string())
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Result<ToolResult, RpcError> {
        Ok(match name {
            "load_rom" => self.load_rom(args),
            "press_buttons" => self.press_buttons(args),
            "run_frames" => self.run_frames(args),
            "screenshot" => self.screenshot(args).map(|img| vec![img]),
            "read_memory" => self.read_memory(args),
            other => return Err(RpcError::new(INVALID_PARAMS, format!("unknown tool '{other}'"))),
        })
    }

    fn load_rom(&mut self, args: &Value) -> ToolResult {
        let path = args.get("path").and_then(Value::as_str).ok_or("missing 'path'")?;
        let rom = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let size = rom.len();
        let gb = GameBoy::new(rom).map_err(|e| e.to_string())?;
        let h = gb.header();
        let text = format!(
            "Loaded '{}' ({} KiB, {}{}). The console is at frame 0; run some frames to get past the intro.",
            h.title,
            size / 1024,
            dotmatrix_core::cartridge::cartridge_type_name(h.cartridge_type),
            if gb.has_battery() { ", battery RAM" } else { "" },
        );
        self.rom_name = h.title.clone();
        self.gb = Some(gb);
        Ok(vec![text_content(text)])
    }

    fn press_buttons(&mut self, args: &Value) -> ToolResult {
        let names = args.get("buttons").and_then(Value::as_array).ok_or("missing 'buttons' array")?;
        let mut buttons = Vec::new();
        for n in names {
            let s = n.as_str().ok_or("buttons must be strings")?;
            buttons.push(Button::from_name(s).ok_or_else(|| format!("unknown button '{s}'"))?);
        }
        if buttons.is_empty() {
            return Err("press at least one button".into());
        }
        let hold = int_arg(args, "hold_frames", 6, 1, 600)?;
        let release = int_arg(args, "release_frames", 4, 0, 600)?;
        let want_shot = args.get("screenshot").and_then(Value::as_bool).unwrap_or(false);
        let gb = self.gb()?;
        for &b in &buttons {
            gb.set_button(b, true);
        }
        for _ in 0..hold {
            gb.run_frame();
            gb.clear_audio();
        }
        for &b in &buttons {
            gb.set_button(b, false);
        }
        for _ in 0..release {
            gb.run_frame();
            gb.clear_audio();
        }
        let frame = gb.frame_count();
        let mut out = vec![text_content(format!(
            "Held {names} for {hold} frames, then ran {release} more. Now at frame {frame}.",
            names = list(names)
        ))];
        if want_shot {
            out.push(self.screenshot(&json!({}))?);
        }
        Ok(out)
    }

    fn run_frames(&mut self, args: &Value) -> ToolResult {
        let frames = int_arg(args, "frames", 0, 1, 36_000)?;
        let gb = self.gb()?;
        for _ in 0..frames {
            gb.run_frame();
            gb.clear_audio();
        }
        Ok(vec![text_content(format!("Ran {frames} frames. Now at frame {}.", gb.frame_count()))])
    }

    fn screenshot(&mut self, args: &Value) -> Result<Value, String> {
        let scale = int_arg(args, "scale", 2, 1, 4)? as usize;
        let pal = match args.get("palette").and_then(Value::as_str).unwrap_or("gray") {
            "gray" | "grey" => palette::GRAYSCALE,
            "green" => palette::DMG_GREEN,
            other => return Err(format!("unknown palette '{other}'")),
        };
        let gb = self.gb()?;
        let rgb = png::shades_to_rgb(gb.framebuffer(), WIDTH, &pal, scale);
        let bytes = png::encode((WIDTH * scale) as u32, (HEIGHT * scale) as u32, 3, &rgb);
        Ok(json!({ "type": "image", "data": base64(&bytes), "mimeType": "image/png" }))
    }

    fn read_memory(&mut self, args: &Value) -> ToolResult {
        let address = match args.get("address") {
            Some(Value::Number(n)) => n.as_u64().ok_or("address must be a non-negative integer")?,
            Some(Value::String(s)) => {
                let t = s.trim();
                let hex = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).or_else(|| t.strip_prefix('$'));
                match hex {
                    Some(h) => u64::from_str_radix(h, 16),
                    None => t.parse::<u64>().or_else(|_| u64::from_str_radix(t, 16)),
                }
                .map_err(|_| format!("cannot parse address '{s}'"))?
            }
            _ => return Err("missing 'address'".into()),
        };
        if address > 0xFFFF {
            return Err(format!("address 0x{address:X} is outside the 16-bit address space"));
        }
        let length = int_arg(args, "length", 16, 1, 1024)?;
        let gb = self.gb()?;
        let start = address as u32;
        let end = (start + length as u32).min(0x10000);
        let mut text = String::new();
        let mut row = start & !0xF;
        while row < end {
            text.push_str(&format!("{row:04X}:"));
            for a in row..row + 16 {
                if a >= start && a < end {
                    text.push_str(&format!(" {:02X}", gb.peek(a as u16)));
                } else {
                    text.push_str("   ");
                }
            }
            text.push('\n');
            row += 16;
        }
        Ok(vec![text_content(text)])
    }
}

fn int_arg(args: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    match args.get(key) {
        None | Some(Value::Null) if default >= min => Ok(default),
        None | Some(Value::Null) => Err(format!("missing '{key}'")),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| format!("'{key}' must be a non-negative integer"))?;
            if (min..=max).contains(&n) { Ok(n) } else { Err(format!("'{key}' must be between {min} and {max}")) }
        }
    }
}

fn list(values: &[Value]) -> String {
    values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("+")
}

fn text_content(text: impl Into<String>) -> Value {
    json!({ "type": "text", "text": text.into() })
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Modern requests name their protocol version in `_meta`; reject the ones
/// we do not speak, as the 2026-07-28 spec asks.
fn check_version(params: &Value) -> Result<(), RpcError> {
    let Some(requested) = params.pointer("/_meta/io.modelcontextprotocol~1protocolVersion").and_then(Value::as_str)
    else {
        return Ok(());
    };
    if MODERN_VERSIONS.contains(&requested) || LEGACY_VERSIONS.contains(&requested) {
        return Ok(());
    }
    let supported: Vec<&str> = MODERN_VERSIONS.iter().chain(LEGACY_VERSIONS).copied().collect();
    Err(RpcError {
        code: UNSUPPORTED_PROTOCOL_VERSION,
        message: format!("unsupported protocol version {requested}"),
        data: Some(json!({ "supported": supported, "requested": requested })),
    })
}

fn complete(mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("resultType".into(), json!("complete"));
        obj.entry("_meta").or_insert_with(|| json!({ "io.modelcontextprotocol/serverInfo": server_info() }));
    }
    result
}

fn handle(session: &mut Session, method: &str, params: &Value) -> Result<Value, RpcError> {
    check_version(params)?;
    match method {
        "server/discover" => {
            let supported: Vec<&str> = MODERN_VERSIONS.iter().chain(LEGACY_VERSIONS).copied().collect();
            Ok(complete(json!({
                "supportedVersions": supported,
                "capabilities": { "tools": {} },
                "instructions": INSTRUCTIONS,
                "ttlMs": 3_600_000,
                "cacheScope": "public",
            })))
        }
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = if LEGACY_VERSIONS.contains(&requested) { requested } else { LEGACY_VERSIONS[0] };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": server_info(),
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(complete(json!({ "tools": tools(), "ttlMs": 3_600_000, "cacheScope": "public" }))),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call needs a tool name"))?;
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = match session.call_tool(name, &args)? {
                Ok(content) => json!({ "content": content, "isError": false }),
                Err(message) => json!({ "content": [text_content(message)], "isError": true }),
            };
            Ok(complete(result))
        }
        other => Err(RpcError::new(METHOD_NOT_FOUND, format!("method '{other}' not found"))),
    }
}

fn respond(out: &mut impl Write, id: Value, outcome: Result<Value, RpcError>) -> std::io::Result<()> {
    let message = match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => {
            let mut error = json!({ "code": e.code, "message": e.message });
            if let Some(data) = e.data {
                error["data"] = data;
            }
            json!({ "jsonrpc": "2.0", "id": id, "error": error })
        }
    };
    writeln!(out, "{message}")?;
    out.flush()
}

fn main() -> std::io::Result<()> {
    let mut session = Session { gb: None, rom_name: String::new() };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                respond(&mut stdout, Value::Null, Err(RpcError::new(-32700, format!("parse error: {e}"))))?;
                continue;
            }
        };
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            continue; // a response to something we never sent
        };
        let Some(id) = msg.get("id").cloned() else {
            continue; // notification: nothing to answer
        };
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        let outcome = handle(&mut session, method, &params);
        respond(&mut stdout, id, outcome)?;
    }
    if !session.rom_name.is_empty() {
        eprintln!("dotmatrix-mcp: session ended ({})", session.rom_name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        let cases =
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE=")];
        for (input, expected) in cases {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn rejects_unknown_protocol_versions() {
        let params = json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "1999-01-01" } });
        let err = check_version(&params).unwrap_err();
        assert_eq!(err.code, UNSUPPORTED_PROTOCOL_VERSION);
        assert_eq!(err.data.unwrap()["requested"], "1999-01-01");
    }
}
