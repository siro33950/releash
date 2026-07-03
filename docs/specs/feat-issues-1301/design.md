# Design

本書は `docs/specs/feat-issues-1301/requirements.md` / `behavior.md` を受けて、Agent backend（Claude / Codex）ごとの個別実装を infrastructure 層に閉じ込め、backend 実装が agent_session の Entity へ変換してから返す構造への移行方針を確定する。

**対応方針（Y-statement）**: 「backend 固有の振る舞いは各 backend infrastructure の内側に閉じ、Agent session 実行側は Entity と backend interface だけを扱う」という behavior 定義を実現するために、(1) domain/agent_session に GLOSSARY 正規 Entity と backend interface（trait）を新設し、(2) Claude / Codex それぞれの infrastructure 実装を公式 wire 仕様から再構築して Entity へ直接変換させ、(3) 現在 `infrastructure/agent_session/runtime/runtime_support/` に集約されている「Claude と Codex を同じ処理へ流す共有実装」を、Entity を消費する backend 非依存の実行側（usecase + adaptor）と、各 backend 実装内部の処理とに分解・移送する。既存保存済み session の後方互換は要求されない（requirements Constraints）ため、互換レイヤーは作らない。

---

## 0. 前提と用語

- 本書のコードパスは断りがない限り `src-tauri/src/` からの相対。行番号は 2026-07-02 時点の worktree `feat/issues/1301` の実測。
- **Entity**: GLOSSARY（`docs/architecture/GLOSSARY.md:30-36`）が定める agent_session の正規語 — Session / Turn / Message / MessagePart / MessageRole / PermissionRequest / Attachment。
- **backend**: Agent 実行系の実装単位。本 Issue の対象は `claude` と `codex` の 2 つ（Non-goals: 新 backend は追加しない）。
- **backend runtime**: 1 つの Session に対する backend 側の実行実体（プロセス・接続・復旧を内包する per-session オブジェクト）。
- **実行側（host）**: backend interface を呼び出し、返ってきた Entity を永続化・投影・配信する backend 非依存の層。usecase（判定ロジック・業務手順）と adaptor（非同期駆動・presenter・controller）で構成する。

### 0.1 本設計の追加前提（発注者指示。requirements への追加拘束として扱う）

1. `docs/architecture/` を厳守する。
2. Claude / Codex 固有の実装は既存実装を信用せず、各種公式ドキュメント・OSS 実装を参考に、最も表現力があり最も正しく最もシンプルな実装を行う。
3. ユーザーの体験する振る舞いを大きく変更してはいけない。軽微な変更や API 等の破壊的変更はしてよい。
4. 後方互換性を考慮する必要はない。

D3（Node bridge 廃止）はこの前提 2 に基づく決定である（§13 参照）。

### 0.2 外部仕様の根拠

- Claude: 公式 headless / Agent SDK ドキュメント（https://code.claude.com/docs/en/headless, /en/cli-reference, /en/agent-sdk/permissions, /en/agent-sdk/sessions, /en/agent-sdk/user-input）および `@anthropic-ai/claude-agent-sdk`（npm 0.3.x の `sdk.d.ts`）・`anthropics/claude-agent-sdk-python`（`subprocess_cli.py` / `query.py`）の実装。CLI 最低バージョン 2.0.0。
- Codex: `openai/codex` リポジトリ `codex-rs/app-server/README.md` と `codex-rs/app-server-protocol`（crates.io に `codex-app-server-protocol` として公開済み）。**app-server protocol は codex バイナリとロックステップで進化するため、実装時にサポート対象の codex CLI バージョンを確定し、`codex/wire.rs` の冒頭コメントに記録する。wire 型は当該バージョンの protocol 定義に合わせ、本書 §7 の変換表と差異がある場合は M2 の冒頭で実プロセスに対して受理形式を確認してから実装する**（§7.5 の検証項目）。

---

## 1. 決定事項（D1〜D16）

| # | 決定 | 根拠（requirements / behavior / architecture） |
|---|---|---|
| D1 | GLOSSARY 正規 Entity を `domain/agent_session/entities/` に新設する。domain 層は serde / tokio を使わない（`docs/architecture/DOMAIN.md`）。自由形式 JSON は value object `JsonPayload`（有効な JSON テキストの newtype。検証は構築側=境界層の責務で、domain 内では再検証しない）として保持する。**Turn は struct として新設せず、`TurnId` + event log（`TurnStarted`..`TurnCompleted` イベント列）で表現を維持する**（issues-1247 の保存正典合意を優先する設計判断。GLOSSARY 上の Turn Entity はこの形で所有される） | requirements:7,35 / DOMAIN.md 原則 |
| D2 | backend interface を `domain/agent_session/gateway.rs` に定義する。`AgentBackend`（backend 単位）と `AgentSessionRuntime`（session 単位）の 2 trait 構成。イベントは `Stream<Item = AgentRuntimeEvent>` で返す（DOMAIN.md「Gateway trait は Stream を返す形式」） | requirements:9,36 / behavior「backend infrastructure は agent_session の Entity を返す」 |
| D3 | Claude backend は Node bridge（`resources/claude-sdk-bridge.mjs` + `@anthropic-ai/claude-agent-sdk`）を廃止し、`claude` CLI を直接 spawn して公式 stream-json + control protocol を Rust で実装する。公式 SDK 自体が同プロトコルの薄いラッパーであり、これが最も正しく最もシンプルな公式統合面である（§0.1 前提 2、§13 に代替案） | 前提 2 / requirements:39 |
| D4 | Codex backend は既存の app-server v2 JSON-RPC 統合を土台に再構成し、**Claude 方言の合成を全廃**する: `permission_request` 互換 JSON 合成（`codex_app_server.rs:675-702`）、Claude SDK `result` message 互換 JSON の合成と `agent-sdk-message` emit（`codex.rs:263-281,589-593`）、共有 stdin コマンド（`{"type":"setModel"}` / `{"type":"interrupt"}` / `{"type":"close"}`）の Codex プロセスへの書き込み、を全て排除し Entity へ直接変換する | requirements:38 / behavior「Codex app-server event は Codex infrastructure が直接変換する」 |
| D5 | 実行側を `usecase/agent_session/runtime/`（判定ロジック・業務手順）+ `adaptor/gateway/agent_session/runtime_driver/`（tokio 駆動）+ `adaptor/presenter/agent_session.rs`（Tauri emit）に再編する。`runtime_support/` の全 14 ファイル（`mod.rs` / `session_lifecycle.rs` / `claude_sdk_message.rs` / `shared.rs` / `external_agent.rs` / `stream_emit.rs` / `permission.rs` / `recovery.rs` / `session_persistence.rs` / `model_selection.rs` / `skills.rs` / `process_registry.rs` / `system_context_rendering.rs` / `turn_event_log.rs`）は本再編で解体する。移設先は §2.1 の対応表に従う | requirements:33-34 / README.md 依存方向 |
| D6 | `PermissionRequest` を型付き Entity として新設する。`MessagePart::Permission { request: serde_json::Value, status: String }`（`usecase/agent_session/session/mod.rs:123-134`）と `AgentSessionEvent::PermissionRequested { request: Value }`（`usecase/agent_session/event_log/events.rs:224-229`）の生 JSON 表現を置き換える。応答は `PermissionResponse` Entity で受け、backend 固有形式への変換は各 backend 実装内で行う | requirements:40 / behavior「permission と approval payload は共通の PermissionRequest 振る舞いとしてだけ露出する」 |
| D7 | MessagePart の tool 語彙（`Bash` / `Edit` / `WebSearch` / `mcp__<server>__<tool>` 等）は **Releash の共通表示語彙**として本書 §5.4 で正典化する。feat-agent-native-parity の正規化マップ（Codex item → 共通 MessagePart）は維持する。これは「Claude wire 形式の中間 message」ではなく Entity 契約の一部である | feat-agent-native-parity triage/goal の合意 / requirements Non-goals（語彙整理は主目的にしない） |
| D8 | model カタログ（一覧・表示名）は各 backend 実装が所有する。`domain/agent_session/value_objects/agent_models.rs`（`CLAUDE_FIXED_MODELS` / `CODEX_FIXED_MODELS` / `model_display_name` の backend 文字列 match）は削除し、`AgentBackend::available_models() -> Vec<ModelDescriptor>` に移す。一覧が「Rust 定数で完全固定」という現行プロダクト決定は維持する（一覧の中身・順序・表示名は不変） | requirements:42 / behavior「model 取得と解釈は backend infrastructure の内側に閉じる」 |
| D9 | skill 解決・fuzzy file search は backend capability として trait に載せ、`read_codex_skill_catalog` / `read_codex_mentionable_files` という backend 固有 Tauri command と frontend の `currentBackendId === "codex"` 実行分岐（`src/components/panels/AgentChatPanel/MessageInput.tsx:273-286,318-327`）を廃止する。SKILL.md frontmatter の純テキストパース（I/O なし）は domain service（`domain/agent_session/services/skill_frontmatter.rs`）に置き、走査対象ディレクトリの決定と I/O は各 backend 実装が所有する | requirements:33,50-51 |
| D10 | `backend_id` 欠落時の `CLAUDE_BACKEND_ID` 暗黙フォールバック（`session_persistence.rs:222-224,331-344`、`session_lifecycle.rs:182,650,1903,2340,2519-2522`、`model_selection.rs:188-191`、`external_agent.rs:342` の CODEX フォールバック）を全廃する。`SessionMeta.backend_id` は必須フィールドとし、欠損 session はエラー（invalid session 隔離、issues-947/1254 と同方針） | requirements:37,48 |
| D11 | `agent-sdk-message` イベント（Claude SDK bridge 生 JSON の frontend への素通し、`claude_sdk_message.rs:1488` 等）を廃止し、型付きイベント（`agent-turn-usage-updated` 等）に置き換える。現行 frontend listener（`src/hooks/useAgentSdkListeners.ts`）が担っていた表示（token usage / turn 失敗エラー / system テキスト）は全て Rust 側の Entity 経路（§6.2 / §8.2）から供給し、frontend の SDK 形状パースを撤去する | requirements:50 / behavior「backend native 値を frontend や workflow の domain logic にしない」 |
| D12 | `AgentBackendRegistry` は usecase 層（`usecase/agent_session/backend_registry.rs`）へ移設する。registry は「backend_id → `Arc<dyn AgentBackend>` の解決」「default backend 解決」「model entry 解決」のみを担う。backend_id の使用箇所は session metadata と registry / dispatch 境界に限定する | requirements:37,52 |
| D13 | 共有してよいものは (a) Entity 定義、(b) backend interface、(c) domain service（純関数）、(d) backend 意味論を含まない OS プロセス汎用 utility（PID ファイル管理・orphan cleanup・`CleanupGate`・child env 準備 = `infrastructure/process/`）に限る。**(d) は requirements:9 の限定列挙に対する本設計の解釈拡張であり、受け入れ時の争点として明示的に記録する**: これらは agent の実行・変換・復旧の意味論を持たない OS 資源管理（tokio や std と同格の基盤）であり、各 backend への複製は README.md 横断原則「同じ操作の実装は 1 つに集約する」に反するため共有とする。復旧の**方針**（いつ interrupt するか・いつ再 spawn するか・resume の再試行）は各 backend / 実行側が所有し共有しない | requirements:9,34,41 の解釈。§13 に代替案 |
| D14 | Entity / event 語彙から backend・実装由来語を排除する: `InterruptReason::BridgeCrash` → `InterruptReason::Crash`、`AgentRuntimeError::StartupTimeout` の Codex 固有 Display 文言（`runtime/mod.rs:173-180`）→ 中立文言、stale timeout の利用者向け文言 `"Claude 応答が停止したため中断しました。…"`（`recovery.rs:55-57`）→ 中立文言（「エージェントの応答が停止したため中断しました。もう一度お試しください。」） | behavior「backend 固有の source 値は workflow surface に露出しない」 |
| D15 | 保存形式は現行の dir layout（issues-1213/1247/1249 の合意）を維持し、変わるのは (a) `SessionMeta.backend_id` 必須化、(b) events.json / message parts 内の Permission 表現の型付き化、(c) `InterruptReason` の語彙、のみ。migration は行わず、読めない旧 session は既存の invalid session 隔離（`session_storage.rs:28-31`）に落ちる | requirements Constraints:48-49 |
| D16 | 既存のユーザー可視挙動（通常実行 / permission 応答 / model 選択 / 復帰 / workflow 実行の意味）は維持する。変更してよいのは API・イベント名・payload 形状（breaking 可）と、実挙動と乖離していた表示。**本 Issue に含める軽微なユーザー可視変更は次の 2 点のみ**: (a) stale timeout エラー文言の中立化（D14）、(b) Codex の "Steer active turn" ラベル（steering は `codex.rs:1736-1742` で無効化されており実挙動は queue。capability 由来の表示に修正し実挙動と一致させる） | requirements:53 / 前提 3 |

review 追記: `events.json` は現行の JSON 配列形式と in-place 追記を維持する。event log は turn 中に高頻度で追記されるため、temp+rename による配列全体書換へ戻すと session 長に比例した書換コストが復活する。一方で、in-place 追記は閉じ括弧 `]` を一時的に削ってから payload と `]` を書き戻すため、その間のクラッシュで末尾が壊れる可能性がある。これに対し `read_session_events_from_dir` は、末尾の非空白文字が `]` でない場合のみ復旧モードに入り、最後の完全な要素までを JSON 配列として補完して読み込む。完全でない末尾要素は破棄され、既に永続化済みのイベント列を復元する。

