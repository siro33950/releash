# design — issues-1246

Claude turn の各区間レイテンシを安全に計測する latency telemetry（Phase 1）の実装設計。`requirements.md` / `behavior.md` を前提とし、本書は「どのファイル・型・関数でどう実装するか」を確定する。

実装経路の事実は本リポジトリ調査に基づく（`src-tauri/src/other/telemetry/`、`bridge_common.rs`、`claude-sdk-bridge.mjs`、`infra/newrelic/`）。判断できる点は仮定を置いて進め、迷う点のみ Open Questions に残す。

## 概要

- Claude turn の 7 区間 `ui_to_start` / `bridge_spawn` / `query_init` / `first_sdk_event` / `first_assistant_event` / `permission_wait` / `complete` のレイテンシを、既存 telemetry 基盤（`HotPathMetric` と同型の enum + `record_*` + OpenTelemetry histogram/counter）を拡張して計測する。
- 計測値には本文・tool 入出力・path 等のユーザーデータを含めず、有界集合へ正規化した次元（resume 有無 / sessionId 有無 / permission mode / model / context / channel / warm path）のみを付与する。
- `query_init` は bridge（mjs）側 clock で計測し、本文を含まない telemetry イベントとして stdout 経由で Rust に転送、Rust 側で記録する。
- New Relic Terraform（`infra/newrelic/`）を同時更新し、新メトリクスを operation × context で p50/p95 表示するダッシュボードウィジェットを追加する。アラート閾値は追加しない（Non-goal）。

Phase 1 はメトリクス追加のみで、turn の観測可能挙動（応答内容・本文・成否・遅延）は不変とする。

## 変更対象

### Rust（src-tauri/）

- `src-tauri/src/other/telemetry/attributes.rs`
  - turn 区間の operation enum `AgentTurnMetric` を追加（新規）。
  - turn 次元属性キー定数 `releash.agent.*` を追加（新規）。
  - turn 次元の値型（`PermissionModeDim` / `ModelFamily` / `TurnContext` / `WarmPath` / bool 次元）と正規化関数を追加（新規）。
- `src-tauri/src/other/telemetry/mod.rs`
  - `agent_turn_duration` histogram を `Metrics` に追加し `install_metrics()` で生成（新規メトリクス `releash.agent.turn.duration_ms`）。
  - `record_agent_turn_duration(metric, &TurnDimensions, elapsed)` を追加（`record_hot_path_duration` と同型）。
- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`
  - `AgentProcess` に turn 計測状態 `turn_latency: Option<TurnLatencyState>` を追加（turn origin Instant・次元スナップショット・one-shot 記録フラグ）。
  - turn 開始（`start_agent_turn_with_runtime_spawner_locked`）で `TurnLatencyState` を初期化し、`bridge_spawn` を記録。
  - SDK message 分岐（`accumulate_sdk_message`）で `first_sdk_event` / `first_assistant_event` を one-shot 記録。
  - permission request 遷移で待ち開始時刻を保持し、permission response 遷移で `permission_wait` を記録する。
  - turn complete 遷移（`run_turn_complete_transition_locked*`）で `complete` を one-shot 記録。
  - bridge stdout の telemetry イベント（`query_init`）を受信して記録する分岐を追加。
- `src-tauri/src/adaptor/controller/command/agent_session/session.rs`
  - `send_agent_message` に `client_sent_at_ms: Option<f64>` 引数を追加し、`ui_to_start` を記録（Rust 側で算出）。
- `src-tauri/resources/claude-sdk-bridge.mjs`
  - 当該 turn の prompt yield（turn 開始・初回/turn 間の user prompt 待ち idle を除外）から subprocess init（first `system`/`init` SDK message）完了までを bridge clock で計測し、`{ "type": "telemetry", "metric": "query_init", "duration_ms": N, "turn_token": ... }` を emit。

### フロントエンド（src/）

- `src/hooks/useSessionStore.ts` の `sendAgentMessage`（送信時に `send_agent_message` を invoke する箇所）と、その呼び出し元 `src/hooks/useAgentChat.ts` に、送信時刻 `Date.now()`（epoch ms）を `clientSentAtMs` として渡す追加のみ。invoke 引数は camelCase（既存 `chatSessionId` / `permissionMode` 等に倣う）。集計・正規化・記録は行わない（`.claude/rules/rust-first-logic.md` 準拠）。
- **turn 送信のエントリは現状 Tauri invoke のみ**（実コード確認済み）。`protocol/mod.rs` の `WsMessage` enum はサーバー→クライアントの push/sync 型のみで、クライアント→サーバーの agent 送信リクエスト型は存在せず、`ws_server/routing.rs` の `route_message()` は常に `INVALID_MESSAGE` を返す空実装。したがって WebSocket 経由の turn 送信は無く、`ui_to_start` のフロント時刻付与は Tauri invoke 経路のみで足りる。`channel` 次元は配信側（stream sync）で `tauri_event` / `websocket` を取りうるため次元としては残すが、`ui_to_start` 記録時点では実質 `tauri_event` 固定となる（仮定 A-Channel を参照）。

### New Relic Terraform（infra/newrelic/）

- `locals.tf`: turn metric 名、turn operation リスト、`operation_filter.agent_turn`、turn 次元属性キー（`releash.agent.*`）を追加。
- `dashboards.tf`: turn latency ウィジェット（operation × context の p50/p95）を追加。
- `data_management.tftest.hcl`: 既存テストを壊さない。turn operation リストの健全性検証を追加（任意）。
- `alerts.tf`: 変更しない（latency budget アラートは Non-goal）。

## アーキテクチャと責務分割

### 全体フロー上の計測点

```
[Frontend]  送信時刻 client_sent_at_ms を invoke 引数に付与
    │  (Tauri invoke / WebSocket)
    ▼
