# Dead Code Candidates for #878

This issue does not delete command surfaces or compatibility paths. The list below records candidates found while splitting `bridge_common.rs` so #878 can verify and remove them separately.

## Verified In-Use Command Surface

The agent bridge Tauri commands touched by this split are still referenced from the frontend and are not deletion candidates here:

- `start_agent_session`, `send_agent_message`, `init_agent_sessions`, `interrupt_agent_query`, `close_agent_session`
- `set_agent_permission_mode`, `respond_agent_permission`, `set_agent_model`, `set_session_backend`
- `get_session_page`, `scan_agent_skills`
- `prepare_image_attachment`, `prepare_image_attachments_from_paths`

Reference check: `rg` found frontend `invoke(...)` call sites under `src/hooks/useAgentChat.ts`, `src/hooks/useSessionStore.ts`, `src/components/panels/AgentChatPanel/MessageInput.tsx`, and `src/components/panels/AgentChatPanel/ChatSessionView.tsx`.

## Candidates

| Symbol / path | Current references | Candidate rationale | #878 verification needed |
|---|---|---|---|
| `bridge_common::bridge_script_names` Codex legacy branch | Internal only, via `resolve_bridge_script`; tests assert `CODEX_BACKEND_ID` returns an error. | Codex uses the app-server runtime, and the legacy Node bridge path is deliberately disabled. This branch is a compatibility guard rather than an active runtime path. | Confirm no release/config path can still request a Codex Node bridge, then remove the branch and its compatibility test. |
| `bridge_common::dev_bridge_path` | Internal bridge resolver and tests only; no frontend command surface. | This is a development helper for resolving the unbundled Claude Node bridge. It may remain useful for dev builds, but it is a compat path around the bundled resource resolver. | Decide whether dev builds still need unbundled bridge lookup. If not, collapse into bundled-only resolution and remove related tests. |
| `generated/bridges/claude-sdk-bridge.bundled.mjs` resource path handling | `tauri.conf.json`, `resolve_bridge_script`, build script, tests. | Still required today, but tied to the legacy Node bridge packaging path. | Only remove if Claude runtime no longer uses the Node bridge resource. |

No deletion was performed in this issue.