---

## 2. 全体構成

### 2.1 モジュール構成(変更後)と移設対応表

```
src-tauri/src/
├── domain/agent_session/
│   ├── entities/
│   │   ├── session.rs            # Session（メタ状態の domain 表現）, SessionState, ContextCarryState
│   │   ├── turn.rs               # TurnId, TurnResult, TurnStopReason, InterruptReason, TokenUsage
│   │   ├── message.rs            # Message, MessageRole
│   │   ├── message_part.rs       # MessagePart（+ 差分マージ規則 merge_part）
│   │   ├── permission_request.rs # PermissionRequest, PermissionRequestBody, PermissionRequestStatus,
│   │   │                         # PermissionDecision, PermissionQuestion, PermissionResponse
│   │   └── attachment.rs         # Attachment（ref）, AttachmentPayload（base64 入力）
│   ├── value_objects/            # 既存 + JsonPayload, ModelDescriptor, BackendCapabilities,
│   │   │                         # SlashCommand, EditorContext, TodoListItem, SystemNotificationType,
│   │   │                         # ToolOutputRef, ToolOutputSummary
│   │   └── (agent_models.rs は削除。skill_entry.rs から serde を除去)
│   ├── gateway.rs                # AgentBackend / AgentSessionRuntime trait, AgentRuntimeEvent,
│   │                             # SessionSpec, TurnInput, ForkSessionRequest, AgentBackendError
│   ├── repository.rs             # 旧 storage.rs（AgentSessionReader/Writer。関連型・String エラーは現状維持 = 既存
│   │                             # 逸脱の温存。DomainError 化は別 Issue とする）
│   └── services/                 # 既存（context_replacement 等）+ skill_frontmatter.rs（D9）
│
├── usecase/agent_session/
│   ├── backend_registry.rs       # AgentBackendRegistry（dispatch 境界。infrastructure/runtime/mod.rs から移設）
│   ├── runtime/
│   │   ├── usecase.rs            # AgentSessionRuntimeUsecase（実行側の公開 API・業務手順）
│   │   ├── session_state.rs      # per-session 実行状態（runtime handle / parts buffer / seq / turn phase /
│   │   │                         # pending queue / lock。queue は runtime handle と独立に生存する: §8.2）
│   │   ├── event_apply.rs        # AgentRuntimeEvent → event log 追記・parts 更新・status 更新
│   │   │                         # （旧 turn_event_log.rs の記帳・終端処理を吸収）
│   │   ├── streaming.rs          # seq 付き delta / flush 判定（旧 stream_emit.rs の decision 関数群。純関数）
│   │   ├── stale.rs              # turn liveness 判定（旧 recovery.rs の evaluate_turn_liveness 系。純関数）
│   │   ├── queue.rs              # pending message queue（旧 process_registry.rs の PendingMessage +
│   │   │                         # session_lifecycle.rs の queue 手順）
│   │   ├── system_prompt.rs      # system prompt 合成（旧 shared.rs compose_system_prompt +
│   │   │                         # system_context_rendering.rs を吸収。SessionSpec/TurnInput.system_prompt の供給者）
│   │   ├── context_restore.rs    # 復帰計画（旧 runtime/context_restore.rs + session_persistence.rs の plan 決定）
│   │   └── ports.rs              # AgentSessionEventNotifier / AgentTaskSpawner / Clock port
│   ├── event_log/                # 既存（events.rs の語彙を D6/D14 に合わせ更新）
│   ├── session/                  # 既存（SessionStore / 保存・転送モデル。§9.5 の逸脱記録参照）
│   ├── status.rs                 # 既存（SessionStatus.pending_permission_request を型付き DTO 化）
│   └── skill_catalog.rs          # backend 中立化（CODEX_BACKEND_ID 定数と codex 固有メソッドを削除）
│
├── adaptor/
│   ├── gateway/agent_session/
│   │   ├── session_storage/      # 既存（保存形式は D15 の差分のみ）
│   │   └── runtime_driver/       # event pump（tokio task 起動・タイマー駆動）。usecase port
│   │                             # （AgentTaskSpawner / Clock）の実装群 = GATEWAY.md の service_impl に準ずる
│   ├── presenter/agent_session.rs# Tauri event emit（usecase port AgentSessionEventNotifier の実装。
│   │                             # 既存 presenter/agent_status.rs と同パターン）
│   ├── protocol/agent_session.rs # frontend 向け wire 型（MessagePartMsg / PermissionRequestMsg /
│   │                             # SkillEntryMsg / SlashCommandMsg / ModelInfoMsg / BackendInfoMsg / …）
│   └── controller/command/agent_session/  # 既存 command 群を AgentSessionRuntimeUsecase 呼び出しに置換
│
└── infrastructure/
    ├── agent_session/
    │   ├── claude/
    │   │   ├── mod.rs            # ClaudeBackend（AgentBackend impl）
    │   │   ├── session.rs        # ClaudeSessionRuntime（AgentSessionRuntime impl / respawn / resume）
    │   │   ├── process.rs        # claude CLI spawn・env・stdout/stderr reader・graceful shutdown
    │   │   ├── wire.rs           # stream-json / control protocol の serde 型（公式 sdk.d.ts 準拠）
    │   │   ├── convert.rs        # wire message → Entity 変換（turn 集約状態を内包）
    │   │   ├── permission.rs     # can_use_tool ↔ PermissionRequest/Response、auto-allow 方針、mode 変換
    │   │   ├── models.rs         # 固定モデルカタログ + 表示名（旧 CLAUDE_FIXED_MODELS + model_display_name）
    │   │   └── skills.rs         # .claude/skills 走査、initialize 応答からの slash command 抽出
    │   └── codex/
    │       ├── mod.rs            # CodexBackend（AgentBackend impl）
    │       ├── session.rs        # CodexSessionRuntime（thread lifecycle / startup retry / resume）
    │       ├── app_server.rs     # プロセス spawn・JSONL framing・ワンショット client（旧 codex_app_server.rs の骨格）
    │       ├── wire.rs           # JSON-RPC method 定数・params builder・応答型（対象 codex バージョンを記録）
    │       ├── convert.rs        # item / notification → Entity 変換
    │       ├── permission.rs     # approval ↔ PermissionRequest/Response、sandbox / approval policy 変換
    │       ├── models.rs         # 固定モデルカタログ + 表示名（旧 CODEX_FIXED_MODELS）
    │       └── skills.rs         # .agents/skills 走査、skills/list・fuzzyFileSearch ワンショット
    └── process/
        ├── pid_registry.rs       # PidFileV1 / save_pgid / remove_pgid / cleanup_orphan_processes / CleanupGate
        │                         #（旧 recovery.rs の backend 非依存部分 + 旧 startup.rs の起動時 cleanup 配線）
        └── child_env.rs          # prepare_child_env 連携・session env（RELEASH_SESSION_ID 等。旧 shared.rs:1090-1113）
```

旧 `runtime_support/` 14 ファイルの移設対応表:

| 旧ファイル | 行き先 |
|---|---|
| `mod.rs` | 削除（facade 不要） |
| `claude_sdk_message.rs` | 削除。Claude wire 処理は `claude/{wire,convert,permission}.rs` として公式仕様から再実装 |
| `shared.rs` | `compose_system_prompt`/fingerprint → usecase/runtime/system_prompt.rs。env → infrastructure/process/child_env.rs。status 通知 → event_apply + presenter。stream part 統合 → domain `merge_part`。bridge cmd builder / 定数 → 削除 |
| `session_lifecycle.rs` | turn/close/queue 手順 → usecase/runtime/usecase.rs + queue.rs。Codex 分岐・bridge 直書き → 削除 |
| `external_agent.rs` | Codex 固有部 → codex/session.rs。turn 状態遷移・workflow 通知 → usecase/runtime |
| `stream_emit.rs` | 判定純関数 → usecase/runtime/streaming.rs。timer/emit 駆動 → runtime_driver + presenter |
| `permission.rs` | 状態遷移 → usecase/runtime/event_apply.rs。Claude 応答書込 → claude/permission.rs |
| `recovery.rs` | PID/orphan/CleanupGate → infrastructure/process。bridge spawn/EOF/watchdog env → claude/{process,session}.rs。stale 判定 → usecase/runtime/stale.rs |
| `session_persistence.rs` | spawn 情報解決・context carry 永続化 → usecase/runtime/{usecase,context_restore}.rs |
| `model_selection.rs` | 検証・永続化 → usecase/runtime/usecase.rs（反映は trait `set_model`）。`{"type":"setModel"}` 書込 → 削除 |
| `skills.rs` | scan → claude/skills.rs・codex/skills.rs（frontmatter parse は domain service）。画像添付 → usecase（既存 domain policy 呼び出しへ） |
| `process_registry.rs` | `AgentProcess`/`AgentProcessMap`/`BridgeState` → 削除（プロセスは各 runtime が所有）。`TurnPhase` → usecase/status.rs の定義に一本化。`PendingMessage` → usecase/runtime/queue.rs |
| `system_context_rendering.rs` | usecase/runtime/system_prompt.rs |
| `turn_event_log.rs` | usecase/runtime/event_apply.rs |

### 2.2 依存方向

```
infrastructure/agent_session/{claude,codex} ──▶ domain/agent_session（entities / gateway / value_objects / services）
infrastructure/agent_session/{claude,codex} ──▶ infrastructure/process（OS utility）
adaptor/{gateway,presenter,controller,protocol} ──▶ usecase ──▶ domain
lib.rs（composition root）が backend 実装を構築し usecase の registry へ注入
```

- backend 実装（infrastructure/agent_session/claude, codex）は **domain のみに依存する**。usecase / adaptor の型を import しない（現行の infra → usecase 直結を解消）。
- adaptor は infrastructure を import しない。現行の `adaptor/gateway/workflow/*` → `infrastructure::agent_session::runtime::*` 直接依存は §8.1 の usecase API へ置換する。`runtime_driver` が扱う `Box<dyn AgentSessionRuntime>` は domain trait であり、この規則に反しない。
- backend_id での分岐が残ってよい場所は、`AgentBackendRegistry`（dispatch 境界）と session metadata の保持・表示のみ。

### 2.3 GATEWAY.md からの逸脱記録

GATEWAY.md は「ドメイン型 ↔ 外部システム型の変換は adaptor/gateway で行う」「infrastructure は薄いラッパーに徹する」と定めるが、**本 Issue は requirements（:33,35「backend 固有の入力・状態・イベントを infrastructure 実装の内側で Entity へ変換して返す」）の明示指定を優先し、domain trait 実装と Entity 変換を `infrastructure/agent_session/{claude,codex}` に置く**。この逸脱は requirements 由来であり、docs/architecture 側への追補（agent backend の配置規約）は本 Issue 完了後に別途提案する。

review-03 R3-15 追記: `usecase/agent_session/runtime/usecase.rs` は `send_message` / `start_turn` / event pump 適用 / watchdog / queue drain / streaming flush / terminal projection を同一ファイルに残している。§2.1 の最終分割方針（event_apply への移設等）は正典として維持するが、review-03 の P0 では session lock 出口設計と世代相関の修正を優先し、大規模移設は後続 Issue に分離する。理由は、同一修正内で移設を重ねると lock 境界・queue drain・watchdog の回帰検証範囲が拡大し、R3-1/R3-2 の安全性確認が不透明になるため。残置箇所は `usecase/agent_session/runtime/usecase.rs` の event pump 適用、`complete_turn`、`start_next_queued_turn`、streaming flush 関連 helper。後続 Issue ではこの 4 群を `runtime/event_apply.rs` / `runtime/turn_completion.rs` / `runtime/queue_drain.rs` / `runtime/streaming.rs` へ移す。

---

## 3. Entity 定義（domain/agent_session/entities/）

domain 層は serde / tokio / tauri を使わない。自由形式 JSON は `JsonPayload` で運ぶ。

```rust
// value_objects/json_payload.rs
/// 有効な JSON テキストの newtype。domain は中身を解釈しない（判断材料として運搬するのみ）。
/// 有効性の保証は構築側（境界層: backend infra / adaptor）の責務であり、domain 内で再検証しない。
pub struct JsonPayload(String);
impl JsonPayload {
    pub fn new_unchecked(raw: String) -> Self;
    pub fn as_str(&self) -> &str;
}
```

