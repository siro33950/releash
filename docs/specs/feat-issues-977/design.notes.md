# Design Notes

This file is the working implementation plan for discussion. It is intentionally broader than `design.md`; only design decisions that need agreement should be copied into `design.md`.

## Inputs read

- `docs/specs/feat-issues-977/requirements.md`
- `docs/specs/feat-issues-977/behavior.md`
- `docs/architecture/README.md`
- `docs/architecture/DOMAIN.md`
- `docs/architecture/USECASE.md`
- `docs/architecture/GATEWAY.md`
- `docs/architecture/CONTROLLER.md`
- `docs/architecture/TEST.md`
- Current Rust entry points and legacy modules:
  - `src-tauri/src/lib.rs`
  - `src-tauri/src/agent_commands.rs`
  - `src-tauri/src/session_commands.rs`
  - `src-tauri/src/ws_server/handlers.rs`
  - `src-tauri/src/agent_sdk.rs`
  - `src-tauri/src/agent_status.rs`
  - `src-tauri/src/session/`
  - `src-tauri/src/backends/`
  - `src-tauri/src/domain/agent_session/`

## Requirements summary

- Move the `agent_session` responsibilities currently spread across `agent_sdk`, `agent_status`, `session`, and `backends` into the clean architecture layers documented in `docs/architecture/`.
- Keep external resources such as Claude/Codex SDK processes, PTY, file I/O, and Tauri outside the domain layer, behind gateway/infrastructure implementations.
- Remove the old responsibility-bearing modules after migration.
- Preserve the basic desktop and remote Agent session capabilities:
  - start an Agent session
  - stop an Agent session
  - query and broadcast session state
  - broadcast Agent output
  - send messages to a running Agent session
  - interrupt a running Agent turn
  - change model and permission mode
- Do not migrate the dependent `pty_session` or `mcp` domains as part of this spec.
- Frontend and remote client changes are out of scope except for necessary compatibility follow-up.

## Existing implementation snapshot

- `src-tauri/src/domain/agent_session/` exists but currently contains only model-related value objects.
- `src-tauri/src/lib.rs` still wires legacy modules directly:
  - `agent_sdk`
  - `agent_status`
  - `session`
  - `backends`
  - `agent_commands`
  - `session_commands`
- Tauri commands currently depend on legacy runtime and persistence types such as:
  - `AgentProcessMap`
  - `AgentBackendRegistry`
  - `SessionStore`
  - `OpenTabRegistry`
  - `AgentStatusCenter`
- WebSocket handlers currently translate remote messages and call the same legacy Agent/session surface.
- `backends/bridge_common.rs` is very large and mixes command construction, SDK/runtime coordination, session persistence updates, output handling, permission/model concerns, and tests.
- `agent_status.rs` mixes status derivation, aggregation, event emission, and WebSocket broadcasting concerns.
- `session/` mixes domain-like data (`ChatSession`, state, permission/model fields), persistence, lifecycle operations, open tab tracking, and Tauri/data-dir resolution helpers.

## Architecture implementation plan

- Introduce `agent_session` as the primary bounded context under the existing clean architecture layout.
- Domain layer candidates:
  - session identity and lifecycle state
  - turn phase and session aggregate state transitions
  - selected backend/model/permission value objects
  - status read model derivation rules that do not emit events
  - repository/gateway traits needed by usecases
- Usecase layer candidates:
  - start session
  - close session
  - restore session
  - send user message
  - interrupt turn
  - change model
  - change permission mode
  - aggregate/query session status
  - publish output/status changes through abstract notification gateways
- Adapter/controller candidates:
  - Tauri commands for desktop
  - WebSocket handlers for remote
  - request/response DTO conversion
  - conversion from protocol errors to existing wire responses
- Adapter/gateway candidates:
  - file-backed session repository implementation
  - runtime handle registry implementation
  - Agent backend registry gateway
  - Tauri event / WebSocket notification gateway
  - open tab registry gateway if tab state remains part of the Agent session application workflow
- Infrastructure candidates:
  - Claude SDK process bridge
  - Codex SDK process bridge
  - process spawning, stdio, PTY-dependent primitives
  - file I/O helpers used by gateway implementations
- Keep `pty_session` and `mcp` as referenced external contexts. Agent session usecases may call gateway abstractions implemented with existing PTY/MCP infrastructure, but must not absorb those domains.

## Interface implementation plan