[send_agent_message]  ui_to_start = now_ms - client_sent_at_ms を記録
    │
    ▼
[start_agent_turn → spawn_bridge_process]  bridge_spawn = spawn 前後の Instant 差を記録
    │
    ▼  TurnLatencyState を初期化（turn origin = Instant::now、次元スナップショット確定）
[bridge_common: turn 実行]
    │  stdin に message cmd を書き込み
    ▼
[claude-sdk-bridge.mjs]  prompt yield〜subprocess init を bridge clock で計測（prompt 待ち idle は除外）
    │  → emit { type:"telemetry", metric:"query_init", duration_ms, turn_token }
    ▼
[bridge_common: stdout 受信ループ]
    ├─ telemetry/query_init を受信 → query_init を記録
    ├─ first SDK message 受信 → first_sdk_event を one-shot 記録
    ├─ first assistant text/thinking/tool event → first_assistant_event を one-shot 記録
    ├─ permission_request → permission response → permission_wait を記録
    └─ result(turn complete) → complete を one-shot 記録
```

### 責務分割の原則

- **計測値の算出・正規化・記録は全て Rust**。フロントは送信時刻（epoch ms）を渡すのみ。bridge（mjs）は `query_init` の生 duration_ms を渡すのみで、正規化・次元付与・メトリクス記録は Rust が行う。
- **既存 telemetry 基盤を拡張**（A2）。新たな並行基盤は作らない。`record_agent_turn_duration` は `record_hot_path_duration`（mod.rs:287）と同型で、`is_performance_active()` ガード・`#[cfg(test)] record_test_metric`・`METRICS.get()` パターンを踏襲する。
- **#1214/#1195/#1178 との分離**（R6）は専用メトリクス名 `releash.agent.turn.duration_ms` と operation prefix `agent.turn.*` で担保する。既存 `hot_path` には混ぜない。

### turn 計測状態の保持

turn の各区間記録は時間軸上で分散して発生する（spawn 直後・stdout 受信ループ・complete 遷移）。これらを同一 turn として束ね、同一次元を付与し、one-shot 性を保証するため、`AgentProcess` に turn ごとの計測状態を持たせる。