```rust
// entities/message_part.rs
pub enum MessagePart {
    Thinking { content: String, parent_tool_use_id: Option<String> },
    Text     { content: String, parent_tool_use_id: Option<String> },
    ToolUse  { id: String, tool: String, input: JsonPayload, parent_tool_use_id: Option<String> },
    ToolResult {
        content: String, is_error: bool,
        tool_use_id: Option<String>, parent_tool_use_id: Option<String>,
        content_ref: Option<ToolOutputRef>, summary: Option<ToolOutputSummary>,
    },
    Error    { content: String, parent_tool_use_id: Option<String> },
    Permission { request: PermissionRequest },          // status / parent_tool_use_id は request が持つ
    TaskStatus { task_tool_use_id: String, status: String, description: Option<String>, summary: Option<String> },
    TodoListSnapshot { items: Vec<TodoListItem> },
    SystemNotification { notification_type: SystemNotificationType, status: String,
                         label: String, detail: Option<String>, hook_id: Option<String> },
    Image    { data: String, media_type: String },      // base64
    ImageRef { attachment: Attachment },
}

/// parts 列への差分適用規則。現在 projector.rs / codex.rs / claude_sdk_message.rs に
/// 三重実装されている push_or_update_* 群を、この単一実装へ集約する（README.md 横断原則）:
/// - Text/Thinking: 直前の同種 part（同 parent_tool_use_id）へ連結、なければ append
/// - ToolUse: 同 id を in-place 更新、なければ append
/// - ToolResult: 同 tool_use_id へ累積更新（apply_tool_result_update の規則を移設）、なければ append
/// - Permission: request.id 一致で in-place 更新（status 遷移含む）、なければ append
/// - TaskStatus: 同 task_tool_use_id を in-place 更新、なければ append
/// - TodoListSnapshot: 既存 snapshot を置換（単一スロット）
/// - SystemNotification: 同 type の in_progress を置換、なければ append
/// - Error: 同一内容の重複は追加しない
/// - Image/ImageRef: append
pub fn merge_part(parts: &mut Vec<MessagePart>, incoming: MessagePart);
```

```rust
// entities/permission_request.rs
pub struct PermissionRequest {
    /// backend 採番の request id（session 内で一意）。応答の相関キー。
    pub id: String,
    pub tool_use_id: Option<String>,
    /// サブタスク（Task/Agent）配下の permission をネスト表示するための相関。現行 part の
    /// parent_tool_use_id（session/mod.rs:128-133）を引き継ぐ。wire では part level の
    /// parentToolUseId として serialize する（§9.3）。
    pub parent_tool_use_id: Option<String>,
    /// §5.4 の共通 tool 語彙。UI 表示・アイコン分類・edit preview 判定に使う。
    pub tool_name: String,
    pub body: PermissionRequestBody,
    pub title: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub decision_reason: Option<String>,
    pub status: PermissionRequestStatus,
}

pub enum PermissionRequestBody {
    /// ツール実行の承認。input は §5.4 の tool 入力契約に従う（編集可否は tool_name + 構造で決まる）
    ToolApproval { input: JsonPayload },
    /// 計画の承認（Claude ExitPlanMode 相当）
    PlanApproval { plan: String, allowed_prompts: Vec<PermissionAllowedPrompt> },
    /// ユーザーへの質問（Claude AskUserQuestion / Codex request_user_input）
    Question { questions: Vec<PermissionQuestion> },
    /// 追加権限の付与要求（Codex item/permissions/requestApproval）
    PermissionGrant { requested: JsonPayload },
}

/// 現行 AgentPermissionAllowedPrompt（adaptor permission.rs:10-15）の domain 表現
pub struct PermissionAllowedPrompt { pub tool: String, pub prompt: String }

pub struct PermissionQuestion {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<PermissionQuestionOption>, // { label: String, description: Option<String> }
    pub multi_select: bool,
}

pub enum PermissionRequestStatus {
    Pending,
    Resolved { decision: PermissionDecision, answers: Option<JsonPayload> },
}
pub enum PermissionDecision { Allowed, Denied, Cancelled }

/// 実行側 → backend への応答。
pub struct PermissionResponse {
    pub request_id: String,
    pub decision: PermissionResponseDecision,
}
pub enum PermissionResponseDecision {
    /// updated_input: UI が編集した tool 入力（§5.4 契約）。answers: Question への回答
    Allow { updated_input: Option<JsonPayload>, answers: Option<JsonPayload> },
    Deny { message: Option<String> },
}
```

```rust
// entities/turn.rs
pub type TurnId = u64;
pub enum TurnResult {
    Completed   { stop_reason: Option<TurnStopReason>, token_usage: Option<TokenUsage> },
    Failed      { error: String, token_usage: Option<TokenUsage> },
    Interrupted { reason: InterruptReason, error: Option<String> },
}
pub enum TurnStopReason { Refusal }
pub enum InterruptReason { Abort, Timeout, Crash }   // BridgeCrash → Crash（D14）
pub struct TokenUsage { pub input_tokens: u64, pub output_tokens: u64,
                        pub total_tokens: Option<u64>, pub context_window_tokens: Option<u64> }
```

```rust
// entities/attachment.rs
pub struct Attachment { pub id: String, pub media_type: String, pub byte_size: u64 } // 旧 AttachmentRef
pub struct AttachmentPayload { pub data: String, pub media_type: String }           // base64 入力・復元用
```

- `Message` / `MessageRole` / `Session` / `SessionState` / `ContextCarryState` は現行 usecase 定義（`usecase/agent_session/session/mod.rs:171-194,342-396`）と同フィールド構成で domain へ定義する。serde 付きの保存・転送モデルの扱いは §9.5。
- `TodoListItem` / `SystemNotificationType` / `ToolOutputRef` / `ToolOutputSummary` は現行定義のまま domain value_objects へ移す（serde は除去し、serde 表現は保存・転送モデル側が持つ）。
- Turn の表現は D1 のとおり（struct を新設しない）。

---

## 4. backend interface（domain/agent_session/gateway.rs）

```rust
use futures::stream::Stream;   // DOMAIN.md が gateway trait に許容する形式

/// backend 起動時に実行側が渡す per-session 仕様。
pub struct SessionSpec {
    pub session_id: String,          // Releash Session id（chat_session_id）
    pub cwd: String,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub model: ModelId,
    pub system_prompt: Option<String>,   // 実行側（usecase/runtime/system_prompt.rs）が合成済みのもの
    /// 保存済み backend session（Claude session_id / Codex thread_id）での復帰要求。
    /// None なら新規。復帰の実行方式は backend が所有し、結果を SessionEstablished で報告する。
    pub resume: Option<String>,
    /// workflow step 由来の設定（無指定は backend / 実行側の既定値）。
    /// stale_timeout は実行側の stale 方針（§8.4）と同じ値のヒントであり、backend は
    /// 内部 watchdog（例: Claude CLI の stream idle timeout env）の設定に使ってよい。強制は実行側が行う。
    pub startup_timeout: Option<Duration>,
    pub startup_max_retries: Option<u32>,
    pub stale_timeout: Option<Duration>,
}

pub struct TurnInput {
    pub prompt: String,              // 復元 prefix（Reinject）は実行側が前置済み
    pub images: Vec<AttachmentPayload>,
    pub system_prompt: Option<String>,   // turn 時点の合成済み system prompt
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub editor_context: Option<EditorContext>,   // 対応しない backend は無視してよい
}

pub struct ForkSessionRequest {      // 現行 BackendSessionLifecycleRequest（stored_lifecycle.rs:178-195）相当
    pub backend_session_id: String,
    pub cwd: String,
    pub model: Option<String>,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
}

/// backend runtime → 実行側のイベント。全て Entity / value object のみで構成する。
pub enum AgentRuntimeEvent {
    /// backend session が確立した（新規 / resume 完了）。
    SessionEstablished { backend_session_id: String, resume: ResumeOutcome },
    /// backend が session id を破棄した（以後 resume 不能）。
    BackendSessionCleared,
    /// メッセージ part の差分。実行側が MessagePart::merge_part で適用する。
    PartsMerged(Vec<MessagePart>),
    /// 許可要求。status=Pending が通常。backend 側で自動解決済み（例: Claude permission_denied）の
    /// 場合は status=Resolved で届く。
    PermissionRequested(PermissionRequest),
    /// backend 起点の permission mode 変化（Claude の SDK 内モード遷移等）。
    PermissionModeChanged(PermissionMode),
    SlashCommandsUpdated(Vec<SlashCommand>),
    TokenUsageUpdated(TokenUsage),
    /// turn の終端。1 turn につき必ず 1 回。
    TurnCompleted(TurnResult),
    /// runtime が使用不能になった（turn 外のプロセス消滅・初期化失敗等）。実行側は runtime を破棄する。
    Fatal { message: String },
}

pub enum ResumeOutcome { NotRequested, Resumed, Mismatch { actual: String } }

pub enum AgentBackendError {
    StartupTimeout { retry_count: u32, max_retries: u32 },
    Unavailable(String),
    Invalid(String),
    Other(String),
}

#[async_trait::async_trait]
pub trait AgentBackend: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn available_models(&self) -> Vec<ModelDescriptor>;   // { id: ModelId, display_name: String }
    fn capabilities(&self) -> BackendCapabilities;        // { steering: bool }
    /// session runtime を開く（プロセス起動・resume 込み）。復旧・リトライは実装内で完結する。
    async fn open_session(&self, spec: SessionSpec)
        -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError>;

    // --- 稼働 runtime を要さない backend 操作（既存 usecase port の実装がここへ集約される） ---
    async fn archive_session(&self, backend_session_id: &str, cwd: &str) -> Result<(), AgentBackendError>;
    async fn unarchive_session(&self, backend_session_id: &str, cwd: &str) -> Result<(), AgentBackendError>;
    /// fork した backend session id を返す。fork を実装しない backend は Ok(None)。
    async fn fork_session(&self, req: ForkSessionRequest) -> Result<Option<String>, AgentBackendError>;
    async fn skill_catalog(&self, cwd: &Path, query: Option<&str>, limit: Option<usize>)
        -> Result<Vec<SkillEntry>, AgentBackendError>;
    /// runtime-native fuzzy search。非対応 backend は Ok(None)（呼び出し側が汎用列挙へフォールバック）。
    async fn fuzzy_file_search(&self, root: &Path, query: &str, limit: usize)
        -> Result<Option<Vec<String>>, AgentBackendError>;
}

#[async_trait::async_trait]
pub trait AgentSessionRuntime: Send + Sync {
    /// イベントストリーム。open 直後に一度だけ取得できる（2 回目以降は空 stream を返す）。
    fn take_events(&mut self) -> Pin<Box<dyn Stream<Item = AgentRuntimeEvent> + Send>>;
    async fn start_turn(&self, input: TurnInput) -> Result<(), AgentBackendError>;
    async fn steer(&self, input: TurnInput) -> Result<(), AgentBackendError>;      // 既定: Err(Unavailable)
    async fn interrupt(&self) -> Result<(), AgentBackendError>;
    async fn respond_permission(&self, response: PermissionResponse) -> Result<(), AgentBackendError>;
    async fn set_permission_mode(&self, mode: PermissionMode, plan_mode: bool) -> Result<(), AgentBackendError>;
    async fn set_model(&self, model: &ModelId) -> Result<(), AgentBackendError>;
    async fn set_session_title(&self, title: &str) -> Result<(), AgentBackendError>;  // 既定: Ok(())（no-op）
    /// graceful teardown。プロセス回収（SIGTERM→SIGKILL / pgid sweep / PID ファイル削除）まで実装が行う。
    async fn close(&self);
}
```

契約（実装者向けの正確な取り決め）:

1. **イベント順序**: part 系イベント（`PartsMerged` / `PermissionRequested`）は原則 turn 内（`start_turn` 後〜 `TurnCompleted`）で emit する。**TurnCompleted 後に届いた遅延 part（tool_result の残り・task 通知等）の emit は許容され**、実行側が直前 turn の agent message への post-turn 更新として適用・永続化する（現行 `accumulate_stream_or_post_turn_message_locked` の挙動を実行側で維持）。backend が emit してはならないのは、破棄済み turn（interrupt 済み・runtime 入替済み）に属する stale イベントである（Claude runtime は現行 turn_token 相当の内部相関で破棄する）。
2. **Turn の帰属**: backend は turn id を発行しない。実行側が event log 上で `TurnId` を採番し、イベントの到着順で現行 turn（turn 中でなければ直前 turn への post-turn 更新）に帰属させる。
3. **`SessionEstablished` のタイミング**: `open_session` は `SessionEstablished` を待たずに返ってよい。`SessionEstablished` は stream 上のイベントとしてのみ届き、遅くとも最初の `TurnCompleted` より前に emit する。
4. **`TurnCompleted` と `Fatal` の順序**: turn 実行中に runtime が致命故障した場合、backend は**必ず `TurnCompleted(Interrupted { reason: Crash })` を先に emit してから `Fatal` を emit する**。§8.2 の Fatal 受信時の turn 終端処理は、この契約が破られた場合の防御的フォールバックである。
5. **`close()` と stream 終端**: `close()` 呼び出し後、backend は `Fatal` を emit せずに stream を終端する。event pump は stream 終端で停止する。close 起因の stdout EOF を `Fatal` にしてはならない。
6. **復旧の所有**: プロセス死・接続断からの再起動、resume の実行、startup retry は各 runtime 実装の内側で完結する。`Fatal` / `Crash` 後の次 turn では実行側が `open_session` からやり直す。
7. **stale timeout は実行側の方針**: 無進捗検出（`stale_timeout` 超過）と、その際の `interrupt()` → `close()` → turn の Timeout 完了は実行側が行う（§8.4）。backend は `SessionSpec.stale_timeout` を内部 watchdog のヒントに使ってよい。
8. **`PermissionRequested`（Pending）中の turn**: backend は応答（`respond_permission`）が来るまで turn を進めない。応答後は継続または停止（Deny 時の継続/停止は backend 固有: Claude は tool 拒否で継続、Codex decline も継続）。
9. **`set_*` の失敗**: `set_permission_mode` / `set_model` / `set_session_title` の失敗は turn を壊さない。エラーを返してよいが、実行側は log に留めて処理を継続する（§8.1 の persist-first 規則）。

