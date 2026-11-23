# dotmatrix

A Game Boy (DMG) emulator in Rust. Work in progress.

```sh
./scripts/fetch-test-roms.sh   # Blargg, Mooneye and friends, into test-roms/ (not committed)
cargo test --workspace         # ROM suites skip themselves if test-roms/ is missing
cargo run --release -p dotmatrix-cli -- check test-roms/blargg/cpu_instrs
```

MIT licensed.
