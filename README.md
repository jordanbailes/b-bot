# B-Bot

Agentic command line assistant that translates natural language into pre-designated tooling calls. For this lazy programmer who doesn't want to remember stuff like `docker rm -fv $(docker ps -qa)`

`bbot` is a minimal Rust CLI with typed tool-oriented capabilities:

- `docker.remove_all_running_containers`
- `rust.count_lines_of_code`
- `rust.run_coverage`

It uses a planner backend to decide whether the user request matches one of the available tools. Ollama with `qwen3:8b` remains the default local backend, and Codex app-server is available as an alternate backend. Docker cleanup is executed through Docker's API using `bollard`, Rust LOC counting is executed directly through the embedded `tokei` crate, and Rust coverage is executed through the embedded `cargo-tarpaulin` library using the `ptrace` engine.

## Prerequisites

Install and start Ollama before running `bbot` with the default backend.

Linux quick start:

```bash
curl -fsSL https://ollama.com/install.sh | sh
ollama serve
```

In another terminal, pull the default model used by this repo:

```bash
ollama pull qwen3:8b
```

If you want Ollama to run as a background service, the official Linux docs cover the recommended `systemd` setup. For macOS and Windows, use the platform-specific install docs:

- <https://docs.ollama.com/linux>
- <https://docs.ollama.com/cli>
- <https://docs.ollama.com/>

If you want to use Codex instead, make sure the `codex` CLI is installed, on `PATH`, and authenticated or able to complete the interactive login flow when `bbot` launches it.

## Usage

```bash
bbot "delete all running docker containers"
bbot "count rust lines of code in this repo"
bbot "run coverage for this repo"
```

Use Codex as the planner backend:

```bash
bbot --backend codex "delete all running docker containers"
bbot --backend codex --model gpt-5.4-mini "run coverage for this repo"
bbot --backend codex --codex-effort high "count rust lines of code in this repo"
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
bbot --backend codex --codex-network "count rust lines of code in this repo"
```

## Environment

- `BBOT_BACKEND` defaults to `ollama`
- `BBOT_MODEL` is optional and applies to either backend
- `OLLAMA_HOST` defaults to `http://127.0.0.1:11434`
- `CODEX_BINARY` defaults to `codex`
- `CODEX_EFFORT` defaults to `medium`
- `CODEX_NETWORK` defaults to `false`

## Backend Notes

- `--model` defaults to `qwen3:8b` for Ollama.
- `--model` is optional for Codex. If omitted, B-Bot asks Codex app-server for available models and picks a preferred one.
- The Codex backend is planner-only in B-Bot. It is prompted to return a single structured planning decision and B-Bot still owns tool execution.
