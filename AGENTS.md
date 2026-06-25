# AGENTS.md — ZeroClaw

Cross-tool agent instructions for any AI coding assistant working on this repository.

> **Scope:** This file governs `C:/Users/22414/dev/zeroclaw` and all subdirectories.
> **Default branch:** `master` (not `main`; `main` was removed March 2026).
> **Version:** `0.8.0-beta-2`
> **Rust edition:** 2024 · **MSRV:** `1.87` (CI pins `1.93.0`)

---

## ABSOLUTE RULE — SINGLE SOURCE OF TRUTH (NO DRY VIOLATIONS)

**No piece of state lives in two places. Ever. Anywhere in this codebase.**

This is not a guideline. It is not a preference. It is not deferrable to a
follow-up PR. If a fact already lives somewhere in this codebase, you do NOT
copy it into a new field, struct, config block, schema entry, runtime cache,
or anywhere else. You reference it. You resolve it from its source on demand.

**Why this matters more than anything else you're tempted to ship:** every
duplicate state breeds a drift bug whose symptoms surface months later in
production — operator edits the canonical location, the cached copy serves
stale data, the agent silently misbehaves. The previous incarnation of this
codebase had channel `allowed_users` Vec fields cached inside channel handles
while the truth lived in config TOML; reloading config didn't refresh the
channels; an authorized user couldn't talk to the bot until daemon restart.
Every such field is now banned by this rule.

### Forcing mechanism — what happens when you violate

Adding a duplicate state field is an automatic-revert-on-detect change. The
pre-push gate runs `dev/ci.sh dry-check`. If it fires, the maintainer will
`git reset --hard` your branch back to the prior good state, and the time you
spent is wasted. Save yourself the burn: do not write the duplicate in the
first place.

### Pre-edit ritual — before any new struct field, channel/handle field, schema field, config entry

State, in your response text, the source of truth for the new data BEFORE you
write the field. Two valid answers:

  1. **"This is the source of truth — created here."** OK to write the
     field. State what it represents.
  2. **"Source of truth is `<path/to/canonical>` — this would be a
     duplicate."** Do NOT write the field. Resolve from the canonical
     location at use-time (closure, helper, `&Config` parameter, getter
     trait, whatever fits — never a cache).

Any third answer ("we'll only refresh on restart", "snapshot is fine",
"orchestrator passes a Vec in") is a duplicate. Refuse the edit. Find the
canonical source and resolve from there.

### Examples of patterns that ARE duplicate state (forbidden):

- A channel handle struct holding `Vec<String>` of "authorized users" alongside
  `peer_groups` in `Config`.
- A schema enum variant list duplicated across an enum and a `const &[Variant]`
  table that aren't generated from the same macro.
- A `ConfigSnapshot` struct that clones live `Config` fields the runtime can
  already reach through its `Arc<RwLock<Config>>` handle.
- Re-emitting a model-provider's API key into a runtime struct field when the
  runtime already has the typed alias config.

### Patterns that are NOT duplicate state (allowed):

- Resolver closures (`Arc<dyn Fn() -> T + Send + Sync>`) that close over
  `Arc<RwLock<Config>>` and resolve on call.
- `&Config` / `&AgentConfig` parameters threaded through call sites.
- Materialized views built ON-DEMAND from canonical state (cached per-call,
  not stored).
- Derive macros that emit multiple surfaces from one input table (e.g.
  enum + const list from one macro invocation — both come from the same
  source of truth at expansion time).

---

## Project Snapshot

**ZeroClaw** is a Rust-first, local-first autonomous agent runtime. It is distributed as a single configurable binary that talks to 20+ LLM providers, reaches users across 30+ channels, and acts through tools — all on the user's machine with user-owned keys and data.

> "You own the agent. You own the data. You own the machine it runs on."

Core architecture is trait-driven and modular. Extend by implementing traits and registering in factory modules.

### Key extension points (`crates/zeroclaw-api/src/`)

| Trait | File | Use case |
|---|---|---|
| `ModelProvider` | `provider.rs` | New LLM endpoint |
| `Channel` | `channel.rs` | New messaging platform |
| `Tool` | `tool.rs` | New agent capability |
| `Memory` | `memory_traits.rs` | New memory backend |
| `Observer` | `observability_traits.rs` | Metrics/observability sink |
| `RuntimeAdapter` | `runtime_traits.rs` | Runtime integration |
| `Peripheral` | `peripherals_traits.rs` | Hardware boards (STM32, RPi GPIO) |