```rust
// bridge_common.rs（新規）
struct TurnLatencyState {
    /// turn 開始 Instant。first_sdk_event/first_assistant_event/complete の起点。
    turn_origin: std::time::Instant,
    /// この turn の次元スナップショット（turn 開始時に確定。以後不変）。
    dims: crate::other::telemetry::TurnDimensions,
    /// one-shot 記録済みフラグ。
    first_sdk_event_recorded: bool,
    first_assistant_event_recorded: bool,
    /// permission_request 受信から permission 応答までの待ち開始時刻。
    permission_wait_started_at: Option<std::time::Instant>,
    complete_recorded: bool,
}
```

`AgentProcess`（bridge_common.rs:90 付近）に `turn_latency: Option<TurnLatencyState>` を追加。turn 開始時に `Some(..)` を設定し、complete 記録後に区間記録を止める（フラグで抑止）。次 turn 開始で再初期化する。

`ui_to_start` と `bridge_spawn` は turn 開始前後で確定するため `TurnLatencyState` には載せず、次元を別途構築して即記録する（spawn は `AgentProcess` 生成前の経路でも発生しうるため）。次元スナップショットの構築ロジックは関数 `build_turn_dimensions(..)` に集約して共有する。

## データモデルまたは型

### operation enum（attributes.rs 新規）

`HotPathMetric`（attributes.rs:32）と同じ設計。

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentTurnMetric {
    UiToStart,
    BridgeSpawn,
    QueryInit,
    FirstSdkEvent,
    FirstAssistantEvent,
    PermissionWait,
    Complete,
}

impl AgentTurnMetric {
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::UiToStart => "agent.turn.ui_to_start",
            Self::BridgeSpawn => "agent.turn.bridge_spawn",
            Self::QueryInit => "agent.turn.query_init",
            Self::FirstSdkEvent => "agent.turn.first_sdk_event",
            Self::FirstAssistantEvent => "agent.turn.first_assistant_event",
            Self::PermissionWait => "agent.turn.permission_wait",
            Self::Complete => "agent.turn.complete",
        }
    }
}
```

### 次元の型と属性キー（attributes.rs 新規）

属性キーは `releash.agent.*` プレフィックス。**`session_id` / `sessionId` / `path` 等の名前は使わない**（後述 PII pruning 制約）。「sessionId 有無」「resume 有無」は bool 値の次元として表現する。

```rust
pub(crate) const KEY_AGENT_RESUME: &str = "releash.agent.resume";              // "true" | "false"
pub(crate) const KEY_AGENT_HAS_SESSION: &str = "releash.agent.has_session";    // "true" | "false"
pub(crate) const KEY_AGENT_PERMISSION_MODE: &str = "releash.agent.permission_mode";
pub(crate) const KEY_AGENT_MODEL: &str = "releash.agent.model";
pub(crate) const KEY_AGENT_CONTEXT: &str = "releash.agent.context";
pub(crate) const KEY_AGENT_WARM_PATH: &str = "releash.agent.warm_path";
// channel は既存 KEY_CHANNEL ("releash.channel") / PayloadChannel を再利用（R3）
```

次元値（すべて有界集合）:

| 次元 | キー | 値の集合 |
|---|---|---|
| resume 有無 | `releash.agent.resume` | `true` / `false` |
| sessionId 有無 | `releash.agent.has_session` | `true` / `false` |
| permission mode | `releash.agent.permission_mode` | `ask` / `edit` / `full`（`PermissionMode::as_str()` 由来。未知は `other`） |
| model | `releash.agent.model` | `opus` / `sonnet` / `haiku` / `other` |
| context | `releash.agent.context` | `chat` / `workflow_step` |
| channel | `releash.channel` | `tauri_event` / `websocket`（既存 `PayloadChannel`） |
| warm path | `releash.agent.warm_path` | `query_direct` / `prewarm`（Phase 1 は常に `query_direct`。R7 用） |

model 正規化関数（既知ファミリへ写像、未知は `other`）:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelFamily { Opus, Sonnet, Haiku, Other }

impl ModelFamily {
    /// model_id 文字列（例 "claude-opus-4-8"）を既知ファミリへ正規化。
    pub(crate) fn normalize(model_id: Option<&str>) -> Self {
        match model_id {
            Some(id) if id.contains("opus") => Self::Opus,
            Some(id) if id.contains("sonnet") => Self::Sonnet,
            Some(id) if id.contains("haiku") => Self::Haiku,
            _ => Self::Other,
        }
    }
    pub(crate) fn as_str(self) -> &'static str { /* opus|sonnet|haiku|other */ }
}
```