---

## 5. Entity 変換契約

### 5.1 実行側が観測できる状態（behavior「source 値は露出しない」の適用）

実行側・UI・workflow が受け取ってよいのは次のみ:

- Entity: Session / Turn（event log） / Message / MessagePart / PermissionRequest
- 表示・選択用 DTO / value object: `ModelDescriptor`（→ `ModelInfo` DTO）、`BackendInfo`、`SkillEntry`、`SlashCommand`、`SessionStatus`
- 上記の serde 表現（adaptor/protocol の wire 型）

以下は infrastructure の外に出してはならない: Claude SDK message JSON・control protocol payload・`permissionMode` 生値（`default`/`acceptEdits`/`bypassPermissions`/`plan`）・Codex JSON-RPC payload・approval decision 生値（`accept`/`decline` 等）・sandbox policy 生値（`read-only`/`workspaceWrite` 等）・rollout/thread の生 JSON。

### 5.2 permission_mode の抽象語彙（現行維持）

外部境界は `PermissionMode::{Ask, Edit, Full}`（`domain/agent_session/value_objects/permission_mode.rs`）+ 独立した `plan_mode: bool`（issues-947 / parity #18 の合意）。backend 語彙への変換は各 backend の `permission.rs` に置く。現行 `infrastructure/agent_session/runtime/permission_flags.rs` は分割して各 backend へ移し、ファイルごと削除する。

| 抽象 | Claude wire mode | Codex（thread/turn params） |
|---|---|---|
| Ask + plan_mode=false | `default` | `approvalPolicy: "on-request"`, `sandboxPolicy: {type:"readOnly", networkAccess:false}` |
| Edit + plan_mode=false | `acceptEdits` | `approvalPolicy: "on-request"`, `sandboxPolicy: {type:"workspaceWrite", writableRoots:[cwd], networkAccess:false, excludeTmpdirEnvVar:false, excludeSlashTmp:false}` |
| Full | `bypassPermissions` | `approvalPolicy: "never"`, `sandboxPolicy: {type:"dangerFullAccess"}` |
| plan_mode=true（Ask/Edit いずれでも） | `plan` | plan collaboration mode + `approvalPolicy: "on-request"` + readOnly sandbox。**wire 形式はバージョン依存**: 現行コード（`codex_app_server.rs:1302-1310`）は `collaborationMode: "plan"`（文字列）を送るが、公式 main の protocol では `collaborationMode` は struct `{ mode, settings { model, reasoning_effort, developer_instructions } }` である。M2 冒頭で対象 codex バージョンの受理形式を実プロセスで確認し、受理される形式を採用する（§7.5） |
| permission_profile_id 指定時 | （Claude は profile 非対応。無視） | mode 変換をスキップし `permissions: <profile_id>` のみ設定（現行 `codex_app_server.rs:1276-1281` の挙動維持） |

### 5.3 turn 完了の意味論（現行維持）

- `TurnResult::Completed` → event log `TurnCompleted { exit_code: 0 }` 相当。session state は `Done`（`SessionLifecycleController` 規則: `lifecycle_controller.rs:33-41`）。
- `TurnResult::Failed` → `exit_code: 1` 相当。session state `Error`。
- `TurnResult::Interrupted { Abort }` → `Idle`。`{ Timeout }` → exit_code 124 相当・`Error`。`{ Crash }` → `Error`。
- workflow への通知（`WorkflowTurnCompleteNotification`）は projection（`SessionReadModel.workflow_turn_complete`）から従来どおり実行側が生成する（issues-1247 B5 の合意維持）。

### 5.4 共通 tool 語彙契約（正典化）

`MessagePart::ToolUse.tool` / `PermissionRequest.tool_name` に現れてよい語彙と入力契約を Releash の Entity 契約として固定する（feat-agent-native-parity goal.md A-1 / B-2 の正規化マップを継承）。adaptor / frontend はこの契約に対してのみ分岐してよい（backend_id での分岐は不可）。

| tool 名 | 意味 | input 契約（表示・編集に使うキー） | 供給元 |
|---|---|---|---|
| `Bash` | コマンド実行 | `{ command, cwd?, status? }` | Claude ネイティブ / Codex `commandExecution` |
| `Edit` / `Write` / `MultiEdit` / `NotebookEdit` | ファイル変更 | Claude 形: `{ file_path, old_string?, new_string?/content, edits? }`（編集可）。diff 形: `{ file_path, kind, diff, changes? }`（プレビューのみ） | Claude ネイティブ / Codex `fileChange`（diff 形） |
| `Read` / `Glob` / `Grep` / `Task` / `Agent` / `WebFetch` | 読み取り・探索・サブタスク | Claude ネイティブ input | Claude |
| `WebSearch` | Web 検索 | `{ query }` | Claude / Codex `webSearch` |
| `mcp__<server>__<tool>` | MCP ツール | `{ server?, tool?, arguments? }` または MCP native input | Claude / Codex `mcpToolCall` |
| `TodoWrite` | TODO 更新（`TodoListSnapshot` part へ正規化） | — | Claude |
| `AskUserQuestion` | 質問（`PermissionRequestBody::Question`） | — | Claude / Codex user-input 要求 |
| `ExitPlanMode` | 計画承認（`PermissionRequestBody::PlanApproval`） | — | Claude |
| `CodexCommand` / `CodexFileChange` / `CodexPermissions` / `CodexApproval` / `CodexTool` | Codex 承認・動的ツールの表示名 | §7.2 の各契約 | Codex |

- 現行の `codex_file_change: true` マーカー（`codex_app_server.rs:484`、`edit_preview.rs:245-252` が解釈）は削除し、**diff 形（`diff` キーの存在）**で edit preview の分岐を行うよう `edit_preview.rs` / `tool_activity.rs` を書き換える。
- `present_agent_permission_request` の kind 判定（`permission.rs:186-218` の tool_name match）は `PermissionRequestBody` の variant 参照に置き換える。編集可否判定（Edit/Write/MultiEdit + input 構造）は本表の共通契約に対する判定として存置する。

---

## 6. Claude backend 実装仕様（infrastructure/agent_session/claude/）

### 6.1 統合方式: `claude` CLI 直接 spawn（D3）

Node bridge を全廃する。公式 SDK（TS/Python）はいずれも `claude` CLI を子プロセス起動し stream-json（NDJSON）で通信する薄いラッパーであり、Rust から同一プロトコルを直接話すのが公式サポートされた最短経路である（headless docs / cli-reference / `subprocess_cli.py:225` の spawn 引数）。現行 bridge も `pathToClaudeCodeExecutable: "claude"` で PATH 上の CLI に依存しているため、前提バイナリは変わらない（CLI >= 2.0.0 をチェックする）。

**spawn**（`process.rs`）:

```
claude --input-format stream-json --output-format stream-json --verbose \
       --include-partial-messages \
       --permission-prompt-tool stdio \
       --allow-dangerously-skip-permissions \
       --setting-sources user,project \
       --permission-mode <default|acceptEdits|bypassPermissions|plan> \
       [--model <model_id>] \
       [--resume <backend_session_id>] \
       [--append-system-prompt-file <tmpfile>]     # 合成済み system prompt（長文のため file 渡し）
```

- `--allow-dangerously-skip-permissions` は常時付与する。これは bypass を**開始**するフラグではなく、セッション途中の `set_permission_mode(bypassPermissions)`（Ask/Edit → Full 切替）を CLI に許可するフラグである（cli-reference / sdk.d.ts の `allowDangerouslySkipPermissions`）。現行の「実行中セッションでいつでも Full へ切替できる」ユーザー可視挙動を維持するために必要。
- `current_dir(cwd)`、stdin/stdout/stderr piped、Unix は `pre_exec` で `setsid()`。
- env: `CLAUDECODE` / `CLAUDE_CODE_ENTRYPOINT` を除去。`infrastructure/process::child_env` で PATH alias・`RELEASH_DATA_DIR`・`RELEASH_SESSION_ID`・`RELEASH_BASE_BRANCH` を注入（現行 `recovery.rs:945-969` / `shared.rs:1090-1113` 相当）。CLI 側 watchdog 設定 env（`CLAUDE_STREAM_IDLE_TIMEOUT_MS`（= `SessionSpec.stale_timeout` を ms 化。未指定時は既定 180 秒） / `CLAUDE_ENABLE_STREAM_WATCHDOG=1` / `CLAUDE_ENABLE_BYTE_WATCHDOG=1` / `CLAUDE_CODE_MAX_RETRIES=10` / `API_TIMEOUT_MS=600000`、現行 `recovery.rs:58-71`）を維持。
- PID 登録: `infrastructure/process::pid_registry`（`save_pgid` / spawn 前の `CleanupGate` 待機）。
- stdout は 1 行 1 JSON。パース失敗行はバッファに蓄積して再試行（長大行対策。Python SDK と同じ speculative parse、上限 1MB）。非 JSON 行は無視。**未知の message type / 未知フィールドはエラーにせず無視する**（SDK と同じ前方互換規約）。
- 終了（`close()`）: stdin EOF → 5 秒 → SIGTERM → 5 秒 → SIGKILL（SDK と同じ graceful shutdown。session ファイル flush を守る）→ pgid sweep fallback → PID ファイル削除。契約 5 のとおり close 起因の EOF では `Fatal` を出さず stream を終端する。
- `result` が `is_error: true` の後に CLI が非ゼロ exit するのは正常系として扱う（SDK 同様）。

**handshake**: spawn 直後に control_request `initialize`（`hooks: null`）を送る。応答の `commands` から `SlashCommandsUpdated` を発行する（現行 bridge の `supportedCommands()` → `supported_commands` message を置換。イベント形は不変）。応答の `models` はカタログには使わない（D8。固定カタログ維持）。

**turn 実行**（`start_turn`）: stdin に user メッセージを書く。

```json
{"type":"user","session_id":"","parent_tool_use_id":null,
 "message":{"role":"user","content":[{"type":"text","text":"<prompt>"},
   {"type":"image","source":{"type":"base64","media_type":"<mt>","data":"<b64>"}}]}}
```

- `TurnInput.system_prompt` の fingerprint が spawn 時と異なる場合、runtime は自プロセスを graceful に入れ替えてから turn を開始する（現行 `replace_ready_runtime_if_system_prompt_changed` の挙動を runtime 内部へ移設）。
- `TurnInput.permission_mode` / `plan_mode` が前回と異なる場合、turn 開始前に control_request `set_permission_mode` を送る（現行 `sync_pre_turn_settings` 相当）。
- `editor_context` は Claude では使用しない（無視）。

### 6.2 wire → Entity 変換（`convert.rs`）

| stream-json message | Entity 変換 |
|---|---|
| `system/init` | `SessionEstablished { backend_session_id: session_id, resume: 判定 }`。resume 要求 id と `session_id` の不一致は `ResumeOutcome::Mismatch`（現行 `session_ready_resume_mismatch` 相当の判定を runtime 内で実施） |
| `stream_event` `content_block_delta.text_delta` | `PartsMerged([Text { content: delta }])` |
| `stream_event` `content_block_delta.thinking_delta` | `PartsMerged([Thinking { content: delta }])` |
| `assistant` content `tool_use`（tool=TodoWrite 以外） | `PartsMerged([ToolUse { id, tool, input }])` |
| `assistant` content `tool_use`（tool=TodoWrite） | `PartsMerged([Text(進捗ログ), TodoListSnapshot { items }])`（現行 `extract_todo_items` / `push_todo_snapshot` の規則を移設） |
| `user` content `tool_result` | `PartsMerged([ToolResult { content, is_error, tool_use_id }])`。`agentId:` / `with ID:` prefix からの task id 対応表も convert 内で維持。turn 終端後に届いた場合も emit する（契約 1 の post-turn 更新） |
| control_request `can_use_tool` | §6.3 の auto-allow 判定後、必要なら `PermissionRequested(PermissionRequest)`（Pending） |
| `system/permission_denied` | `PermissionRequested`（`status: Resolved { decision: Denied }`。現行 `permission_denied` part 相当） |
| `system` subtype `task_started/task_notification/task_progress/task_updated` | `PartsMerged([TaskStatus {..}])`（現行 `accumulate_sdk_message` の task 系規則を移設） |
| `system/compact_boundary`・`system/status ("compacting")` | `PartsMerged([SystemNotification { Compaction, .. }])` |
| `system/status` の `permissionMode` 変化 | `PermissionModeChanged(mode)`（`plan` は無視。現行 `mode_from_claude_flag` 規則） |
| その他の `system` subtype のうち、現行 frontend listener（`useAgentSdkListeners.ts:110-145` handleSystemMessage）が本文表示していたもの | `PartsMerged([SystemNotification または Error])` へ変換して表示を維持する。**実装時に現行 listener が処理する subtype 集合を移植し、対応表を convert.rs のテストに固定する**（`task_*` 系は上記 TaskStatus 行が吸収） |
| `result`（success） | `TokenUsageUpdated`（`modelUsage` 合算。現行 `token_usage_from_result_message` の規則を convert 内へ移設）→ `TurnCompleted(Completed { stop_reason, token_usage })` |
| `result`（`is_error` / error subtype） | **`PartsMerged([Error { content: result/errors 連結 }])` を先に emit してから** `TurnCompleted(Failed { error })`（現行は frontend の handleResultErrors:147-171 が表示していたエラー本文を Entity 経路で供給する） |
| interrupt 後の turn 終端 | interrupt control_response(success) 受信後に result が届いた場合も `TurnCompleted(Interrupted { reason: Abort })` とする（現行 `buildResultTurnCompletion` の wasAborted 優先を踏襲）。result が 10 秒以内に届かなければ `TurnCompleted(Interrupted { Abort })` を合成する |
| stdout EOF（turn 中、close 起因を除く） | `TurnCompleted(Interrupted { reason: Crash, error })` → `Fatal` |
| stdout EOF（idle、close 起因を除く） | `Fatal` |
| `keep_alive` | `KeepAlive`（生存通知。実行側は stale 監視の progress のみ更新し、part は生成しない。改訂: 当初「無視」だったが、無視すると長時間ツール実行中の健全な turn が §8.4 の stale timeout で誤終端されるため変換対象に変更） |
| 未知 type | 無視 |