---

## Repository Map

| Path | Purpose |
|---|---|
| `src/main.rs` | Main `zeroclaw` CLI binary |
| `src/lib.rs` | Library re-exports and CLI command enum |
| `src/bin/zeroclaw-acp-bridge.rs` | ACP bridge binary (requires `acp-bridge` feature) |
| `crates/zeroclaw-api/` | Kernel ABI: public traits |
| `crates/zeroclaw-config/` | TOML schema, config loading/merging, encrypted secrets |
| `crates/zeroclaw-log/` | Unified structured log surface (`record!`, JSONL, broadcast hook) |
| `crates/zeroclaw-spawn/` | Sanctioned `tokio::spawn` wrapper (`spawn!`) preserving attribution spans |
| `crates/zeroclaw-macros/` | Derive macros for config schema and tool/channel registration |
| `crates/zeroclaw-providers/` | LLM client implementations, routing, retry wrappers |
| `crates/zeroclaw-channels/` | 30+ messaging platform integrations + orchestrator |
| `crates/zeroclaw-tools/` | Tool execution surface (shell, file, browser, HTTP, hardware) |
| `crates/zeroclaw-tool-call-parser/` | Normalizes model-side tool-call syntax |
| `crates/zeroclaw-runtime/` | **Transitional holding crate**: agent loop, security, SOP, cron, subagents, observability, TUI, skills, doctor |
| `crates/zeroclaw-memory/` | Memory backends (SQLite default, optional Postgres), embeddings, vector retrieval |
| `crates/zeroclaw-infra/` | Shared infrastructure: debounce, session backend, stall watchdog |
| `crates/zeroclaw-gateway/` | HTTP/WebSocket gateway, REST API, webhook ingress, dashboard serving |
| `crates/zeroclaw-hardware/` | GPIO / I2C / SPI / USB / serial abstraction |
| `crates/zeroclaw-plugins/` | WASM plugin host (Extism), Ed25519 signature verification |
| `crates/robot-kit/` | Specialized robotics hardware support |
| `apps/zerocode/` | TUI onboarding wizard |
| `apps/tauri/` | Tauri v2 desktop app |
| `web/` | React 19 + TypeScript + Vite + Tailwind CSS 4 web dashboard |
| `tests/` | Five-level test taxonomy (`component`, `integration`, `system`, `live`, `manual`, `support`) |
| `docs/book/` | mdbook documentation source |
| `xtask/` | Custom cargo aliases (`mdbook`, `fluent`, `web`) |
| `scripts/`, `dev/` | Build, CI, release, container helpers |

---

## Build Commands

### Standard Rust commands

```bash
# Debug build
cargo build

# Release build (size-optimized; opt-level z, fat LTO, strip, panic=abort)
cargo build --release --locked

# Faster release build for 16GB+ machines
cargo build --profile release-fast --locked

# CI profile build
cargo build --profile ci --locked --target x86_64-unknown-linux-gnu

# Check only
cargo check --all-targets

# Check with all features / no default features
cargo check --locked --features ci-all
cargo check --locked --no-default-features

# 32-bit check
cargo check --locked --target i686-unknown-linux-gnu --no-default-features
```

### Just recipes (preferred task runner)

```bash
just --list
just fmt                 # cargo fmt --all
just fmt-check           # cargo fmt --all -- --check
just lint                # cargo clippy --all-targets -- -D warnings
just test                # cargo test --locked
just test-lib            # cargo test --lib
just ci                  # fmt-check + lint + test
just build               # cargo build --release --locked
just build-debug         # cargo build
just check               # cargo check --all-targets
just doc                 # cargo doc --no-deps --open
just docs                # cargo mdbook serve
just docs-build          # cargo mdbook build
just docs-refs           # regenerate reference docs
just audit               # cargo audit
just deny                # cargo deny check
just fmt-toml            # taplo format
```

### Full local CI

```bash
./dev/ci.sh all          # full local CI in Docker
./dev/ci.sh lint
./dev/ci.sh test
./dev/ci.sh build
./dev/ci.sh security
./dev/ci.sh docker-smoke
```

### Install from source

```bash
./install.sh             # interactive source/prebuilt installer
./install.sh --source --features agent-runtime,channel-discord
./install.sh --skip-quickstart
```

