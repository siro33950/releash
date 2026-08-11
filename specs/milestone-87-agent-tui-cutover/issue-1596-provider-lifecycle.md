# Issue #1596 Provider lifecycle integration specification

## Status

Accepted implementation specification for GitHub Issue #1596 in Milestone 87.

This specification is the implementation goal. Implementation is complete only when every requirement, Red-Green-Refactor cycle, deletion, acceptance case, and quality gate in this document is satisfied.

## Sources of truth

- GitHub Milestone 87
- GitHub Issue #1596 body and all comments
- GitHub Issues #1594 through #1599 and their responsibility boundaries
- `specs/milestone-87-agent-tui-cutover/acceptance-contract.md`
- repository `AGENTS.md`
- `src-tauri/AGENTS.md`
- `.claude/rules/rust-first-logic.md`
- `docs/architecture/README.md`
- `docs/architecture/DOMAIN.md`
- `docs/architecture/USECASE.md`
- `docs/architecture/CONTROLLER.md`
- `docs/architecture/GATEWAY.md`
- `docs/architecture/INFRASTRUCTURE.md`
- `docs/architecture/TEST.md`
- official Claude Code and Codex Hook documentation

Where the existing glossary describes the pre-Milestone-87 Message or Terminal model, the Milestone 87 acceptance contract and Issue decisions take precedence.

## Required development method

Every observable behavior change must use a complete Red-Green-Refactor cycle.

1. RED
   - Add a test that expresses the accepted external behavior.
   - Run the smallest relevant test command.
   - Confirm that the test fails for the intended missing or incorrect production behavior.
   - A compile failure caused only by an unfinished test is not sufficient when a runnable black-box failure can be written.
2. GREEN
   - Add the smallest production implementation that satisfies the failing behavior.
   - Run the focused test and confirm it passes.
   - Run the directly related test module or integration target and confirm no regression.
3. REFACTOR
   - Remove duplication, old branches, dead code, and invalid dependencies while all tests remain Green.
   - Re-run the focused and related tests after refactoring.

Production behavior must not be written before its RED has been observed. Test expectations must not be changed to match an incorrect implementation. Physical deletion of the old Hook implementation is the refactor after the replacement path is Green; it is not deferred to another Issue.

## Issue responsibility

Issue #1596 owns all of the following:

- Claude and Codex Provider lifecycle signal ingestion.
- Provider session identity association.
- Opaque Provider transcript reference association.
- Validated Stop association.
- Exact AgentSession and NodeExecution attempt correlation.
- Per-launch Hook configuration and launch binding.
- The hidden Hook-only Releash CLI command.
- The authenticated Local API endpoint used by the Hook CLI.
- Durable Provider lifecycle facts.
- Fail-closed handling of unavailable or invalid lifecycle signals.
- Removal of the explicitly rejected workflow CLI commands.
- Physical deletion of the replaced legacy Claude-only Hook implementation, configuration, frontend entry points, tests, and documentation.

Issue #1596 does not own:

- Applying validated Stop or Submit to workflow Node completion or WaitingApproval. Issue #1598 owns those transitions.
- Automatically marking missing Stop as Stalled after a workflow deadline. Issue #1598 owns the workflow transition.
- Killing an AgentSession or Terminal Surface when a Node becomes complete.
- Connecting the actual Claude or Codex TUI process to a durable Terminal Surface. Issue #1597 owns that vertical slice and consumes the launch boundary produced here.
- Resume or continuity guarantees after the Releash application process or computer restarts.
- Migration or cleanup of legacy persisted data or previously modified user settings.
- The remaining legacy GUI, Message projection, or Agent runtime removal that is unrelated to the Hook implementation. Issue #1599 owns those removals.

The #1596 and #1599 Issue comments must record this boundary. Old Hook removal must not remain assigned to #1599.

## End-to-end production path

The required path is:

```text
Claude / Codex Hook
    -> hidden Releash Hook CLI
    -> dynamically discovered authenticated Local API
    -> API controller
    -> Provider lifecycle usecase
    -> Provider lifecycle domain model
    -> durable local event store
```

The acceptance test must use this production path. A fixture-only TCP capture, direct Domain call, direct repository call, or controller-only test does not satisfy the product acceptance requirement.

## Lifecycle correlation contract

Each active launch binding must identify exactly:

- A Provider lifecycle Slot identity that remains stable across retry attempts of the same logical execution lane.
- Provider kind: Claude or Codex.
- Releash AgentSession identity.
- Workflow execution identity.
- NodeExecution identity.
- NodeExecution attempt number.
- A launch-specific binding identity.
- A launch-specific capability that is available only to the launched Provider process.

`ProviderLifecycleSlotId` is opaque to the Provider lifecycle domain. The launch owner supplies it and reuses it across retry attempts of the same logical execution lane. `NodeExecutionId`, AgentSession identity, and attempt are audit scope fields and must not be used as a substitute for this stable identity. Independent normal Nodes and fanout child lanes use different Slot identities.

Each Slot retains only its current launch binding. Arming a new binding for the same Slot atomically expires the previous binding and arms the replacement. Historical bindings are not retained in the live registry merely to classify stale input; a binding that does not equal the Slot's current binding is rejected as not current. The durable `BindingExpired` fact remains the authority for the replacement history.

The lifecycle domain model owns these invariants:

- The first valid SessionStart associates the Provider session identity with the launch binding.
- A transcript path is stored only as an opaque Provider-owned reference.
- Releash never reads, parses, copies, mirrors, or owns the Provider transcript body.
- Repeated delivery of the same SessionStart is idempotent.
- Repeated delivery of the same Stop is idempotent and cannot create a second Stop fact.
- A different Provider session identity cannot replace the identity already associated with a binding.
- A signal for another AgentSession, workflow execution, NodeExecution, or attempt is rejected.
- A signal using an expired, superseded, or invalid launch binding is rejected.
- A signal carrying the real binding, capability, and scope from a previous retry attempt is rejected without adding an accepted lifecycle fact.
- Process exit is not Provider Stop.
- Visible terminal text, shell prompts, and process output are not lifecycle signals.
- A missing or rejected signal cannot create an inferred Stop.
- Validated Stop does not kill the AgentSession, PTY, or Terminal Surface.
- Validated Stop does not itself mutate workflow Node state in #1596.

The Domain owns all correlation and state transition decisions. The live aggregate is one `ProviderLifecycleSlot` containing at most one current `ProviderLifecycleBinding`. The Usecase owns the per-Slot sequence `lock current Slot -> create Domain candidate -> atomically append scoped facts -> publish candidate`; a persistence failure discards the candidate and leaves the prior live Slot unchanged. Gateway implementations provide credential issuance and durable event I/O primitives only. Command paths use Domain entities and value objects; they do not introduce QueryService DTOs.

Slot locking is independent per Slot. The implementation must not clone the complete live registry or hold one registry-wide lock across durable I/O. Explicit AgentSession or Provider termination may release the current binding after a durable expiry fact is committed. Release identifies both Slot and expected binding so a delayed termination for an old binding cannot release its replacement. Provider Stop and workflow Node completion do not release a Slot or terminate the AgentSession, PTY, or Terminal Surface.

## Durable lifecycle facts

Provider lifecycle persistence must extend the existing SQLite local event store and reuse its stream versioning, optimistic concurrency, and idempotent operation support. A separate JSON file or parallel lifecycle database must not be introduced.

Durable facts include:

- Launch binding armed.
- Provider session identity associated.
- Opaque transcript reference associated or updated according to the Domain invariant.
- Validated Stop observed.
- Binding expired or superseded.
- Rejection or fail-closed reason when it is a lifecycle fact needed for diagnosis.

Durable data must not include:

- Transcript contents.
- Provider conversation contents.
- Terminal output contents.
- Entire raw Hook stdin payloads.
- Raw Local API bearer tokens.
- Raw launch capability secrets.
- Global Provider configuration snapshots.

Provider lifecycle events are represented by a dedicated Domain event family and encoded through a versioned adaptor gateway codec. Infrastructure stores raw serialized local event records and must not import inner-layer Domain types.

Replacing a binding may expire a binding scoped to an older AgentSession and arm a binding scoped to a newer AgentSession. Those scoped facts must be committed as one atomic local event batch across the affected streams.

## Provider protocol boundaries

Infrastructure owns only raw stdin, process environment, command argument, and HTTP mechanics. Provider-specific JSON conversion belongs in adaptor gateways. External HTTP request and response structures belong in adaptor protocol. Controllers convert protocol values and call the Usecase; they do not query repositories directly and do not perform lifecycle decisions.