- Preserve existing external desktop and remote protocol behavior unless a discussion explicitly accepts a behavior change.
- Keep Tauri command names and WebSocket message names stable where possible, moving their internals to controller/usecase wiring.
- Normalize inbound permission/model/backend values at controller/usecase boundaries with domain value objects.
- Keep remote responses aligned with current `protocol` DTOs:
  - start success/error response shape
  - message response shape
  - interrupt response shape
  - status and output broadcast messages
- Prefer a shared Rust usecase for desktop and remote routes so that entry points differ only in transport conversion.
- Preserve workflow-step session guards and restored session semantics visible to existing workflows.

## Data model implementation plan

- Define the persisted Agent session record as a domain-owned concept, then map it to the current storage format through a repository implementation.
- Preserve externally meaningful fields:
  - session id
  - worktree path
  - messages/history
  - lifecycle state
  - timestamps
  - provider/backend session id
  - permission mode
  - selected model
  - backend id
  - workflow-step session marker
- Separate transient runtime state from persisted session state:
  - process/runtime handles
  - active turn phase
  - output streams/buffers
  - cancellation handles
  - open tab state
- Keep value-object validation for model IDs and permission modes in domain or domain-adjacent value objects.

## Database and persistence implementation plan

- No new database is expected.
- Continue using the current file-backed session persistence unless a later decision changes the storage compatibility requirement.
- Repository/gateway implementations should own data-dir resolution and file I/O details outside domain/usecase.
- Decide whether migration preserves current on-disk JSON compatibility exactly or allows a one-time shape change with migration.

## UI/UX implementation plan

- No intentional frontend feature redesign.
- Desktop and remote flows should continue to expose the same user capabilities from the behavior spec.
- UI changes should be limited to compile-time contract updates caused by Rust command/protocol reshaping.
- If backend errors become more typed internally, external messages should remain compatible unless explicitly changed.

## Algorithm implementation plan

- Extract state transition rules from legacy lifecycle/status code into testable domain logic.
- Extract status aggregation from `AgentStatusCenter` into pure derivation logic, leaving broadcasting to gateways.
- Keep command construction and backend-specific streaming semantics in infrastructure/gateway code, but drive ordering from usecases.
- Decide how cancellation and interrupt semantics are represented:
  - as a usecase operation against a runtime gateway
  - or as a domain state transition plus gateway side effect
- Keep permission/model changes atomic with respect to persisted session state and runtime behavior.

## Infra implementation plan

- Move Claude/Codex bridge implementations under clean architecture paths while preserving current SDK integration behavior.
- Keep process spawning and environment setup outside domain/usecase.
- Keep AppHandle, Tauri emit, and WebSocket broadcaster usage in adapter/gateway wiring.
- Keep `lib.rs` composition root responsible for constructing usecases and concrete gateways.
- Remove or turn legacy modules into temporary compatibility shims only during migration; final state should not keep old modules as responsibility owners.

## Test implementation plan

- Domain tests:
  - value objects for backend/model/permission
  - session lifecycle transitions
  - turn phase and status derivation
  - aggregation edge cases
- Usecase tests:
  - start/stop/message/interrupt/model/permission flows with fake repositories and runtime gateways
  - persistence is not mutated on invalid input
  - workflow-step session guards are preserved
  - desktop and remote request paths share behavior through the same usecases
- Gateway/infrastructure tests:
  - command construction and backend-specific bridge behavior without invoking external processes
  - file-backed repository compatibility with current session data
- Controller tests:
  - DTO conversion
  - WebSocket response shapes
  - Tauri command error mapping
- Existing `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` should be used for Rust verification.

## Design decisions likely requiring discussion

- Whether to preserve the current persisted session JSON shape exactly or allow a migration to a new domain-owned schema.
- Whether open tab tracking belongs inside `agent_session` usecases or remains an adjacent UI/session gateway.
- Whether status aggregation should be represented as part of the Agent session domain or as an application read-model usecase.
- Whether backend registry selection is a domain concept (`BackendId` and supported models) or purely infrastructure configuration.
- Whether final migration removes legacy module names in one change or keeps compatibility shims temporarily while responsibility moves.
- Whether desktop and remote entry points must remain strictly identical in semantics, including all current edge cases, or only preserve the behavior-spec surface.
- How much of workflow-step session behavior is part of `agent_session` versus workflow integration.
- How to expose typed errors internally while preserving current string-based Tauri command and WebSocket wire contracts.
- Whether runtime handle/cancellation state is modeled as a domain runtime session or only as a gateway concern.