---

## Test Commands and Conventions

### Five-level test taxonomy

| Level | Location | Boundary |
|---|---|---|
| Unit | `#[cfg(test)]` in `src/**` | Everything mocked |
| Component | `tests/component/` | One subsystem real, rest mocked |
| Integration | `tests/integration/` | Real internals, external APIs mocked |
| System | `tests/system/` | Full internal request→response, external APIs mocked |
| Live | `tests/live/` | Real external services, `#[ignore]`'d |

Plus `tests/manual/` (human-driven scripts) and `tests/support/` (shared mocks/fixtures).

### Common test commands

```bash
cargo test                                  # unit + component + integration + system
cargo test --lib                            # unit only
cargo test --test component                 # component only
cargo test --test integration               # integration only
cargo test --test system                    # system only
cargo test --test live -- --ignored         # live tests (requires credentials)
cargo test --test integration agent         # filter within a level
cargo test -- security                      # security-related tests
cargo test -- tools::shell                  # shell-tool tests

# CI uses nextest
cargo nextest run --locked --workspace --exclude zeroclaw-desktop

# Architecture invariant gates
cargo test --test architecture tests_that_persist_config_isolate_the_path
cargo test --test architecture user_facing_strings_route_through_fluent
```

### Live test rules

- Always `#[ignore]`.
- Read credentials from `env::var("ZEROCLAW_TEST_*")`, not operator config.
- Run with `cargo test --test live -- --ignored --nocapture`.

---

## Code Style and Conventions

### Rust edition

- Workspace edition: **2024** (`Cargo.toml` `[workspace.package]`).
- `rustfmt.toml` currently says `edition = "2021"` — treat as stale and prefer the workspace edition.

### Formatting (`rustfmt.toml`)

```toml
max_width = 100
tab_spaces = 4
hard_tabs = false
use_field_init_shorthand = true
use_try_shorthand = true
reorder_imports = true
reorder_modules = true
match_arm_leading_pipes = "Never"
```

### Clippy (`clippy.toml`)

```toml
cognitive-complexity-threshold = 30
too-many-arguments-threshold = 10
too-many-lines-threshold = 200
array-size-threshold = 65536
```

**Critical workspace-wide bans** (`clippy.toml` `disallowed-macros` / `disallowed-methods`):

| Banned | Replacement |
|---|---|
| `tracing::trace/debug/info/warn/error`, `log::*`, `std::dbg`, `anyhow::anyhow` | `::zeroclaw_log::record!` |
| `tokio::spawn` | `::zeroclaw_spawn::spawn!` |

Using the banned macros/methods will fail CI.

### cargo-deny (`deny.toml`)

- Vulnerability advisories: error by default.
- Yanked crates: deny.
- Allowed licenses: MIT, Apache-2.0, BSD-2/3, ISC, Unicode-3.0/DFS, OpenSSL, Zlib, MPL-2.0, CDLA-Permissive-2.0, 0BSD, BSL-1.0, CC0-1.0.
- Multiple crate versions: warn.
- Unknown registries/git sources: deny; only `crates.io-index` allowed.

### Hard project rules

- No `unwrap()` / `expect()` in production paths; propagate errors or document the invariant that makes panic impossible.
- No silent failures — every fallible path must log, return an error, or assert.
- No heavy dependencies for minor convenience.
- No speculative config/feature flags "just in case".
- Do not mix massive formatting-only changes with functional changes.
- Do not modify unrelated modules "while here".
- Do not bypass failing checks without explicit explanation.
- Do not hide behavior-changing side effects in refactor commits.
- Do not suppress unused production code with underscore prefixes or `#[allow(dead_code)]`; delete it, wire it into behavior, or track a follow-up issue. Reserve underscore names for required but intentionally unused API, trait, or callback parameters.
- User-facing strings must route through Fluent (`fl!()`) — never bare literals.
- Log messages, `tracing::` spans/events, and panic messages stay in English with stable `error_key` fields.

---

## Architecture Overview

### Layered architecture