- turn 相関: runtime は turn ごとに内部 token を保持し、破棄済み turn の遅延イベントを捨てる（現行 `turn_token` / `active_turn_token` / `post_turn_message_token` 相当の機構は Claude runtime 内部実装。Entity には露出しない）。
- `interrupt()`: control_request `interrupt`。`set_model()`: control_request `set_model`。`set_permission_mode()`: control_request `set_permission_mode`（失敗は Err で返すが契約 9 のとおり実行側は継続する）。
- `respond_permission()`: §6.3。`set_session_title()`: no-op `Ok(())`。`steer()`: `Err(Unavailable)`、`capabilities().steering == false`。
- interrupt 時の resume rollback: 現行 bridge の `rollbackResumeSessionIdAfterInterrupt`（最後に正常完了した turn の session_id へ戻す）と同じ規則を runtime が実装する（「最後に成功した turn の session_id」を保持し、interrupted 終端時にそれを有効な backend_session_id として扱う）。

### 6.3 permission（`permission.rs`）

- `--permission-prompt-tool stdio` により、未解決のツール承認が control_request `can_use_tool`（`tool_name` / `input` / `tool_use_id` / `title` / `display_name` / `description` / `decision_reason` / `permission_suggestions`）として届く。
- **auto-allow 方針（現行 bridge `claude-sdk-bridge.mjs:260-289` の挙動を維持。ユーザー可視挙動の不変条件）**。判定キーは抽象 PermissionMode ではなく **spawn / set_permission_mode で送信済みの Claude wire mode**:

| wire mode | 対話ツール（`AskUserQuestion` / `EnterPlanMode` / `ExitPlanMode`） | それ以外のツール |
|---|---|---|
| `bypassPermissions` | 即 allow | 即 allow |
| `acceptEdits` | `PermissionRequested` に上げる | 即 allow |
| `plan`（plan_mode=true） | `PermissionRequested` に上げる | **即 allow**（現行 bridge の `mode !== "default"` 判定に一致。plan 中の編集系 can_use_tool も auto-allow される点は現行踏襲の設計判断として記録する） |
| `default`（Ask） | `PermissionRequested` に上げる | `PermissionRequested` に上げる |

- Entity 化: `tool_name == "ExitPlanMode"` → `PlanApproval`（`input.plan` / `input.allowedPrompts` を構造化）、`"AskUserQuestion"` → `Question`（`input.questions` を `PermissionQuestion` へ）、それ以外 → `ToolApproval { input }`。`parent_tool_use_id` は message envelope / can_use_tool の `agent_id` 相当情報から現行規則（`claude_sdk_message.rs:618-645`）どおり設定する。
- 応答変換: `Allow { updated_input, answers }` → control_response `{"behavior":"allow","updatedInput":<updated_input または元 input。answers があれば {questions: 元, answers} を合成>}`。`Deny { message }` → `{"behavior":"deny","message":<message または "User denied">}`。エンベロープは `{"type":"control_response","response":{"subtype":"success","request_id":...,"response":<上記>}}`。

### 6.4 復旧・カタログ・skills

- **resume**: `SessionSpec.resume` があれば `--resume <id>` で spawn。`system/init` の session_id 不一致は `ResumeOutcome::Mismatch` として報告（実行側が reinject へ切替える。§8.5）。
- **respawn**: turn 開始時にプロセスが死んでいれば runtime 内で再 spawn する（保存済み backend_session_id での resume を含む）。
- **models**（`models.rs`）: 固定カタログを移設。id・表示名・順序は現行値を維持（`claude-opus-4-8`→"Opus 4.8"、`claude-opus-4-7`→"Opus 4.7"、`opus[1m]`→"Opus 1m"、`claude-sonnet-4-5`→"Sonnet 4.5"、`claude-haiku-4-5-20251001`→"Haiku 4.5"）。
- **skills**（`skills.rs`）: `~/.claude/skills` / `<cwd>/.claude/skills` の走査（現行 `skills.rs:88-133` の Claude 側）。frontmatter parse は domain service（D9）。`fuzzy_file_search` は `Ok(None)`。
- **fork**: Claude CLI 自体は `--resume <id> --fork-session` で fork 可能だが、現行 Releash の fork は storage レベル（`agent_session_id` リセット）であり Codex のみ runtime fork を行う。**本 Issue では現行挙動維持のため `fork_session` は `Ok(None)` とする**（能力が無いのではなく実装しない判断。capability 拡張は将来 Issue）。
- **削除**: `resources/claude-sdk-bridge.mjs`、`resources/bridge-utils.mjs`（+ test）、`scripts/build-bridge.mjs`、`package.json` の `@anthropic-ai/claude-agent-sdk` 依存と `build:bridge` script、`tauri.conf.json` の bridge resource 登録、`generated/bridges/`。

---

## 7. Codex backend 実装仕様（infrastructure/agent_session/codex/）

### 7.1 統合方式

`codex app-server`（stdio、JSONL）+ v2 API（thread/turn/item）。現行実装（`codex_app_server.rs` / `codex.rs`）の JSON-RPC 面は**現在サポートしている codex バージョンに対して動作実績がある**ため骨格（initialize handshake / thread lifecycle / read loop / startup retry / JSONL framing / ワンショット client）を再利用し、**出口を Entity に差し替える**。ただし wire 形の細部は protocol バージョンに依存するため、§7.5 の検証を M2 冒頭で行い、`wire.rs` に対象バージョンを記録する。

- initialize params: `clientInfo { name: "releash", title: "Releash", version }` + `capabilities { experimentalApi: true }`（現行維持。新規の experimental method は追加しない）。
- `SessionSpec.system_prompt` / `TurnInput.system_prompt` は `thread/start` / `thread/resume` / `turn/start` の `developerInstructions` param に設定する（現行 `codex_app_server.rs:1361-1365,1399-1403,1539-1543` の挙動維持）。
- startup timeout / retry: `SessionSpec.startup_timeout` / `startup_max_retries` を消費（上限 clamp 定数は現行 `timeouts.rs` から codex/ 内へ移設）。

### 7.2 wire → Entity 変換（`convert.rs`）

現行 `app_server_message_to_session_events`（`codex_app_server.rs:737-903`）の変換を Entity 直行に改める:

| Codex message | Entity 変換 |
|---|---|
| notif `thread/started` / resp `result.thread.id` | `SessionEstablished { backend_session_id: thread_id, resume: 判定 }` |
| notif `turn/started` | （turn 相関の内部記録のみ。イベント不要） |
| notif `item/agentMessage/delta` | `PartsMerged([Text { content: delta }])` |
| `item/started`・`item/completed` の `commandExecution` | `ToolUse { tool: "Bash", input: {command, cwd, status} }` / `ToolResult { content: aggregatedOutput.., is_error }`（現行マップ維持。Codex 固有 `source` フィールドの素通しは削除） |
| 同 `fileChange` | `ToolUse { tool: "Edit", input: { file_path, kind, diff, changes } }` + `ToolResult { content: diff }`（`codex_file_change` マーカー廃止 → diff 形契約 §5.4） |
| 同 `mcpToolCall` | `ToolUse { tool: "mcp__{server}__{tool}", input: {server, tool, arguments} }` / `ToolResult` |
| 同 `webSearch` | `ToolUse { tool: "WebSearch", input: {query} }` / `ToolResult` |
| 同 `dynamicToolCall`（user-input 要求以外） | `ToolUse { tool: item.tool or "CodexTool" }` / `ToolResult` |
| `item/completed` `reasoning` | `PartsMerged([Thinking { content }])` |
| `item/completed` `error`（※） | `PartsMerged([Error { content }])` |
| `item/completed` `todo_list`（※） | `PartsMerged([Text(進捗ログ), TodoListSnapshot { items }])` |
| notif `item/commandExecution/outputDelta` / `item/fileChange/patchUpdated` | `ToolResult` 累積 / Edit ペア更新（merge_part 規則で適用） |
| notif `thread/compacted` | `PartsMerged([SystemNotification { Compaction, .. }])` |
| notif `thread/tokenUsage/updated` | `TokenUsageUpdated(TokenUsage)`（**Claude `result` 互換 JSON の合成と `agent-sdk-message` emit は行わない**） |
| notif `error`（turn 中） | `PartsMerged([Error { content: message }])` |
| notif `turn/completed` | status=`completed` → `TurnCompleted(Completed)`、`failed` → `Failed { error: turn.error.message }`、`interrupted` → `Interrupted { Abort }` |
| resp error（tracked request: thread/start・resume・turn/start） | `PartsMerged([Error])` +（init/resume 失敗時）`BackendSessionCleared` → `Fatal` |
| server request `item/commandExecution/requestApproval` | `PermissionRequested(PermissionRequest { tool_name: "CodexCommand", body: ToolApproval { input: params 由来の {command, cwd, reason} } })` |
| server request `item/fileChange/requestApproval` | `PermissionRequested({ tool_name: "CodexFileChange", body: ToolApproval { input: {itemId 対応の changes/diff} } })` |
| server request `item/permissions/requestApproval` | `PermissionRequested({ tool_name: "CodexPermissions", body: PermissionGrant { requested: params } })` |
| user-input 要求（server request `item/tool/requestUserInput`、および tool 名 `request_user_input` の `dynamicToolCall` item。現行コードは method 名部分一致 `is_user_input_request_method` で両対応） | `PermissionRequested({ tool_name: "AskUserQuestion", body: Question { questions } })`（現行 `codex_question_input` の正規化を維持） |
| stdout EOF（turn 中、close 起因を除く） | `TurnCompleted(Interrupted { reason: Crash, error })` → `Fatal`（現行は stale watchdog 任せで明示終端しないが、契約 4 に合わせ Claude と対称にする） |
| stdout EOF（idle、close 起因を除く） | `Fatal` |
| realtime / goal / account 系 notif・未知 method | 無視（現行同様） |

（※）`error` / `todo_list` item は**公式 main の `ThreadItem` に存在しない**（todoList は protocol v0.63 に存在し v0.77 で削除、error は notification `error` / `turn.error` に移行済み）。現行コードが処理している legacy 互換行として残すが、**M2 の検証（§7.5）で対象バージョンが emit しないことを確認した場合はこの 2 行を削除し、TODO 表示の要否は `turn/plan/updated` notification への対応として別途判断する**（本 Issue では追加しない）。

### 7.3 permission 応答変換（`permission.rs`）

request_id → 元 method の対応表（現行 `pending_approval_methods`）を runtime 内に保持し、`PermissionResponse` を JSON-RPC 応答へ変換する:

| 元 method | Allow | Deny |
|---|---|---|
| commandExecution / fileChange requestApproval | `{"result":{"decision":"accept"}}` | `{"result":{"decision":"decline"}}` |
| permissions/requestApproval | `{"result":{"permissions": <updated_input 由来>, "scope":"turn"}}`。updated_input 欠如時は `{"permissions":{"fileSystem":null,"network":null},"scope":"turn"}`（現行 `codex_app_server.rs:1105-1119` の fallback 維持） | JSON-RPC error 応答（コード値は現行踏襲で `-32001`。公式に decline 用コードの規定はなく「error 応答 = 拒否」が実契約。`-32001` は server 側 overload の予約コードでもあるため、Releash 規約としての採用である旨をコードコメントに記録） |
| user-input 要求 | `{"result":{"answers": <answers>}}` | 同上（error `-32001`） |

### 7.4 lifecycle・復旧・カタログ・skills