次元の束（mod.rs / attributes.rs どちらでも可。記録関数が受け取る）:

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct TurnDimensions {
    pub(crate) resume: bool,
    pub(crate) has_session: bool,
    pub(crate) permission_mode: PermissionModeDim, // ask|edit|full|other
    pub(crate) model: ModelFamily,
    pub(crate) context: TurnContext,               // chat|workflow_step
    pub(crate) channel: PayloadChannel,            // 既存
    pub(crate) warm_path: WarmPath,                // query_direct|prewarm
}
```

`TurnDimensions` は `Copy` で、`KeyValue` 配列へ変換する `to_attrs(&self) -> [KeyValue; 7]` を持つ。値はすべて `&'static str`（`to_string()` 不要・割り当てなし）にして hot path のコストを抑える。

### 記録関数（mod.rs 新規）

```rust
pub(crate) fn record_agent_turn_duration(
    metric: AgentTurnMetric,
    dims: &TurnDimensions,
    elapsed: Duration,
) {
    if !is_performance_active() {
        return;
    }
    let mut attrs = vec![KeyValue::new(KEY_OPERATION, metric.operation())];
    attrs.extend(dims.to_attrs());
    #[cfg(test)]
    record_test_metric("releash.agent.turn.duration_ms", elapsed.as_secs_f64() * 1000.0, &attrs);
    let Some(metrics) = METRICS.get() else { return; };
    metrics.agent_turn_duration.record(elapsed.as_secs_f64() * 1000.0, &attrs);
}
```

`Metrics` 構造体（mod.rs:58）に `agent_turn_duration: Histogram<f64>` を追加、`install_metrics()`（mod.rs:99）で
```rust
agent_turn_duration: meter
    .f64_histogram("releash.agent.turn.duration_ms")
    .with_unit("ms")
    .build(),
```
を追加する。operation を histogram の FACET 属性として持たせるため、operation 別 metric 名は分けず単一 histogram + `releash.operation` 属性で表現する（既存 `hot_path` と同方式）。

### bridge → Rust telemetry イベント（mjs / protocol）

bridge stdout JSON（本文を含まない）:

```json
{ "type": "telemetry", "metric": "query_init", "duration_ms": 123.4, "turn_token": "<streaming_message_id>" }
```

Rust 側は既存の stdout 行パース（`accumulate_sdk_message` へ至る分岐）で `type == "telemetry"` を先に判定し、`AgentProcess.turn_latency` の次元で `record_agent_turn_duration(QueryInit, dims, Duration::from_secs_f64(duration_ms/1000.0))` を記録する。`turn_token` が現在の `active_turn_token` と一致する場合のみ記録（stale 抑止）。protocol への新規型追加は不要（内部 stdout プロトコルで完結）だが、telemetry イベント形を定数化して mjs/Rust 双方でキー名を一致させる。

## 処理フロー

### ui_to_start

- フロント: invoke 時に `client_sent_at_ms = Date.now()` を付与。
- `send_agent_message`（session.rs:1227）に `client_sent_at_ms: Option<f64>` を追加。
- Rust 側で `now_ms = SystemTime::now()` の epoch ms を取り、`elapsed = max(0, now_ms - client_sent_at_ms)` を `Duration` 化して `record_agent_turn_duration(UiToStart, dims, elapsed)`。
- 次元 `dims` はこの時点で確定可能なもの（permission_mode・model_id・channel・context・resume/has_session）から構築。`None` の場合は記録をスキップ（負値・欠損で誤計測しない）。

### bridge_spawn

- `spawn_bridge_process`（bridge_common.rs:6099 付近の呼び出し）の前後を `Instant` で挟み、`record_agent_turn_duration(BridgeSpawn, dims, elapsed)`。
- bridge が再 spawn されず既存プロセスを再利用する turn では `bridge_spawn` を記録しない（spawn が発生していないため。区間判別の意味を保つ）。

### query_init（bridge clock）