```
External world
  ├── CLI / chat platforms / gateway clients / ACP IDEs
  └── LLM providers

Edge crates
  ├── zeroclaw-channels   (30+ messaging integrations)
  ├── zeroclaw-gateway    (REST / WebSocket / dashboard / webhooks)
  ├── zeroclaw-providers  (LLM clients / retry / routing)
  └── zeroclaw-tools      (browser / HTTP / PDF / shell / hardware)

Core crates
  ├── zeroclaw-runtime    (agent loop, security, SOP, cron, subagents)
  ├── zeroclaw-config     (schema, secrets, autonomy levels)
  └── zeroclaw-memory     (SQLite / Postgres / embeddings)

Support crates
  ├── zeroclaw-api        (kernel traits)
  ├── zeroclaw-log        (structured logging)
  ├── zeroclaw-spawn      (attributed tokio spawn)
  ├── zeroclaw-infra      (debounce, watchdog, session backend)
  └── zeroclaw-macros     (derive macros)
```

### Request lifecycle

1. User → Channel (`deliver_message(ctx)`)
2. Channel → Runtime
3. Runtime → Provider (`chat(messages, tools)`)
4. Provider → Runtime (stream: text / tool_call)
5. Runtime → Security (`validate(tool_call)`)
6. Runtime → Tool (`invoke(args)`)
7. Runtime → Provider (with tool_result)
8. Runtime → Channel → User

### Stability tiers

Every workspace crate carries a stability tier per the Microkernel Architecture RFC.

| Crate | Tier | Notes |
|---|---|---|
| `zeroclaw-api` | Experimental | Stable at v1.0.0 |
| `zeroclaw-config` | Beta | Stable at v0.8.0 |
| `zeroclaw-log` | Beta | — |
| `zeroclaw-spawn` | Beta | — |
| `zeroclaw-providers` | Beta | — |
| `zeroclaw-memory` | Beta | — |
| `zeroclaw-infra` | Beta | — |
| `zeroclaw-tool-call-parser` | Beta | Stable at v0.8.0 |
| `zeroclaw-macros` | Beta | Tightly coupled to config schema |
| `zeroclaw-channels` | Experimental | Plugin migration at v1.0.0 |
| `zeroclaw-tools` | Experimental | Plugin migration at v1.0.0 |
| `zeroclaw-runtime` | Experimental | Transitional holding crate; do not add new functionality here |
| `zeroclaw-gateway` | Experimental | Separate binary at v0.9.0 |
| `zerocode` | Experimental | TUI onboarding wizard |
| `zeroclaw-plugins` | Experimental | WASM plugin system |
| `zeroclaw-hardware` | Experimental | USB discovery, peripherals, serial |

**Tiers**: Stable = covered by breaking-change policy. Beta = breaking changes permitted in MINOR with changelog notes. Experimental = no stability guarantee.
Tiers are promoted, never demoted, through deliberate team decision.

### Feature-flag taxonomy

- `default` — sensible core build.
- `ci-all` — everything on (CI).
- `agent-runtime` — core agent loop + tools + channels + persistence.
- `default-channels` — lean channel bundle (ACP, webhook, email, telegram).
- `channels-full` — historical broad bundle.
- `channel-<name>` — per-channel opt-in (e.g., `channel-discord`, `channel-matrix`).
- `gateway`, `acp-bridge`, `observability-prometheus`, `observability-otel`.
- `hardware`, `peripheral-rpi`, `dev-sim`, `probe`.
- `sandbox-landlock`, `sandbox-bubblewrap`.
- `browser-native`, `rag-pdf`, `plugins-wasm`, `webauthn`, `memory-postgres`.

---

## Security / Safety Constraints

### Autonomy levels

| Level | Capability |
|---|---|
| `ReadOnly` | Read only, no shell/write |
| `Supervised` | Medium-risk ops require approval (default) |
| `Full` | Full access within workspace sandbox |

### Sandboxing layers

1. Workspace isolation — file ops confined to workspace directory.
2. Path traversal blocking — `..` and absolute paths rejected.
3. Command allowlisting — only approved commands execute.
4. Forbidden path list — `/etc`, `/root`, `~/.ssh`, etc.
5. Rate limiting / cost caps.
6. OS-level sandboxes: Landlock (Linux), Bubblewrap, Seatbelt (macOS), Docker.

### Tool receipts

Every tool call produces a signed, chained audit log. See `docs/book/src/security/tool-receipts.md`.

### Secret handling