- `interrupt()` → `turn/interrupt`（thread_id + turn_id）。**共有 stdin へ `{"type":"interrupt"}` を書く経路は消滅する**（実行側の stale 処理も trait の `interrupt()` を呼ぶ）。
- `set_permission_mode()` → `thread/settings/update`（現行実装維持）。`set_session_title()` → `thread/name/set`。
- `set_model()`: 選択の永続化は実行側。runtime は次回 `thread/start` / `turn/start` の `model` param に反映する（**`{"type":"setModel"}` 行の書き込みは廃止**。現行 `model_selection.rs:264-266` の backend 無差別 stdin 書き込みは Codex に対する不正入力であり削除）。
- **resume**: `SessionSpec.resume`（= 保存済み thread_id）→ `thread/resume`。失敗（tracked error）時は `BackendSessionCleared` + `Fatal` を報告し、実行側が context_carry を Failed 化 → 次回 Reinject（§8.5）。
- **startup retry**: thread_id 待ちタイムアウトで再 spawn、`startup_max_retries` 超過で `open_session` が `Err(StartupTimeout)`（現行 `retry_startup_until_ready` を runtime 内へ）。
- **models**: 固定カタログ移設（`gpt-5.5`→"GPT-5.5"、`gpt-5.4`→"GPT-5.4"、`gpt-5.4-mini`→"GPT-5.4 Mini"）。
- **skills**: `skills/list` ワンショット（scope 正規化 user→personal / repo→project）+ `.agents/skills` ローカル走査。`fuzzy_file_search` → `fuzzyFileSearch` ワンショット（現行 gateway 3 種の実装を codex/ 配下へ集約。CLI パスは `AgentConfigRepository::cli_path_for("codex")` から解決）。
- archive / unarchive / fork → `thread/archive` / `thread/unarchive` / `thread/fork`（現行 `thread_lifecycle_gateway.rs` の実装を `CodexBackend` メソッドへ移設。Claude 側は no-op / `Ok(None)`）。

### 7.5 M2 冒頭のバージョン検証（実装手順）

対象 codex CLI バージョン（`codex --version`）を確定・記録した上で、実プロセスに対して次を確認し、結果に応じて wire.rs / 変換表を確定する:

1. plan mode の `collaborationMode` param の受理形式（文字列 `"plan"` か struct か。§5.2）。
2. `item/completed` に `todo_list` / `error` item が届くか（届かなければ §7.2 の該当行を削除）。
3. user-input 要求の server request method 名（`item/tool/requestUserInput` か旧名か）。
4. `thread/settings/update` の params 形。

---

## 8. 実行側（host）仕様

### 8.1 AgentSessionRuntimeUsecase（usecase/agent_session/runtime/usecase.rs）

実行側の唯一の公開 API。controller / workflow adaptor はこれだけを呼ぶ。

```rust
pub struct AgentSessionRuntimeUsecase {
    session_store: Arc<SessionStore>,
    registry: Arc<AgentBackendRegistry>,
    status_center: Arc<AgentStatusCenter>,
    notifier: Arc<dyn AgentSessionEventNotifier>,   // presenter 実装（Tauri emit）
    spawner: Arc<dyn AgentTaskSpawner>,             // adaptor 実装（tokio::spawn）
    sessions: /* per-session 実行状態 map。runtime handle と pending queue は別フィールド
                 （queue は runtime 破棄を生き延びる） */,
}

impl AgentSessionRuntimeUsecase {
    pub async fn start_session(&self, session_id: &str, opts: StartSessionOptions) -> Result<(), RuntimeUsecaseError>;
    pub async fn send_message(&self, req: SendAgentMessageRequest) -> Result<SendMessageResponse, ...>;   // 新規 session 作成含む
    pub async fn start_turn_locked(&self, ...) -> Result<(), ...>;   // workflow step 用（lock 保持前提）
    pub async fn interrupt(&self, session_id: &str) -> Result<(), ...>;
    pub async fn cancel_queued_turn(&self, session_id: &str, queued_turn_id: Option<&str>) -> Result<CancelQueuedTurnResponse, ...>;
    pub async fn respond_permission(&self, session_id: &str, response: PermissionResponse) -> Result<(), ...>;
    pub async fn set_permission_mode(&self, session_id: &str, mode: PermissionMode) -> Result<(), ...>;
    pub async fn set_plan_mode(&self, session_id: &str, plan_mode: bool) -> Result<(), ...>;
    pub async fn set_model(&self, session_id: &str, entry_id: &str) -> Result<(), ...>;
    pub async fn set_session_backend(&self, session_id: &str, backend_id: &str) -> Result<GetSessionResponse, ...>;
    pub async fn set_session_title(&self, session_id: &str, title: &str) -> Result<(), ...>;
    pub async fn close_session(&self, session_id: &str) -> Result<(), ...>;
    pub async fn close_all(&self);
    pub async fn get_session(&self, session_id: &str) -> Result<Option<GetSessionResponse>, ...>;
    pub async fn init_sessions(&self, worktree_path: &str) -> Result<InitSessionsResponse, ...>;
    // --- workflow adaptor 向け（現行 infrastructure 直接依存の置換先） ---
    pub async fn is_runtime_busy(&self, session_id: &str) -> bool;          // 旧 is_agent_step_runtime_busy
    pub async fn has_live_runtime(&self, session_id: &str) -> bool;         // 旧 AgentProcessMap.contains_key
    pub async fn active_session_ids(&self, candidates: &[String]) -> HashSet<String>; // 旧 collect_runtime_session_sets
    pub async fn turn_phase(&self, session_id: &str) -> Option<TurnPhase>;  // 旧 approval_runtime の phase 参照
    pub async fn acquire_session_lock(&self, session_id: &str) -> SessionRuntimeLockGuard;
    pub fn list_backends(&self) -> BackendListResult;                       // { backends: Vec<BackendInfo>, default_id }
    pub async fn skill_catalog(&self, backend_id: Option<&str>, cwd: &Path, query: Option<&str>, limit: Option<usize>) -> Result<Vec<SkillEntry>, ...>;
    pub async fn mentionable_files(&self, backend_id: Option<&str>, root: &Path, query: &str, limit: usize) -> Result<Option<Vec<String>>, ...>;
}
```

- `StartSessionOptions` は現行 `start_agent_session` command の引数（permission_mode / plan_mode）+ workflow instructions に対応する。
- turn 実行手順（`send_message` / `start_turn_*` の業務手順。現行 `session_lifecycle.rs` の手順から backend 分岐を除いたもの）:
  1. session 解決・作成（backend_id は `resolve_session_backend` / registry で確定済み。以後 Option ではない）
  2. busy 判定（turn phase / pending / starting）。busy なら steering 可否を `backend.capabilities().steering` + `runtime.steer()` に委ね、不可なら pending queue へ + `interrupt()`
  3. runtime 確保: 生きた runtime がなければ `registry.get(backend_id)?.open_session(spec)`。`spec.resume` / Reinject は §8.5、`spec.system_prompt` は runtime/system_prompt.rs の合成結果、`spec.{startup_timeout, startup_max_retries, stale_timeout}` は meta の workflow_step_context 由来
  4. human/agent message の永続化、`agent-turn-prepared` 通知、event log `TurnStarted` 追記
  5. `runtime.start_turn(TurnInput)`、turn phase → Streaming、event pump 稼働確認
- **設定系（set_model / set_permission_mode / set_plan_mode / set_session_title）の共通規則: persist-first**。SessionStore への永続化を常に先に成功させ、live runtime が存在する場合のみ trait を呼ぶ。trait 呼び出しの失敗は turn を壊さず log に留める（次回 open_session / turn 開始時の pre-turn 同期で保存値から再同期）。runtime 不在はエラーではない（現行 `model_selection.rs:119-150` / `model.rs:31-45` の best-effort を維持）。
- 並行制御: 現行 `runtime_coordinator.rs` の per-session lock / closing / pending-turn フラグを usecase 内の DI 所有状態に移す（static singleton を廃止）。
- エラー: `RuntimeUsecaseError` は `AgentBackendError::StartupTimeout` を保持し、workflow の `WorkflowStepFailureKind::StartupTimeout` 変換（`engine_error.rs:93,108`）は usecase error からの変換に置き換える。

### 8.2 event 適用（event_apply.rs + runtime_driver）

`open_session` 後、`runtime.take_events()` の Stream を `AgentTaskSpawner` で pump し、イベントごとに:

| AgentRuntimeEvent | 実行側処理 |
|---|---|
| `SessionEstablished` | `persist_agent_session_id`（trim・空無視は現行維持）、context_carry 更新（Resumed / Reinjected）、`Mismatch` は §8.5 |
| `BackendSessionCleared` | meta の `agent_session_id` クリア |
| `PartsMerged(parts)` | turn 中: live buffer へ `merge_part` 適用 → streaming 判定（§8.3）→ event log へ durable part 記録 → 1 秒間隔の parts 永続化。**turn 外（phase=Idle）: 直前 turn の agent message への post-turn 更新として適用し即時永続化**（現行 post-turn 経路の維持。契約 1） |
| `PermissionRequested(req)` | Pending: live buffer へ `Permission` part を merge → 保留 delta を flush 後 turn phase → WaitingPermission → `pending_permission_request` 付きで state 通知（現行 `run_permission_request_transition_locked` の順序保証を維持）。Resolved 到着（auto-deny 等）: part 記録のみ |
| `PermissionModeChanged(mode)` | 保存値と一致すれば無視、相違すれば保存値を優先して `runtime.set_permission_mode(saved)` を呼び戻す（現行 issues-947 の resync 挙動を Entity 経由で維持） |
| `SlashCommandsUpdated` | `agent-supported-commands-updated` 通知（形不変） |
| `TokenUsageUpdated` | `latest_token_usage` 更新 + `agent-turn-usage-updated` 通知（D11 の新イベント） |
| `TurnCompleted(result)` | event log 終端（`FinalPartsRecorded` + `TurnCompleted`/`TurnInterrupted`）→ projection で final parts / session state 確定 → parts 永続化 → streaming_parts 解放（issues-1194 の不変条件）→ state 通知 → workflow turn-complete 通知 → **pending queue drain（次 turn 起動。Crash 終端後も queue は保全されており、drain が再 open_session を含む）** |
| `Fatal` | runtime handle・parts buffer・turn phase を破棄（`close()` 呼び出し含む）。**pending queue は破棄しない**（session 状態として保全し、次の send / drain で再 open する）。turn 中に届いた場合（契約 4 違反）は防御的に `TurnCompleted(Interrupted{Crash})` 相当の終端処理を先に行う |
| stream 終端 | pump task を停止する（close 起因の正常終了。§4 契約 5） |

- **permission 応答（`respond_permission`）の順序**（現行 `respond_agent_permission_internal`（`permission.rs:393-513`）の順序を維持）: behavior 検証 → live runtime が無ければエラー返却 → `runtime.respond_permission()`（**失敗時はエラー返却し、part の patch は行わない**）→ 成功後に live buffer の該当 `Permission` part を Resolved に patch + force flush → turn phase → Streaming → state 通知 → event log `PermissionResolved` 記録。
- turn latency telemetry（`releash.agent.turn.duration_ms`）は Entity イベントから backend 非依存に記録する（ui_to_start / first event / permission wait / complete。現行 `turn_latency.rs` の Claude SDK 形状パースと `!= CLAUDE_BACKEND_ID` ゲートは削除。`query_init` metric は「turn 開始〜最初の runtime イベント」で近似する）。

### 8.3 streaming 配信（streaming.rs + presenter）

issues-1214 の確定仕様を維持する: `(session_id, message_id)` 単位の seq 付き delta、`agent-streaming-delta` payload `{chat_session_id, message_id, seq, snapshot, parts}`、coalescing（33ms / 1000 parts / 256KiB）、resync 時のみ snapshot、rollback 付き retry。判定ロジック（flush 判定・timer 判定）は現行 `stream_emit.rs` の decision 関数群を usecase の純関数として移植し、タイマー駆動と Tauri emit は `runtime_driver` + `presenter` が担う。

### 8.4 stale 監視（stale.rs）

- 対象: turn phase = Streaming のみ。`last_progress_at`（イベント到着で更新。`KeepAlive` 生存通知を含む）からの経過が `stale_timeout`（`SessionMeta.workflow_step_context.stale_timeout_secs`、上限 1800 秒、既定 180 秒）を超えたら Timeout。
- ツール実行中（ToolResult 未到着の ToolUse が streaming parts に残っている間）は、長時間コマンドの無出力が正常系であるため timeout を上限値（1800 秒）まで延長する（改訂: 当初は一律 `stale_timeout` だったが、`cargo test` 等の長時間ツール実行中に健全な turn を誤終端していたため）。
- 処理: turn を `Interrupted { Timeout }`（exit 124 相当）で終端（Error part 文言は D14 の中立文言）→ `runtime.interrupt()` を試行 → 10 秒 grace → `runtime.close()` → runtime handle を破棄（pending queue は保全）。次 turn は lazy re-open。
- これにより watchdog は backend 非依存の実行側方針となり、Claude 形式 `{"type":"interrupt"}` を Codex stdin に書く現行バグ（`recovery.rs:1471-1477`）は構造的に消える。

### 8.5 復帰計画（context restore）

- 計画決定は実行側（session の messages を所有するため）: 現行 `context_restore.rs` の `ContextRestorePlan`（Resume / Reinject / NoContext）決定ロジックを `usecase/agent_session/runtime/context_restore.rs` に置き、`Resume` は `SessionSpec.resume` として backend へ渡す。`Reinject` は実行側が初回 turn の prompt に `prompt_prefix` を前置する（両 backend 共通の prompt 操作であり backend 処理ではない）。
- `ResumeOutcome::Mismatch` 受信時: 進行中 turn を pending queue へ requeue → meta の `agent_session_id` / `context_carry` をクリア → runtime を `close()` → Reinject 計画で再 `open_session` → pending turn 再開（現行 `handle_session_ready_resume_mismatch` の挙動を Entity 経由で維持）。
- `ContextCarryState`（Resumed / Reinjected / Failed）の永続化・`agent-session-context-carry-updated` 通知は現行どおり実行側。

---

## 9. Surface / protocol 仕様

