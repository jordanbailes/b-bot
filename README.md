# B-Bot

`bbot` is a minimal Rust CLI with typed tool-oriented capabilities:

- `docker.remove_all_running_containers`
- `rust.count_lines_of_code`
- `rust.run_coverage`

It uses a local Ollama model to decide whether the user request matches one of the available tools. Docker cleanup is executed through Docker's API using `bollard`, Rust LOC counting is executed directly through the embedded `tokei` crate, and Rust coverage is executed through the embedded `cargo-tarpaulin` library using the `ptrace` engine.

## Usage

```bash
bbot "delete all running docker containers"
bbot "count rust lines of code in this repo"
bbot "run coverage for this repo"
```

Interactive mode:

```bash
bbot
```

Then type prompts at the `bbot>` prompt. Use `exit` or `quit` to leave.

Helpful flags:

```bash
bbot --dry-run "delete all running docker containers"
bbot --yes "delete all running docker containers"
bbot --model qwen3:8b "delete all running docker containers"
```

## Environment

- `OLLAMA_HOST` defaults to `http://127.0.0.1:11434`
- `OLLAMA_MODEL` defaults to `qwen3:8b`