- mjs: `const t0 = performance.now()` を当該 turn の prompt が yield される時点（turn 開始・初回/turn 間の user prompt 待ち idle を除外）で取り、その turn の最初の `system`/`init` SDK message を受けた時点で `performance.now() - t0` を `duration_ms` として emit。
- Rust: 受信して `QueryInit` を記録。

### first_sdk_event / first_assistant_event

- `accumulate_sdk_message`（bridge_common.rs:3315）の冒頭で、`turn_latency` が `Some` かつ `first_sdk_event_recorded == false` の場合、`first_sdk_event` を `Instant::now() - turn_origin` で記録しフラグを立てる（msg_type 不問の最初の SDK message）。
- assistant の text/thinking/tool_use を含む分岐（`"stream_event"` の text_delta/thinking_delta、`"assistant"` の tool_use）に到達した最初の時点で `first_assistant_event` を one-shot 記録。
- いずれも `turn_token` が現 turn と一致する場合のみ（stale 抑止）。

### permission_wait

- `run_permission_request_transition_locked` で permission request 遷移を処理する時点で、現在の `TurnLatencyState` に `permission_wait_started_at = Some(Instant::now())` を保持する。既に待ち開始時刻がある場合は上書きしない。
- `apply_respond_permission_locked` で `WaitingPermission → Streaming` に戻る transition が成立したとき、`permission_wait_started_at.take()` からの elapsed を `record_agent_turn_duration(PermissionWait, dims, elapsed)` で記録する。
- 同一 turn 内に複数の permission request/response が発生する場合は、各 request/response ペアごとに `permission_wait` を記録する。request_id・tool input・回答内容は属性に含めず、他の turn latency と同じ `TurnDimensions` のみを付与する。

### complete

- `run_turn_complete_transition_locked*`（bridge_common.rs:1714/）で、`turn_latency` が `Some` かつ `complete_recorded == false` のとき `complete` を `Instant::now() - turn_origin` で記録。記録後フラグを立て、`turn_latency = None`（または次 turn で再初期化）。
- 成功/失敗（exit_code）に関わらず complete を記録する。区間レイテンシ計測が目的のため、`OpStatus` のような成否は turn latency には付与しない（次元は有界集合の 7 種に限定）。

### permission 待ち・stale watchdog・workflow prompt の区別（R2）

- permission 待ちは専用 operation `agent.turn.permission_wait` として分離する。これにより `complete` に含まれる待ち時間を New Relic 上で通常のモデル応答待ちから定量分離できる。
- workflow system prompt 増加は `context = workflow_step` 次元で `chat` と分離して比較可能にする。
- stale watchdog は既存 watchdog ログ/メトリクスと operation 別 latency の突合で識別する。

### Terraform（infra/newrelic/）

`locals.tf` 追加:

```hcl
# metrics に追加
agent_turn = "releash.agent.turn.duration_ms"

# attr に追加
agent_context = "releash.agent.context"
# （他 releash.agent.* キーは必要に応じ FACET 用に追加）

# operation リスト
agent_turn_operations = [
  "agent.turn.ui_to_start",
  "agent.turn.bridge_spawn",
  "agent.turn.query_init",
  "agent.turn.first_sdk_event",
  "agent.turn.first_assistant_event",
  "agent.turn.permission_wait",
  "agent.turn.complete",
]

# operation_filter に追加
agent_turn = "`${local.attr.operation}` IN (${join(", ", [for op in local.agent_turn_operations : "'${op}'"])})"
```

`dashboards.tf` 追加（既存 "Diff open P95" を雛形に、threshold なし・FACET を operation と context の 2 軸）:

```hcl
widget_line {
  title  = "Agent turn latency P50 by operation x context"
  ...
  nrql_query {
    account_id = var.newrelic_account_id
    query      = "FROM Metric SELECT percentile(`${local.metrics.agent_turn}`, 50) WHERE ${local.operation_filter.agent_turn} FACET `${local.attr.operation}`, `${local.attr.agent_context}` TIMESERIES"
  }
  units { unit = "ms" }
}
widget_line {
  title  = "Agent turn latency P95 by operation x context"
  ...
  query  = "FROM Metric SELECT percentile(`${local.metrics.agent_turn}`, 95) WHERE ${local.operation_filter.agent_turn} FACET `${local.attr.operation}`, `${local.attr.agent_context}` TIMESERIES"
}
```