- Config lives at `~/.zeroclaw/config.toml`.
- Secrets encrypted when `secrets.encrypt = true` (key at `~/.zeroclaw/.secret_key`).
- Env-var override surface: `ZEROCLAW_<dotted_path_with_double_underscores>=<value>`
  - e.g., `ZEROCLAW_providers__models__anthropic__default__api_key=...`
- Legacy env vars (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `ZEROCLAW_API_KEY`, etc.) were **eradicated in v0.8.0**.
- `.env.example` documents the schema-mirror grammar.
- **Never commit** `.env`, API keys, tokens, passwords, `~/.zeroclaw/.secret_key`, or PII.
- Pre-commit hook runs `gitleaks protect --staged --redact` when installed.

### Unsafe / low-level code

- Any new `unsafe` block must include a SAFETY comment explaining the contract and why it is sound.
- Hardware crates (`zeroclaw-hardware`, `robot-kit`) deal with GPIO/I2C/SPI/USB.
- `zeroclaw-plugins` uses Extism/WASM sandboxing.

---

## Development Workflow

### Initial setup

```bash
git clone https://github.com/zeroclaw-labs/zeroclaw.git
cd zeroclaw
git config core.hooksPath .githooks   # enable pre-push hook
```

### Build/test/lint locally

```bash
cargo build
cargo test --locked
./scripts/ci/rust_quality_gate.sh     # fmt + clippy correctness
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

### Run locally

```bash
zeroclaw quickstart               # one-shot setup: pick a provider, write a working config
zeroclaw agent -a <alias>         # interactive chat using the [agents.<alias>] entry
zeroclaw daemon                   # run daemon (gateway + channels + scheduler)
zeroclaw gateway                  # gateway only
cargo run -p zeroclaw-gateway     # gateway from source
zerocode                          # TUI config manager
zeroclaw service install          # register as systemd/launchctl/Windows Service
zeroclaw service start            # run always-on in the background
```

### Containerized development

```bash
./dev/cli.sh up       # build and start dev containers
./dev/cli.sh agent    # enter agent container
./dev/cli.sh shell    # enter sandbox user container
./dev/cli.sh build    # rebuild agent
./dev/cli.sh clean    # stop + remove volumes/config
```

### Docs development

```bash
cargo mdbook serve --locale en
cargo mdbook build
cargo mdbook refs     # regenerate CLI/config reference
cargo mdbook sync     # sync translations
```

### Branch/PR rules

- Target branch: **`master`** only.
- Work from a non-`master` branch. Open a PR to `master`; do not push directly.
- Use conventional commit titles.
- Prefer small PRs (`size: XS/S/M`).
- Follow `.github/pull_request_template.md` fully.
- One concern per PR — avoid mixed feature+refactor+infra patches.
- Stacked PR: declare `Depends on #...`. Replacing old PR: declare `Supersedes #...`.
- Never commit secrets, personal data, or real identity information (see `docs/book/src/contributing/privacy.md`).

### Risk tiers

- **Low risk**: docs/chore/tests-only changes.
- **Medium risk**: most `crates/*/src/**` behavior changes without boundary/security impact.
- **High risk**: `crates/zeroclaw-runtime/src/**` (especially `src/security/`), `crates/zeroclaw-gateway/src/**`, `crates/zeroclaw-tools/src/**`, `.github/workflows/**`, access-control boundaries.

When uncertain, classify as higher risk.

### General workflow

1. **Read before write** — inspect existing module, factory wiring, and adjacent tests before editing.
2. **Map non-trivial changes** — before architecture, config, security, workflow, governance, CI, or agent-assisted contribution changes, read `docs/book/src/contributing/architecture-map.md` to choose the relevant architecture and foundation docs.
3. **One concern per PR** — avoid mixed feature+refactor+infra patches.
4. **Implement minimal patch** — no speculative abstractions, no config keys without a concrete use case.
5. **Validate by risk tier** — docs-only: lightweight checks. Code changes: full relevant checks.
6. **Document impact** — update PR notes for behavior, risk, side effects, and rollback.

---

## Subagents

Subagents (via `spawn_subagent` or cron `JobType::Agent`) inherit the parent's identity and permissions but run in isolated sessions. **Before running any shell commands or filesystem operations, subagents must explicitly set their working directory to the repository root** (the directory containing the top-level `Cargo.toml` and `AGENTS.md`). Do not assume the shell starts at repo root; always `cd` to it first (or use the equivalent in the tool's context).