Claude support includes:

- SessionStart.
- Stop.
- StopFailure.

StopFailure is a diagnostic failure signal and must never be converted into Stop.

Codex support includes:

- SessionStart.
- Stop.

Provider transcript fields remain nullable opaque references. Provider-specific fields that are not needed for the lifecycle contract are not retained in the Domain model.

## Per-launch configuration and trust

Releash must not mutate Claude or Codex global user configuration for the new mechanism.

- Hook configuration applies only to a Provider process launched by Releash.
- Local API ports are discovered dynamically and are not written into global Provider configuration.
- Local API bearer tokens and launch capability secrets are not embedded as fixed values in Hook command definitions.
- Launch capability is supplied only through the launched process environment.
- Hook command text is stable for trust evaluation.
- Codex Hook trust must be respected.
- `--dangerously-bypass-hook-trust` must never be used.
- If required Codex Hook trust is absent and SessionStart is not delivered, Releash fails closed with a diagnosable reason.

Before selecting the Claude launch mechanism, a characterization RED must run against the supported installed Claude CLI and prove:

- Existing user Hooks continue to run.
- The Releash Hook is active only for the Releash-launched process.
- The user's settings file remains byte-for-byte unchanged.

If Claude `--settings` replaces existing user Hooks, that mechanism must not be used. A session-only plugin or other official per-process configuration source that preserves user Hooks must be selected and tested instead.

## CLI contract

The following workflow CLI commands must be removed:

- `releash workflow list`
- `releash workflow start`
- `releash workflow executions`
- `releash workflow logs`
- `releash workflow approve`
- `releash workflow abort`
- `releash workflow stop`
- `releash workflow resume`
- `releash workflow output validate`

The following commands remain:

- `releash workflow status`
- `releash workflow output submit`
- `releash workflow output get`
- Existing review commands used by workflow Nodes.

The new Hook-only commands are:

```text
releash hook receive --provider claude
releash hook receive --provider codex
```

The Hook command must parse but must not appear in root help, nested normal help, or the Agent-facing `render_long_help()` output.

Removal of workflow CLI commands removes only their CLI variants, dispatch, CLI-only helpers, fallbacks, and tests. Local API routes and Rust usecases still used by the GUI, workflow runtime, or another supported surface must not be removed merely because their CLI command was removed.

## Hook CLI and Local API behavior

The Hook CLI owns only:

- Bounded stdin reading.
- Explicit Provider selection.
- Provider-specific stdin parsing delegation.
- Local API discovery on every invocation.
- Local API bearer authentication.
- Launch capability forwarding.
- Provider-specific stdout and exit-code adaptation.

The Hook CLI must not contain lifecycle state decisions or workflow transition logic.

The Local API must:

- Use the existing dynamic loopback discovery and bearer authentication mechanism.
- Require the launch-specific capability in addition to ordinary Local API authentication.
- Reject malformed, unauthenticated, stale, cross-binding, and cross-attempt input without mutating accepted lifecycle state.
- Return a structured result that preserves the fail-closed reason for diagnosis.
- Never turn process exit or missing input into Stop.

Hook failure handling must not force the Provider TUI to terminate. Provider-required stdout must remain valid; Codex Stop success returns valid JSON such as `{}`. Failure remains fail closed on the Releash side: no validated lifecycle fact means no workflow progress.

## Legacy Hook physical deletion

After the new production path is Green, the same Issue and pull request must remove:

- The legacy Claude-only Domain Hook configuration generator.
- The legacy settings repository that mutates Claude global settings.
- Generated `curl` commands and `/hooks/agent` references.
- `server.hook_port` and its defaults, validation, serialization, UI, and tests.
- Legacy Hook-specific configuration tokens not used by the new Local API.
- Legacy generate, apply, and status Tauri commands.
- Tauri command registration and state wiring used only by the legacy Hook.
- Frontend invokes, types, hooks, and controls used only by the legacy Hook.
- Legacy Hook tests and fixtures.
- Documentation describing the removed path.
- Modules, exports, imports, and dependencies that become unreachable.

No migration scans or modifies an existing user settings file. This explicit no-migration rule does not permit legacy production code, Config fields, commands, or UI to remain in the repository.

