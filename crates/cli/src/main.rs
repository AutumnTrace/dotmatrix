//! Headless runner: serial output, benchmarks and test ROM checks.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use dotmatrix_core::harness::{self, Verdict};
use dotmatrix_core::{Button, FRAMES_PER_SECOND, GameBoy};

const USAGE: &str = "\
dotmatrix - a Game Boy (DMG) emulator

USAGE:
    dotmatrix run <rom> [options]
    dotmatrix check <rom-or-dir>... [--timeout <frames>]
    dotmatrix info <rom>

RUN OPTIONS:
    --frames <n>            frames to run (default 600, about 10 s)
    --press <btn>@<frame>[:<hold>]
                            hold a button from that frame for <hold> frames
                            (default 5); buttons: a b start select up down left right
    --serial                print serial port output as it arrives
    --save <file>           load battery RAM from this file and write it back on exit
    --bench                 run as fast as possible and report frames per second
";

struct Press {
    button: Button,
    frame: u64,
    hold: u64,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("info") => info(&args[1..]),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!("unknown command '{other}'\n\n{USAGE}")),
    };
    match result {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
    }
}

fn load(path: &Path) -> Result<GameBoy, String> {
    let rom = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    GameBoy::new(rom).map_err(|e| format!("{}: {e}", path.display()))
}

fn value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    *i += 1;
    args.get(*i).map(String::as_str).ok_or_else(|| format!("{flag} needs a value"))
}

fn number(s: &str, flag: &str) -> Result<u64, String> {
    s.parse().map_err(|_| format!("{flag}: '{s}' is not a number"))
}

fn parse_press(spec: &str) -> Result<Press, String> {
    let (name, rest) = spec.split_once('@').ok_or("--press expects <button>@<frame>[:<hold>]")?;
    let button = Button::from_name(name).ok_or_else(|| format!("unknown button '{name}'"))?;
    let (frame, hold) = match rest.split_once(':') {
        Some((f, h)) => (number(f, "--press")?, number(h, "--press")?),
        None => (number(rest, "--press")?, 5),
    };
    Ok(Press { button, frame, hold })
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut rom = None;
    let mut frames = 600u64;
    let mut presses = Vec::new();
    let mut serial = false;
    let mut save: Option<PathBuf> = None;
    let mut bench = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--frames" => frames = number(value(args, &mut i, "--frames")?, "--frames")?,
            "--press" => presses.push(parse_press(value(args, &mut i, "--press")?)?),
            "--serial" => serial = true,
            "--save" => save = Some(value(args, &mut i, "--save")?.into()),
            "--bench" => bench = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option '{flag}'")),
            path => rom = Some(PathBuf::from(path)),
        }
        i += 1;
    }
    let rom = rom.ok_or("run needs a ROM path")?;
    let mut gb = load(&rom)?;

    if let Some(path) = &save {
        if let Ok(data) = std::fs::read(path) {
            if !gb.load_save_data(&data) {
                eprintln!("warning: {} has the wrong size for this cartridge, ignoring it", path.display());
            }
        }
    }

    let mut printed = 0;
    let start = Instant::now();
    for frame in 0..frames {
        for p in &presses {
            if frame == p.frame {
                gb.set_button(p.button, true);
            } else if frame == p.frame + p.hold {
                gb.set_button(p.button, false);
            }
        }
        gb.run_frame();
        gb.clear_audio();
        if serial && gb.serial_output().len() > printed {
            let out = std::io::stdout();
            let mut out = out.lock();
            let _ = out.write_all(&gb.serial_output()[printed..]);
            let _ = out.flush();
            printed = gb.serial_output().len();
        }
    }
    let elapsed = start.elapsed().as_secs_f64();

    if bench {
        let fps = frames as f64 / elapsed;
        println!(
            "{frames} frames in {elapsed:.3} s: {fps:.0} frames/s, {:.1}x real time ({} at {FRAMES_PER_SECOND:.2} Hz)",
            fps / FRAMES_PER_SECOND,
            gb.header().title
        );
    }

    if let (Some(path), Some(data)) = (&save, gb.save_data()) {
        std::fs::write(path, data).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn collect_roms(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            collect_roms(&p, out);
        }
    } else if path.extension().is_some_and(|e| e == "gb") {
        out.push(path.to_path_buf());
    }
}

fn check(args: &[String]) -> Result<ExitCode, String> {
    let mut timeout = 60 * 60u64;
    let mut roms = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--timeout" => timeout = number(value(args, &mut i, "--timeout")?, "--timeout")?,
            path => collect_roms(Path::new(path), &mut roms),
        }
        i += 1;
    }
    if roms.is_empty() {
        return Err("check needs at least one .gb file or directory".into());
    }
    // Write through a lock and stop quietly if the reader goes away
    // (`dotmatrix check dir | head`).
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut failed = 0;
    for path in &roms {
        let verdict = match load(path) {
            Ok(mut gb) => harness::auto(&mut gb, timeout),
            Err(e) => Verdict::Fail(e),
        };
        let label = match &verdict {
            Verdict::Pass => "PASS".to_string(),
            Verdict::Fail(why) => format!("FAIL  {}", why.lines().last().unwrap_or("")),
            Verdict::Timeout => "TIMEOUT".to_string(),
        };
        if !verdict.passed() {
            failed += 1;
        }
        // Forward slashes on every platform, so output reads the same in CI.
        let shown = path.display().to_string().replace('\\', "/");
        if writeln!(out, "{shown:<60} {label}").is_err() {
            return Ok(ExitCode::FAILURE);
        }
    }
    let _ = writeln!(out, "\n{} passed, {failed} failed", roms.len() - failed);
    Ok(if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn info(args: &[String]) -> Result<ExitCode, String> {
    let path = args.first().ok_or("info needs a ROM path")?;
    let rom = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let h = dotmatrix_core::Header::parse(&rom).map_err(|e| e.to_string())?;
    println!("title      {}", h.title);
    println!(
        "type       0x{:02X} {}",
        h.cartridge_type,
        dotmatrix_core::cartridge::cartridge_type_name(h.cartridge_type)
    );
    println!("rom        {} KiB", h.rom_size / 1024);
    println!("ram        {} KiB", h.ram_size / 1024);
    println!("cgb flag   0x{:02X}", h.cgb_flag);
    println!("checksum   {}", if h.header_checksum_ok { "ok" } else { "BAD" });
    Ok(ExitCode::SUCCESS)
}