This guarantees consistent command behavior across parent and child runs.

---

## Localization

- All user-facing output (CLI messages, tool descriptions, onboarding prompts) must use `fl!()` / Fluent strings — never bare string literals.
- Log messages, `tracing::` spans/events, and panic messages stay in English with stable `error_key` fields (RFC #5653 §4.6).
- Panics and `tracing::` lines are never translated.
- The Wiki and internal developer docs are English only.

Dev-operational contracts — files consumed by AI coding skills and development tooling. Do not move or delete without updating all consuming skills and AGENTS.md:

| Protected file | Consuming skill / tool |
|---|---|
| `docs/book/src/contributing/pr-review-protocol.md` | `github-pr-review-session` — review protocol |
| `docs/book/src/maintainers/changelog-generation.md` | `changelog-generation` — release procedure |
| `docs/book/src/maintainers/reviewer-playbook.md` | `github-issue-triage` — triage governance |
| `docs/book/src/maintainers/pr-workflow.md` | `github-issue-triage` — triage discipline |
| `docs/book/src/contributing/privacy.md` | `github-issue-triage`, PR template — privacy rules |
| `docs/book/src/foundations/fnd-00*.md` | `github-pr-review-session` — RFC reference data; public transparency documents |

---

## Skills

AI coding assistant skills live in `.claude/skills/`. Use the right one for the job:

- `.claude/skills/github-pr-review-session/SKILL.md` — PR review co-pilot; assists **you** as the human reviewer. Resolves the active reviewer from session state or `gh`, uses the RFC feedback taxonomy (🔴/🟡/✅/🔵/🟢), and formats formal review findings as H3 headings that start with the taxonomy emoji. Trigger: `review 1234`, `re-review 1234`, `go through the queue`.
- `.claude/skills/changelog-generation/SKILL.md` — generates `CHANGELOG-next.md` between stable tags, resolves contributors via GraphQL, feeds the release workflow. Trigger: `generate changelog`, `release notes for v0.7.x`.
- `.claude/skills/github-issue-triage/SKILL.md` — Issue triage and lifecycle management; manages the backlog, labels, and stale policies. Trigger: `triage issues`, `sweep issues`, `handle issue #N`.
- `.claude/skills/github-issue/SKILL.md` — Interactively files structured GitHub issues (bug reports or feature requests) using repo templates. Trigger: `file issue`, `report bug`, `feature request`.
- `.claude/skills/github-pr/SKILL.md` — Opens or updates GitHub PRs, handles validation evidence, and manages PR descriptions. Trigger: `open PR`, `update PR`, `submit for review`.
- `.claude/skills/skill-creator/SKILL.md` — Framework for creating, testing, evaluating, and optimizing new AI skills. Trigger: `create skill`, `improve skill`, `run skill evals`.
- `.claude/skills/squash-merge/SKILL.md` — Performs conventional squash-merges into master with preserved commit history. Trigger: `squash-merge #123`, `land #789`.
- `.claude/skills/zeroclaw/SKILL.md` — Operational guide for interacting with a ZeroClaw agent instance via CLI or API. Trigger: `check agent status`, `manage memory`, `zeroclaw config`.

---

## Important References

- `docs/book/src/contributing/architecture-map.md` — start-here map for humans and coding agents before non-trivial architecture, workflow, config, security, CI, governance, or agent-assisted contribution changes.
- `docs/book/src/developing/extension-examples.md` — adding providers, channels, tools, peripherals; tool shared-state contract; architecture boundary rules.
- `docs/book/src/contributing/privacy.md` — privacy rules and neutral-placeholder palette.
- `docs/book/src/maintainers/superseding.md` — superseded-PR attribution, PR/commit templates, handoff template.
- `docs/book/src/security/tool-receipts.md` — tool-receipt chain and audit log design.
- `docs/book/src/contributing/testing.md` — five-level testing taxonomy.
- `docs/book/src/foundations/fnd-001-intentional-architecture.md` — architecture decisions.
- `docs/book/src/foundations/fnd-002-documentation-standards.md` — documentation conventions.
- `docs/book/src/foundations/fnd-004-engineering-infrastructure.md` — CI/infrastructure.
- `docs/book/src/foundations/fnd-005-contribution-culture.md` — contribution culture.
- `docs/book/src/foundations/fnd-006-zero-compromise-in-practice.md` — quality bar.