Rust 側 metric 名（`releash.agent.turn.duration_ms`）・operation 文字列（`agent.turn.*`）・属性キー（`releash.operation` / `releash.agent.context` 他）が Terraform 定義と一字一句一致することをレビューとテストで担保する（R8）。

## エラー処理

- `is_performance_active()` が false（telemetry 無効・未設定）の場合は全 record 関数が即 return。turn 実行に影響しない（既存ガード踏襲）。
- `client_sent_at_ms` が `None` / 未来時刻（負の elapsed）/ 異常に大きい値の場合は `ui_to_start` 記録をスキップ（誤計測の混入を防ぐ）。clamp はせず欠損として落とす。
- bridge の `query_init` telemetry イベントが欠落・不正 JSON の場合はその区間の記録を欠損として扱い、turn 実行は継続（ベストエフォート計測）。
- `turn_token` 不一致（stale）の telemetry/SDK イベントは記録しない。one-shot フラグも進めない。
- bridge clock（`performance.now()`）と Rust clock は別物だが、`query_init` は bridge 内の差分のみを転送するためクロック差の影響を受けない（差分計測）。`ui_to_start` のみフロント(webview)時刻と Rust 時刻の差を取るが、同一マシンの wall clock を参照するため実害は無視できる範囲と仮定（仮定 A-Clock）。

## テスト方針

Rust（各モジュール内 `#[cfg(test)] mod tests`、CLAUDE.md 準拠）。`record_test_metric` 経由で記録値を検証する既存テスト方式を再利用。

- **attributes.rs**:
  - `AgentTurnMetric::operation()` が 7 区間それぞれ期待文字列を返す。
  - `ModelFamily::normalize`: `claude-opus-4-8 → opus` / `claude-sonnet-4-6 → sonnet` / `claude-haiku-4-5 → haiku` / 未知識別子・`None → other`（behavior.md の Examples を網羅）。
  - `PermissionModeDim` 正規化: ask/edit/full、未知 → other。
  - `TurnDimensions::to_attrs()` が 7 属性を返し、値がすべて有界集合の `&'static str`（自由文字列を含まない）。
- **mod.rs**:
  - `record_agent_turn_duration` が `releash.agent.turn.duration_ms` に duration(ms) と全次元属性を記録する（`record_test_metric` 検証）。
  - `is_performance_active()` false 時は記録しない。
- **bridge_common.rs**:
  - one-shot: 同一 turn 内で複数の SDK message / assistant event が来ても `first_sdk_event` / `first_assistant_event` / `complete` が各 1 回のみ記録される。
  - stale: `turn_token` 不一致イベントが記録もフラグ更新もしない。
  - query_init telemetry イベントのパースと記録（正常系・不正 JSON のスキップ）。
  - permission_wait が permission request から permission response までで 1 回記録され、重複 response や request_id/tool input を属性に含めない。
  - 次元スナップショットが turn 開始時に確定し、以後のイベントで同一次元が付与される。
- **ユーザーデータ非混入**（R4・セキュリティ要件）:
  - 記録属性キー集合が `{releash.operation, releash.agent.resume, releash.agent.has_session, releash.agent.permission_mode, releash.agent.model, releash.agent.context, releash.channel, releash.agent.warm_path}` に限定され、本文・path・tool 入出力に由来する自由文字列を含まないことをテストで固定（許可キーの allowlist 検証）。
- **Terraform**: `infra/newrelic/` の `terraform test`（`*.tftest.hcl`）が緑。turn operation リストの健全性 assert を追加（既存 `session_io_operations` の検証パターンに倣う）。
- CI 同等チェック: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（src-tauri/）、`pnpm lint` / `pnpm test`（フロント変更分）。

## リスクと代替案