### 9.1 Tauri command（変更点のみ）

| command | 変更 |
|---|---|
| `start_agent_session` / `send_agent_message` / `interrupt_agent_query` / `close_agent_session` / `cancel_agent_queued_turn` / `init_agent_sessions` / `get_session` / `set_session_backend` | controller は `AgentSessionRuntimeUsecase` を呼ぶだけに変更（`session.rs:761-776` の CODEX 分岐、`AgentProcessMap` / registry の直接 State 受け取りを廃止） |
| `set_agent_permission_mode` | `model.rs:26-47` の CODEX 分岐を削除し usecase 呼び出しへ一本化 |
| `set_agent_model` | usecase 呼び出しへ（検証は registry、反映は trait `set_model`、persist-first） |
| `respond_agent_permission` | 引数 `(chat_session_id, request_id, behavior: "allow"/"deny", message?, updated_input?)` を維持。**controller は updated_input JSON から `answers` キーを抽出して `PermissionResponse::Allow { updated_input, answers }` に分離する**（現行 `apply_updated_input_to_permission_result`（`permission.rs:515-527`）/ `permission_grant_from_updated_input` の規則を踏襲） |
| `present_agent_permission_request` | 引数を `(chat_session_id, request_id)` に変更。usecase が保持する `PermissionRequest`（または parts 内の該当 part）から presentation を構築（`PermissionRequestBody` variant 参照） |
| `set_session_title` | `stored_session.rs:132-147` の CODEX 分岐を削除。常に usecase 経由で `set_session_title` を試行（no-op backend は成功扱い） |
| `scan_agent_skills` | 引数 `(cwd, backend_id?, query?, limit?)` のまま registry dispatch。**`read_codex_skill_catalog` は削除** |
| `list_mentionable_files` | `(worktree_path, query, backend_id?)` に拡張し、backend の `fuzzy_file_search` → 汎用列挙フォールバックを Rust 側で行う。**`read_codex_mentionable_files` は削除** |
| `list_agent_backends` | `BackendInfo` に `capabilities: { steering: bool }` を追加 |

### 9.2 Tauri event

| event | 変更 |
|---|---|
| `agent-streaming-delta` / `agent-session-state-changed` / `agent-permission-mode-changed` / `agent-models-updated` / `agent-supported-commands-updated` / `agent-session-context-carry-updated` / `agent-turn-prepared` / `agent-pending-message-consumed` / status 系 4 種 | 名前・意味は不変。emit は `adaptor/presenter/agent_session.rs` に集約。payload 中の permission 表現は `PermissionRequestMsg`（型付き）に変わる |
| `agent-sdk-message` | **削除**（D11。表示情報の代替供給は §6.2 の Error / SystemNotification 行と下記の新イベント） |
| `agent-turn-usage-updated` | **新設**: `{ chatSessionId, tokenUsage: { inputTokens, outputTokens, totalTokens?, contextWindowTokens? } }` |

### 9.3 wire 型（adaptor/protocol/agent_session.rs）

- `MessagePartMsg`: 現行 `MessagePart` の serde 形状（`#[serde(tag="type", rename_all="snake_case")]`、`parentToolUseId` 等の rename）を維持。差分は `permission` variant のみ:

```jsonc
{ "type": "permission",
  "parentToolUseId": "...",            // PermissionRequest.parent_tool_use_id を part level に serialize（現行互換）
  "request": {
    "id": "...", "toolUseId": "...", "toolName": "Edit",
    "kind": "tool_approval" | "plan_approval" | "question" | "permission_grant",
    "input": { ... },                  // kind=tool_approval / permission_grant
    "plan": "...", "allowedPrompts": [...],   // kind=plan_approval
    "questions": [...],                // kind=question
    "title": "...", "displayName": "...", "description": "...", "decisionReason": "..." },
  "status": "pending" | "allowed" | "denied" | "cancelled",
  "answers": { ... } }
```

- domain 型が serde を持たないため、frontend / event へ渡る全型に wire DTO を定義する: `SkillEntryMsg` / `SlashCommandMsg` / `ModelInfoMsg`（`{id, displayName, backend, modelId}`、現行 `ModelInfo` 形状） / `BackendInfoMsg` / `TokenUsageMsg` 等。形状は現行 serde 出力と同一に保つ（frontend 差分を permission / capabilities / 新イベントに限定するため）。
- frontend 型（`src/types/session.ts` の `PermissionRequest`:101-110）を上記に合わせて更新する。
- `GetSessionResponse` に `can_change_backend: bool` を追加し、frontend の複製判定（`useAgentChat.ts:1284-1299` / `BoundSessionChat.tsx:207` の `messages.length > 0 || agentSessionId` 判定）を撤去する。

### 9.4 frontend 変更一覧

1. `MessageInput.tsx:273-286,318-327` — backend_id 分岐を削除し、統一 command（`list_mentionable_files` / `scan_agent_skills`）を呼ぶ。
2. `MessageInput.tsx:706,806-808` — "Steer active turn" ラベルを `BackendInfo.capabilities.steering` 由来に変更（D16(b) の軽微変更）。
3. `useAgentSdkListeners.ts` — `agent-sdk-message` listener（253-266）を削除し、`agent-turn-usage-updated` listener に置換。`modelUsage` 解析（177-219）・result エラー表示（147-171）・system テキスト表示（110-145）は Rust 側供給（§6.2）に置き換わるため撤去。
4. `PermissionDialog` / `ChatSessionView` — 型付き `PermissionRequestMsg` へ追従（`present_agent_permission_request` の id 引数化含む）。表示・操作は不変。
5. `BoundSessionChat.tsx:194-198` — 既定 model の frontend フォールバックを削除（`selected_model` は Rust が常に非 null で返す契約が既にある）。
6. `ModelSelector.tsx` の backend アイコン map は表示メタデータとして存置（metadata 利用は許容）。

### 9.5 保存・転送モデルの配置（USECASE.md との整合記録）

- 現行の serde 付き保存・転送モデル（`ChatSession` / `ChatMessage` / `SessionMeta` / `AgentSessionEvent` / `SessionPage` / `GetSessionResponse` 等）は **usecase/agent_session/session/ と event_log/ に存置**し、runtime 実行側の境界（event_apply / storage 呼び出し時）で domain Entity と相互変換する。
- これは USECASE.md「ドメイン型 ↔ DTO の変換工程は存在しない」/ GATEWAY.md「永続化モデルは gateway の command_models」との**意図的な逸脱**である。理由: これらの型は QueryService の Response（読み取り要求起点の DTO）ではなく**保存・転送の正典モデル**であり、adaptor へ移すと `SessionStore`（usecase）が adaptor 型へ依存する逆依存が生じる。完全な分離は storage 再設計（本 Issue の Non-goal）を要するため、別 Issue として記録する。
- 読み取り経路（`get_session_page` 等）は現行どおり storage から read model を直接組み立てる（Entity 経由の詰め替えをしない）。
- review-02 E-1/G-4 追記: `event_log/projector.rs` は保存 DTO `MessagePart` を正典 read model へ射影する責務を持つため、本 Issue では `session::apply_tool_result_update` への委譲までに留め、projector 全体を domain `merge_part` へ完全置換しない。完全置換には保存 DTO と domain Entity の二重表現解消、および `SessionStore` の read model 再設計が必要であり、上記 storage 再設計の後続 Issue とする。

---

## 10. 永続化仕様

- layout（dir 形式 / meta.json / index.json / messages/ / events.json / attachments/ / tool_outputs/ / private_context.json）は不変。
- `SessionMeta.backend_id`: `Option<String>` → `String`（必須）。読込時に欠落していれば invalid session 隔離（既存 `invalid_sessions` 機構）。新規作成時は registry の default 解決で必ず埋める（現行 `ensure_session_backend_selected` / `resolve_session_backend` を継続使用）。
- `agent_session_id` は従来どおり Claude session id / Codex thread id の共用フィールド（issues-1190 合意維持）。
- events.json: `PermissionRequested` は型付き permission payload（§9.3 と同形）を保存。`InterruptReason` は `abort | timeout | crash`。その他の event 形状は不変。
- messages: `MessagePart::Permission` の保存形が §9.3 の形に変わる。その他 part は不変。
- 旧形式 session の migration は行わない（D15）。

---

## 11. Registry / 設定 / DI

- `AgentBackendRegistry`（usecase）: `register(Arc<dyn AgentBackend>)` / `get(id)` / `list()` / `resolve_backend_id` / `resolve_default_id` / `default_model_for` / `available_models(id)`（backend の `available_models()` を `ModelInfo` DTO 化） / `resolve_model_entry("backend:model" | bare)`。現行 `runtime/mod.rs:315-544` のロジックを移設し、`config_models_for` の config.toml フォールバックは削除する（両 backend とも固定カタログのため dead path。`agents.<backend>.models` config 参照も削除）。
- `SessionBackendResolver`（usecase port）は registry が従来どおり実装。
- `domain/app_config/repository.rs` の `codex_cli_path()` は `cli_path_for(backend_id: &str) -> Option<String>` に一般化（config schema は `agents.codex.cli_path` を維持し、`agents.claude.cli_path` を追加受理。未設定は backend 既定 `"codex"` / `"claude"`）。
- lib.rs（composition root）: `ClaudeBackend::new(config)` / `CodexBackend::new(config)`（infrastructure）→ registry（usecase）→ `AgentSessionRuntimeUsecase`（+ SessionStore / StatusCenter / presenter / spawner）→ `AppState.agent_session_runtime_usecase` として manage。`AgentProcessMap` / CleanupGate 等の個別 `app.manage` と、infrastructure 内の `app.try_state` service-locator 解決（`session_lifecycle.rs:1330-1339,2741-2744` 等）は全廃する。`resolver_ports.rs` の `BaseBranchResolverPort` / `MentionResolverPort` は usecase 定義の port へ移し、DI 注入に変える。
- review-02 G-6 追記: `AgentBackendRegistry` は Tauri command（backend 一覧、stored session lifecycle など）の registry/dispatch 境界そのものでもあるため、composition root の `app.manage(Arc<AgentBackendRegistry>)` は残す。これは backend_id 分岐の温存ではなく、DI container へ registry 境界を登録するための残置である。`application_lifecycle` / workflow gateway の runtime/open-tabs/status 取得は `try_state` を廃止し、composition root で登録済みの state を必須依存として扱う。将来、Tauri command も `AgentSessionRuntimeUsecase` 経由の facade に統一できた時点で registry の直接 State 受け取りを削除する。
- 既存 usecase port の実装差し替え: `AgentSessionBackendLifecycleGateway` / `AgentSessionRuntimeCloser` / `AgentSkillCatalogGateway` / `CodexFuzzyFileSearchGateway` は、registry 経由で backend trait を呼ぶ薄い実装（adaptor/gateway）に置き換え、`infrastructure/agent_session/{thread_lifecycle_gateway, skill_catalog_gateway, codex_fuzzy_file_search_gateway}.rs` を削除する。`CodexFuzzyFileSearchGateway` port は backend 中立な `AgentFuzzyFileSearchGateway` に改名する。
- `runtime_driver` / `presenter` の配置根拠: いずれも usecase 定義 port（`AgentTaskSpawner` / `Clock` / `AgentSessionEventNotifier`）の実装であり、GATEWAY.md の service_impl / 既存 `adaptor/presenter/agent_status.rs` パターンに準ずる。

---

## 12. 削除一覧（実装完了時に存在してはならないもの）

| 対象 | 理由 |
|---|---|
| `infrastructure/agent_session/runtime/runtime_support/` 全 14 ファイル | D5。移設対応表は §2.1 |
| `infrastructure/agent_session/runtime/` 残余全部: `mod.rs`（旧 AgentBackend trait / AgentBackendRegistry / AgentRuntimeError / ModelInfo）, `claude.rs`, `codex.rs`, `codex_app_server.rs`, `permission_flags.rs`, `context_restore.rs`, `timeouts.rs`, `turn_latency.rs`, `runtime_coordinator.rs`（= `runtime/` ディレクトリごと削除） | claude/ codex/ 配下・usecase 実行側へ再編 |
| `infrastructure/agent_session/runtime_gateway.rs`（runtime/ の兄弟ファイル） | usecase API へ置換 |
| `infrastructure/agent_session/{thread_lifecycle_gateway.rs, skill_catalog_gateway.rs, codex_fuzzy_file_search_gateway.rs, resolver_ports.rs, startup.rs}` | backend trait / usecase port / infrastructure/process へ集約 |
| `domain/agent_session/value_objects/agent_models.rs` | D8 |
| `resources/claude-sdk-bridge.mjs` / `resources/bridge-utils.mjs`（+test） / `scripts/build-bridge.mjs` / `@anthropic-ai/claude-agent-sdk` 依存 / `build:bridge` script / bridge resource 登録 / `generated/bridges/` | D3 |
| Tauri command `read_codex_skill_catalog` / `read_codex_mentionable_files`、usecase `read_codex_skill_catalog`（`skill_catalog.rs:48-66`）と `CODEX_BACKEND_ID` 定数（同:5） | D9 |
| Tauri event `agent-sdk-message` と frontend listener | D11 |
| `backend_id == CODEX_BACKEND_ID` / `!= CLAUDE_BACKEND_ID` 分岐（`session_lifecycle.rs:958,1126,1237,1416,1761,2574` / `permission.rs:427,450` / `session.rs:761` / `model.rs:26` / `stored_session.rs:132` / `skill_catalog_gateway.rs:32,62` / telemetry ゲート `session_lifecycle.rs:999,1018,1955,2772`） | requirements:37。dispatch は registry のみ |
| `unwrap_or(CLAUDE_BACKEND_ID)` フォールバック全箇所（D10 記載） | D10 |
| `adaptor/gateway/workflow/runtime_session.rs:284-306` `interrupt_agent` の stdin 直書き（`{"type":"interrupt"}`） | usecase `interrupt()` 呼び出しへ置換 |
| `MessagePart::Permission { request: serde_json::Value, status: String }` 表現・`SessionStatus.pending_permission_request: Option<serde_json::Value>` | D6（型付き DTO 化） |