Completion requires a repository audit proving production source no longer contains the legacy `hook_port`, `/hooks/agent`, legacy Hook command registration, or legacy settings generation path.

## Red-Green-Refactor implementation sequence

### Cycle 1: CLI surface

RED proves rejected commands currently parse, retained commands remain valid, and the missing hidden Hook command does not yet satisfy the contract. GREEN changes the command surface. REFACTOR removes only unreachable CLI-specific code.

### Cycle 2: lifecycle Domain

RED covers all lifecycle invariants and fail-closed cases, including a real previous-attempt binding being superseded by a replacement in the same Slot. GREEN adds the Slot aggregate, binding entity, stable Slot identity, value objects, errors, and outcomes. REFACTOR removes historical live binding retention and confirms the Domain has no infrastructure dependency.

### Cycle 3: durable events

RED covers round-trip encoding, replay, idempotency, stale version conflict, duplicate observation, cross-AgentSession replacement atomicity, and secret/body non-retention. GREEN extends the existing local event store path. REFACTOR aligns codec and repository dependencies with architecture rules.

### Cycle 4: Provider gateways and launch configuration

RED covers exact Claude and Codex payload conversion, StopFailure, nullable transcript references, per-process configuration, trust constraints, and unchanged user settings. GREEN adds the minimum Provider gateways and launch specification. REFACTOR shares only genuinely Provider-independent mechanics.

### Cycle 5: Hook CLI and Local API vertical slice

RED invokes the Hook CLI against the production Local API and proves the route/usecase is missing. GREEN connects CLI, discovery, authentication, protocol, controller, usecase, Domain, and repository. REFACTOR removes the high-level lifecycle Gateway, keeps lifecycle decisions out of CLI and controller, and verifies that different Slots do not block each other.

### Cycle 6: legacy Hook removal

With the replacement path Green, remove the complete legacy Hook path. Add or update behavioral tests first for removed public commands, removed serialized Config, unchanged user settings, and absence of frontend entry points. Treat deletion of unreachable internals as REFACTOR and keep all replacement tests Green.

### Cycle 7: production acceptance

RED adds the complete acceptance matrix through the real CLI subprocess and production Local API. GREEN fills only remaining behavior gaps. REFACTOR consolidates shared test fixture support without replacing product acceptance with fixture self-tests.

## Product acceptance matrix

The integration test must use:

```text
agent_tui_fixture
    -> actual releash CLI subprocess
    -> authenticated production Local API router
    -> production controller/usecase/domain
    -> SQLite lifecycle ledger
```

Both Claude and Codex must cover:

- Correct SessionStart association.
- Correct Provider session identity.
- Correct opaque transcript reference.
- Correct Stop association.
- Duplicate SessionStart.
- Duplicate Stop.
- Delayed Stop.
- Missing SessionStart.
- Missing Stop.
- Previous-attempt signal using the actual superseded binding, capability, and scope after a replacement is armed in the same Slot.
- Other-AgentSession signal.
- Other-NodeExecution signal.
- Malformed payload.
- Invalid or stale launch capability.
- Missing Local API discovery.
- Provider process exit without Stop.
- Visible terminal output that resembles a Stop message.
- No workflow transition in #1596.
- No AgentSession, PTY, or Terminal Surface kill operation.
- No transcript content persisted.

The existing fixture self-test remains useful but cannot be the acceptance evidence for #1596.

## Quality gates

Focused RED and GREEN commands must be recorded during implementation. After all cycles, run:

The installed Provider characterization gate is pinned to Claude Code `2.1.220 (Claude Code)` and Codex CLI `0.145.0`. Codex `SessionStart` is verified at the first user prompt, after explicit Hook trust; the gate never uses `--dangerously-bypass-hook-trust`.

```bash
cd src-tauri
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked --test provider_lifecycle_acceptance_test
cargo test --locked --test provider_lifecycle_characterization_test -- --ignored --nocapture
cargo test --locked --test agent_tui_harness
cargo test --locked
cargo deny --locked check
cargo build --locked

cd ..
pnpm lint
pnpm test
pnpm build
pnpm test:integration
qlty check --no-progress --all
```

Finally re-read Issue #1596, its comments, Milestone 87 acceptance contract, repository rules, and architecture documents. Any discovered nonconformance is part of this specification and must be corrected with another complete Red-Green-Refactor cycle before the Goal is complete.