- **専用 metric vs hot_path への operation 追加**: R6（#1214/#1195/#1178 との分離）を最優先し、専用 metric `releash.agent.turn.duration_ms` を採用。代替（`hot_path` に operation 追加）は実装が軽いが、既存 session/git 系メトリクスと混在し分離要件を満たしにくいため不採用。
- **PII pruning による次元欠落リスク**: `infra/newrelic/locals.tf` の `pii_attribute_keys` は `session_id`/`sessionId`/`path`/`cwd` 等を削除対象とする。次元キーにこれらの語を含めると New Relic 側で属性が剪定され FACET が機能しなくなる。→ 「sessionId 有無」「resume 有無」を bool 次元（`releash.agent.has_session` / `releash.agent.resume`）として表現し、生 ID・path を一切属性化しないことで回避（R4 とも整合）。
- **hot path のオーバーヘッド**: 各 turn で通常 7 種の histogram.record（permission request が複数回ある場合は `permission_wait` のみ request/response ごとに追加）。属性値は `&'static str` で割り当てなし、`is_performance_active()` で無効時は即 return のため、observable behavior 不変（要求の「観測可能な遅延を持ち込まない」）を満たすと判断。
- **bridge とのプロトコル増設**: 新 `type:"telemetry"` 行を stdout に追加するため、Rust 側パーサが未知 type を無視せず誤処理しないか確認が必要。既存パーサは type 分岐で未知を無視する作りであれば後方互換。→ 実装時に既存 default 分岐を確認（仮定 A-Parser）。
- **warm_path 次元の前倒し**: Phase 1 では常に `query_direct`。R7（Phase 2 の before/after 比較）のため次元だけ先行導入。値が単一でもカーディナリティ増加は軽微で、後続 Phase 2 で `prewarm` 値を足すだけで同一メトリクス比較が可能になる。

## 仮定

- **A2（基盤再利用・requirements 由来）**: 既存 `HotPathMetric` / `record_hot_path_duration` / `PayloadChannel` / one-shot origin と同型で実装する。
- **A3（評価先）**: New Relic 上で operation × context の p50/p95 として評価する。
- **A-Dim-Keys**: sessionId・resume はカーディナリティと PII pruning を避けるため bool 次元（`has_session` / `resume`）で表現する。生 ID は属性化しない。
- **A-Metric-Name**: 専用 metric `releash.agent.turn.duration_ms` + `releash.operation` 属性（operation 別 metric 名は分けない）。
- **A-R2**: permission 待ちは専用 operation `agent.turn.permission_wait` で区別し、stale watchdog と workflow prompt 増加は既存 watchdog ログおよび `context` 次元との併用で区別する。
- **A-Clock**: `ui_to_start` のフロント時刻と Rust 時刻は同一マシン wall clock を参照するため差は無視できる。`query_init` は bridge 内差分のみ転送しクロック差の影響を受けない。
- **A-Parser**: bridge stdout パーサは未知 `type` を安全に無視する（実コード確認済み。`bridge_common.rs` の stdout 受信ループに default arm `_ => {...}` があり、未知 type は `accumulate_stream_or_post_turn_message` に流れる。`type:"telemetry"` を先頭分岐で捕捉すれば既存処理に影響しない）。
- **A-Channel**: channel 次元は既存 `PayloadChannel`（`tauri_event` / `websocket`）を再利用する。実コード確認の結果、turn 送信のエントリは Tauri invoke のみ（WebSocket からの agent 送信は未実装）であり、`ui_to_start` 記録時点の channel は実質 `tauri_event` 固定。`websocket` は配信側（stream sync）で取りうるため次元としては保持する。
- **A-WarmPath**: Phase 1 の warm_path は常に `query_direct`。Phase 2 で `prewarm` を追加して before/after を同一メトリクスで比較する（R7）。

## Open Questions

なし。スコープは Phase 1（latency telemetry の追加のみ）に確定済み（requirements.md / behavior.md と整合）。実装前に確認すべきだった 2 点（A-Parser の bridge パーサ未知 type 無視 / `ui_to_start` のフロント送信経路）は実コードで確認済み：bridge パーサは default arm で未知 type を安全に無視し、turn 送信のエントリは Tauri invoke のみ（WebSocket からの agent 送信は未実装のため `ui_to_start` は invoke 経路のみで足りる）。残る仮定（A-Clock 等）は実装で誤計測ガードにより吸収する。