`CLAUDE_BACKEND_ID` / `CODEX_BACKEND_ID` 文字列定数は、各 backend 実装の `id()` 戻り値と lib.rs の登録、session metadata の値としてのみ残る。M6 の grep 確認対象: `CODEX_BACKEND_ID ==`、`unwrap_or(CLAUDE_BACKEND_ID`、`agent-sdk-message`、`"type":"interrupt"`、`"type":"setModel"`、`codex_file_change`。

---

## 13. 検討した代替案

| 代替案 | 却下理由 |
|---|---|
| Node bridge を Claude infrastructure 内に温存し、境界だけ整える | requirements:39 は bridge 温存でも満たせるが、前提 2（§0.1）に反する。公式 SDK は CLI の薄いラッパーであり、bridge は Rust→Node→CLI の 3 プロセス構成・独自 protocol・SDK バージョン追従という複雑さだけを足す。bridge 独自機能（canUseTool 配線・supportedCommands・resume rollback）は全て公式 control protocol で表現できる |
| Codex event を「共通中間 message」に変換する現行構造の温存（変換表だけ Entity 化） | requirements:38 / behavior「Claude 互換の中間 message を受け取らない」に正面から違反 |
| `AgentBackend` を flat trait（session_id 引数方式）のまま拡張 | per-session 状態（プロセス・approval 対応表・turn 相関）の所有が trait 実装側の内部 map に隠れ、現行の `AgentProcessMap` 共有と同型の問題が残る。per-session runtime オブジェクトの方が所有関係が型で表現される |
| Entity を usecase 層に置く（現行 `ChatSession` 等の改名のみ） | GLOSSARY は Session 等を agent_session domain の Entity と定義し、DOMAIN.md は entities の置き場所を domain と定める。前提 1 に反する |
| domain Entity に serde を付けて転送・保存を一本化 | DOMAIN.md「serde を domain 配下で use しない」に違反 |
| PID 管理・orphan cleanup を各 backend に複製する（D13 の代替） | 同一の OS 資源管理を 2 実装維持することになり README.md 横断原則に反する。backend 意味論を含まないため共有 utility とする（D13 の解釈記録参照） |
| stale watchdog を各 backend 実装に複製 | timeout 値の出自（workflow step 設定）と終端処理（event log / workflow 通知）が実行側の所有物であり、backend 側に置くと同じ方針の二重実装になる。実行側の方針とし、backend には `interrupt()`/`close()` だけを要求する |
| モデル一覧を backend API（Claude initialize `models` / Codex `model/list`）から動的取得 | ユーザー可視のモデル選択肢が変わる（requirements:53 違反）。固定カタログというプロダクト決定を維持し、所有権のみ backend 実装へ移す |

---

## 14. リスクと緩和

| リスク | 緩和 |
|---|---|
| Claude CLI 直接統合の細部差（stream-json のバージョン差・undocumented message） | 未知 type / フィールドを無視する前方互換規約を必須実装とする（SDK と同じ）。CLI 最低バージョン 2.0.0 をチェックし、不足時は明示エラー。`wire.rs` は公式 `sdk.d.ts` の型定義から写経し、参照 URL をコード上に残す |
| bridge 廃止で失われる挙動の見落とし（auto-allow / resume rollback / turn 相関破棄 / post-turn 更新 / mid-session Full 切替） | §6 に不変条件として明記済み（auto-allow 表 / resume rollback 規則 / 契約 1 / `--allow-dangerously-skip-permissions` 常時付与）。M7 の手動確認に「Ask で開始 → 実行中に Full へ切替」「plan mode で許可ダイアログが出ないこと」を含める |
| Codex protocol のバージョン差（collaborationMode 形式・todo_list/error item・requestUserInput method 名） | §7.5 の M2 冒頭検証で対象バージョンの実挙動を確認してから wire.rs を確定し、バージョンをコードに記録する |
| 旧保存 session が読めなくなる | 仕様上許容（requirements:48）。invalid session 隔離により起動は阻害しない。分離後に作成した session の保存→復帰は受け入れテストで担保 |
| 大規模再編による回帰 | §16 の実装順序で「新構造を通してから旧構造を削除」する。各マイルストーンで `cargo clippy -- -D warnings` / `cargo test` / `pnpm lint` / `pnpm test` を通す |
| workflow 経路の破壊（step 実行・approval chat・stale timeout・active session 表示） | workflow adaptor が使う API を §8.1 に明示（`start_turn_locked` / `acquire_session_lock` / `is_runtime_busy` / `has_live_runtime` / `active_session_ids` / `turn_phase` / `close_session`）。`StartupTimeout` の型変換経路を維持 |
| turn 失敗エラー・system 通知の表示退行（agent-sdk-message 廃止） | §6.2 の Error / SystemNotification 供給行と convert.rs テストで担保（現行 listener の subtype 集合を移植） |

---

## 15. 影響するテスト

- **domain（必須）**: `MessagePart::merge_part` の全規則（Text 連結 / ToolUse in-place / ToolResult 累積 / Permission patch / Todo 置換 / SystemNotification 置換 / Error 重複排除）、`PermissionRequest` 状態遷移、skill frontmatter parse。
- **usecase（必須）**: `AgentSessionRuntimeUsecase` を mock backend（`AgentBackend`/`AgentSessionRuntime` のテストダブル）で検証 — turn 実行手順 / pending queue（Fatal 後の保全含む） / permission 遷移順序（送信成功後に patch） / post-turn 更新適用 / stale 判定 / TurnCompleted 終端処理 / Mismatch → Reinject 手順 / persist-first 規則。event log / projector の既存テスト（`event_log/tests.rs`）は語彙変更（typed permission / InterruptReason::Crash）に追従。streaming flush 判定の純関数テストは現行 `stream_emit.rs` のテストを移植。
- **adaptor/gateway（必須）**: session_storage の保存↔復帰 round-trip（typed permission part / backend_id 必須化 / 欠損時の invalid 隔離）。
- **infrastructure（ロジックは単体で必須）**: `claude/convert.rs`・`claude/permission.rs`（wire JSON フィクスチャ → Entity、auto-allow 表（wire mode 4 値×対話/非対話）、control_response 生成、result エラー→Error part、system subtype 対応表）、`codex/convert.rs`・`codex/permission.rs`（JSON-RPC フィクスチャ → Entity、approval 応答表、permissions fallback）は純関数としてテストする（実プロセスは起動しない。TEST.md「長時間プロセスは単体で呼ばず、ロジックのみ単体テスト」）。
- **frontend**: `useAgentSdkListeners` 置換分・PermissionDialog の型付き request 追従・MessageInput の統一 command 呼び出しのテスト更新。
- 削除対象モジュールのテスト（`runtime_support` 各ファイル末尾の `#[cfg(test)]`、`session_lifecycle.rs:3030-` 等）は、対応するロジックの移設先へ意味を保って移植する。期待値の書き換えによる帳尻合わせは不可。

---

## 16. 実装順序（コンパイル可能なマイルストーン）

1. **M1: domain 契約** — entities / value_objects / gateway.rs（trait・イベント）/ services/skill_frontmatter.rs 新設。`skill_entry.rs` の serde 除去。`agent_models.rs` 削除は M4 まで保留（参照が残るため）。domain テスト。
2. **M2: Codex backend** — 冒頭で §7.5 のバージョン検証を実施。`infrastructure/agent_session/codex/` を新設し、既存 `codex_app_server.rs` / `codex.rs` の protocol 面を移植して `AgentBackend`/`AgentSessionRuntime` を実装。Claude 方言合成を Entity 変換に置換。変換テスト。（この時点では旧経路と並存し、未配線でよい）
3. **M3: Claude backend** — `infrastructure/agent_session/claude/` を新設し、CLI 直接統合を実装。変換・permission テスト。
4. **M4: 実行側** — `usecase/agent_session/runtime/` + registry 移設 + `infrastructure/process/` + `runtime_driver` + presenter。event log 語彙更新（typed permission / InterruptReason::Crash）。`AgentSessionRuntimeUsecase` を lib.rs で配線し、controller / workflow adaptor を新 API に切替。`agent_models.rs` 削除。
5. **M5: surface 追従** — protocol 型・`present_agent_permission_request` の id 化・`agent-sdk-message` 廃止と `agent-turn-usage-updated` 新設・統一 command・frontend 追従。
6. **M6: 旧構造削除** — §12 の削除一覧を全て実施し、§12 末尾の grep 確認を行う。
7. **M7: 受け入れ確認** — §17 の traceability を検証。`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` / `pnpm lint` / `pnpm test` / `pnpm test:integration`。手動確認: Claude / Codex それぞれで新規 session → turn 実行 → permission allow/deny（AskUserQuestion 回答含む） → model 変更 → interrupt → 「Ask で開始 → 実行中に Full へ切替」 → plan mode で許可ダイアログが出ないこと → アプリ再起動 → 復帰（resume）→ workflow step 実行（stale timeout 含む）。

---

## 17. requirements / behavior 対応表

| requirements（行） | 設計対応 |
|---|---|
| :33 backend 固有処理は各 infrastructure 実装の内側 | §6 / §7（wire・lifecycle・権限・skill・復旧を claude/ codex/ に封じる）、§12（共有実装の削除） |
| :34 共有処理を持たない | D5 / D13（解釈記録含む）。共有は Entity・trait・domain service・OS utility のみ |
| :35 Entity へ変換してから返す | §4 `AgentRuntimeEvent`、§6.2 / §7.2 変換表 |
| :36 実行側は Entity と interface 経由 | §8（usecase runtime が trait のみ呼ぶ） |
| :37 backend_id は metadata + dispatch 限定 | D10 / D12 / §12 分岐削除一覧 |
| :38 Codex 中間 message 禁止 | D4 / §7.2 |
| :39 Claude 直接変換 | D3 / §6.2 |
| :40 PermissionRequest Entity | D6 / §3 / §9.3 |
| :41 復旧は backend lifecycle | §4 契約 6 / §6.4 / §7.4（stale 方針のみ実行側 = §8.4、backend には interrupt/close のみ要求） |
| :42 model は Entity / DTO | D8 / §11 |
| :43 既存 surface は Entity / interface 経由 | §9（controller / workflow / frontend の接続変更） |
| :44 通常実行・permission・復帰の成立 | §16 M7 受け入れ確認 / §14 リスク表 |
| :48-49 後方互換不要・新規保存の非破壊 | D15 / §10 |
| :50-51 生値を frontend / workflow に持ち込まない | D11 / §5.1 / §9.4 |
| :52 registry / dispatch 境界は残す | D12 |
| :53 ユーザー操作の意味を変えない | D16 / §5.2-5.3 / §6.3 auto-allow 表 / §8.1 persist-first / §13（動的モデル取得の却下） |
| :54 判断材料を落とさない | §5.4 tool 契約 / §9.3 permission wire 形（parentToolUseId / title / description / decision_reason / questions / plan / diff を保持）/ §6.2 Error・SystemNotification 供給 |

| behavior ルール | 設計対応 |
|---|---|
| backend 固有の source 値は workflow surface に露出しない | §5.1 / D14（BridgeCrash・Codex 文言の排除） |
| backend 間で実行・変換・復旧・権限・skill を共有しない | D5 / D13 / §2.1 |
| backend event は infrastructure を出る前に変換される | §4 / §6.2 / §7.2 |
| permission request は PermissionRequest Entity として表現 | D6 / §8.2 |
| turn 完了は共通の session state として観測 | §5.3 / §8.2 TurnCompleted 行 |
| backend_id は dispatch 境界で選択・選択後は分岐しない | D12 / §12 |
| Codex app-server event は Codex infrastructure が直接変換する（Claude 互換中間 message なし・Claude event handling へ流れない） | D4 / §7.2-7.3 |
| Claude SDK / bridge event は Claude infrastructure が直接変換する（Codex event handling へ流れない） | D3 / §6.2-6.3 |
| permission response は共通契約で受理され backend が変換 | §6.3 / §7.3 / §9.1 answers 分離規則 |
| 復旧は各 backend lifecycle・結果は session state | §4 契約 4-6 / §8.2 SessionEstablished・Fatal 行 |
| model choices は表示・選択用データ | D8 / §11 |
| desktop / workflow は backend 非依存 state を観測 | §8 / §9 |
| backend native 値を frontend / workflow の domain logic にしない | D11 / §5.1 / §9.4-3,5 |
| 既存実装経路の踏襲は受け入れ条件にならない | D3 / D5（bridge・共有機構の全面再編を許容） |
| 既存保存済み session は互換性保証の対象外 | D15 |
