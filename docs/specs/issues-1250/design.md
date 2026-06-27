# Design

本書は #1250「Workflow runtime の失敗分類と failure policy enforcement」の実装設計を定義する。
要求は `requirements.md`、外部から観測される振る舞いは `behavior.md` を参照する。

本書では、失敗分類軸（`WorkflowStepFailureKind` / `FailureDisposition`）と 4 ポリシー
（`RetryPolicy` / `TimeoutPolicy` / `ParallelFailurePolicy` / `StructuredOutputRepairPolicy`）の
型・配置・既定値・enforcement 経路・telemetry 送出形式・テスト方針を確定する。

---

## 1. 概要

現状、Workflow / Step の失敗は「exit_code != 0 か否か」にほぼ一元化され、
`WorkflowExecutionState::Failed { reason: String }`、`TurnCompleteDecision::SessionError { exit_code }`、
`WorkflowEvent::{NodeFailed, RunFailed}` の理由文字列に潰し込まれている。
retry / timeout / repair / parallel 伝播のロジックは domain ではなく runtime 側
（`bridge_common/recovery.rs` の `STALE_TIMEOUT_SECS`、`runtime_engine_impl.rs` の
`MAX_CONTRACT_REPAIR_ATTEMPTS`、`parallel.rs` の reduce ルール）に個別実装で散在している。

本 Issue は次を行う。

1. **分類軸を domain に新設する**。`WorkflowStepFailureKind`（失敗の発生源分類）と
   `FailureDisposition`（retryable / partial / terminal / user-action-required）を
   `domain/workflow/value_objects/` に置く。各 kind は「取りうる disposition」を識別できる。
2. **4 ポリシーを domain service として定義し、責務境界を確定する**。各ポリシーは「何を決定するか」
   のみを純粋関数／値オブジェクトで持ち、I/O は持たない。決定の実行（enforcement）は既存 runtime 経路が担う。
3. **既存の失敗表現を分類へマッピングし、enforcement を実装する**。`SessionError { exit_code }` を
   `WorkflowStepFailureKind` へ写像し、startup timeout の retry・stale timeout の template 別適用・
   model refusal の parallel 委譲・structured output の repair を分類に基づいて駆動する。
4. **telemetry に分類情報を乗せる**。#1209 の `other/telemetry` 記録 API に failure 専用の
   record 関数を追加し、`failure_kind` / `retry_count` / `timeout_kind` を attribute として送る。
   #1209 が未稼働でも no-op として安全に呼べる構造にする。

ロジックは全て Rust（domain / adaptor）に置く（`rust-first-logic`）。UI ロジックは変更しない。
ただし `WorkflowExecutionStateView` / state notification で failure kind・retry count を観測できるように、
public frontend contract としての TypeScript 型は Rust 側の wire 契約に追従して更新する。

> **設計の柱**: 分類とポリシーは「決定（policy）」、runtime は「実行（enforcement）」に分離する。
> ポリシーは pure（外部依存・I/O なし）にし、domain 層に閉じる。既存の event log / state transition /
> telemetry emission の一貫性を壊さず、失敗表現に分類情報を**付加**する（既存フィールドは互換のため残す）。

---

## 2. 変更対象

### 2.1 新規追加（Rust / domain）

| パス | 役割 | レイヤー |
|---|---|---|
| `src-tauri/src/domain/workflow/value_objects/failure.rs` | `WorkflowStepFailureKind`、`FailureDisposition`、`TimeoutKind`、`FailureClassification`（kind + disposition + 観測属性）の定義と分類表 | domain |
| `src-tauri/src/domain/workflow/services/failure_policy.rs` | `RetryPolicy` / `TimeoutPolicy` / `ParallelFailurePolicy` / `StructuredOutputRepairPolicy` の型と決定ロジック（pure） | domain |

> **仮定 D1（配置）**: 分類とポリシーは domain の value_objects / services に置く（requirements A2）。
> domain は外部依存を持たない規約のため、ポリシーは I/O・時刻取得・telemetry 送出を一切行わず、
> 「入力（failure kind / 試行回数 / node kind / template）→ 決定（retry 可否・timeout 値・伝播方針）」
> の純粋関数で表現する。実行は adaptor/gateway の runtime が担う。

### 2.2 変更（Rust）

| パス | 変更内容 |
|---|---|
| `src-tauri/src/domain/workflow/value_objects/state.rs` | `WorkflowExecutionState::Failed` に `kind: WorkflowStepFailureKind` を追加（`Failed { reason, kind }`）。既存 `reason` は人間可読理由として残す。`as_str()` / `is_terminal()` は変更しない。 |
| `src-tauri/src/domain/workflow/services/transition.rs` | `decide_turn_complete_action` の `exit_code != 0` 分岐で、`exit_code` を `WorkflowStepFailureKind` へ分類（`classify_session_error(exit_code)`）。`TurnCompleteDecision::SessionError` / `TurnCompleteMutationPlan::SessionError` に `kind` を付加。 |
| `src-tauri/src/domain/workflow/services/parallel.rs` | child 失敗の disposition 判定に `ParallelFailurePolicy` を適用。reduce ルール（`AnyNeedsFix` 等）自体は維持し、「単一子失敗を全体 failed にするか aggregate 委譲するか」の決定を policy 経由にする。 |
| `src-tauri/src/adaptor/gateway/workflow/event.rs` | `NodeFailed` / `RunFailed` に `failure_kind: WorkflowStepFailureKind`（serde で文字列化）と `retry_count: Option<u32>` を追加。`ContractRepairRequested` は `StructuredOutputMismatch` への対応として維持。 |
| `src-tauri/src/adaptor/gateway/workflow/runtime_engine_impl.rs` | (a) `MAX_CONTRACT_REPAIR_ATTEMPTS` を `StructuredOutputRepairPolicy` 経由に置換。(b) retryable な step failure の再試行可否を `RetryPolicy` で決定。(c) node 失敗確定時に telemetry record を呼ぶ。 |
| `src-tauri/src/adaptor/gateway/workflow/runtime_session.rs` | step session 作成時に `TimeoutPolicy` / `RetryPolicy` から startup timeout・stale timeout・startup retry budget を計算し、`WorkflowStepContext` へ注入する。 |
| `src-tauri/src/domain/workflow/value_objects/workflow_step_context.rs` / `src-tauri/src/usecase/agent_session/session/mod.rs` | `WorkflowStepContext` / `WorkflowStepContextDto` に `startup_max_retries` を追加し、workflow gateway で計算済みの startup retry budget を session meta へ保存する。 |
| `src-tauri/src/infrastructure/agent_session/runtime/codex.rs` | Codex app-server 起動待ちで、session meta に注入済みの startup timeout / stale timeout / startup retry budget を消費する。`RetryPolicy` や `WorkflowStepFailureKind` には依存しない。 |
| `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/recovery.rs` | `STALE_TIMEOUT_SECS` 等の hard-coded 値を `TimeoutPolicy` と adaptor/gateway 側で組み立てた override に寄せる（値は後退させない。下記 D5 参照）。stale 確定時の exit_code 124 / `InterruptReason::Timeout` は維持。 |
| `src-tauri/src/other/telemetry/attributes.rs` | failure 用 attribute キー（`KEY_FAILURE_KIND` / `KEY_FAILURE_DISPOSITION` / `KEY_RETRY_COUNT` / `KEY_TIMEOUT_KIND`）と、`WorkflowStepFailureKind`/`FailureDisposition`/`TimeoutKind` の `as_str()` 写像を追加。 |
| `src-tauri/src/other/telemetry/mod.rs` | `record_workflow_step_failure(kind, disposition, retry_count, timeout_kind)` を追加。`operation_status` counter に failure attribute 付きで 1 加算。no-op フォールバックは既存パターンを踏襲。 |
| `src/types/workflow.ts` | `WorkflowStepFailureKind` / `FailureDisposition` / `retryCount` / failed child state を state notification の public contract として追加する。表示ロジックは追加しない。 |

> **仮定 D2（telemetry の置き場）**: telemetry の record API は #1209 の `other/telemetry` を再利用する
> （requirements A3）。本 Issue は計装基盤を新設せず、failure 用の record 関数と attribute キーのみ追加する。
> #1209 が未稼働（`METRICS` 未初期化）でも `record_*` は早期 return で no-op になるため、
> 「分類情報を telemetry へ渡せる構造」は #1209 の状態に依らず成立する（behavior の最後の Scenario を満たす）。

### 2.3 変更しないもの

- workflow エンジンの実行モデル・スケジューリング・DAG 構造（非スコープ）。
- 手動 Resume 操作・既存 run の継続セマンティクス（#965 が所有）。
- `bridge_common.rs` の責務別 module 分割（#1217 が所有）。
- OTel / New Relic 計装基盤そのもの（#1209 が所有）。
- model refusal の検出精度・provider 固有判定ロジック（既存判定結果を分類へ落とすだけ）。
- フロントエンドの UI 表示・操作ロジック。本 Issue で更新するのは workflow state notification を受ける
  TypeScript public contract 型に限る。

---

## 3. アーキテクチャと責務分割

### 3.1 policy（決定）と enforcement（実行）の分離

```text
                 決定（pure, domain）                     実行（I/O, adaptor/infra）
  ┌───────────────────────────────────────┐    ┌──────────────────────────────────────┐
  │ WorkflowStepFailureKind / Disposition  │    │ runtime_engine_impl.rs               │
  │ RetryPolicy.should_retry(kind, n)      │──▶ │  - retry ループ駆動                   │
  │ TimeoutPolicy.timeout(kind, node, tpl) │──▶ │  - 待機判定への timeout 値適用        │
  │ ParallelFailurePolicy.propagate(...)   │──▶ │  - parallel 伝播の分岐               │
  │ StructuredOutputRepairPolicy.next(...) │──▶ │  - repair turn 起動 / 上限打ち切り    │
  └───────────────────────────────────────┘    │ bridge_common/recovery.rs            │
                                                │  - stale timeout 監視（TimeoutPolicy 値）│
                                                │ other/telemetry                      │
                                                │  - record_workflow_step_failure(...)  │
                                                └──────────────────────────────────────┘
```

ポリシーは「入力 → 決定」だけを返す純粋関数の集合とし、決定をどう実行するかは runtime に委ねる。
これにより各ポリシーの責務境界がテスト可能になり、enforcement 経路の差し替え（startup timeout の
場所など）に影響されない。

### 3.2 4 ポリシーの責務境界（何を決定し、何を決定しないか）

| ポリシー | 決定する（責務内） | 決定しない（責務外） |
|---|---|---|
| `RetryPolicy` | failure kind ごとの retry 可否・retry 回数上限。「今 retry すべきか／上限に達したか」 | timeout 値（TimeoutPolicy）、parallel 伝播（ParallelFailurePolicy）、repair の prompt 内容 |
| `TimeoutPolicy` | model / node kind / workflow template ごとの startup / stale timeout 値 | timeout 後に retry するか（RetryPolicy）、timeout の監視実装 |
| `ParallelFailurePolicy` | parallel child の単一失敗を「全体 failed」にするか「aggregate へ委譲（partial 受容）」するか | reduce の集約ロジック（既存 `apply_reduce`）、child の retry（RetryPolicy） |
| `StructuredOutputRepairPolicy` | structured output mismatch 時に repair / reroute を試みるか、試行上限超過時の扱い（terminal 化） | repair prompt の生成、contract 検証ロジック自体、retry（別概念。下記 D3） |

> **仮定 D3（retry と repair の境界）**: `RetryPolicy` は「同一処理の再実行」（startup / stale timeout）を、
> `StructuredOutputRepairPolicy` は「出力不整合の修復試行」（追加 turn で正しい structured output を促す）を扱う。
> 両者は試行回数の概念を持つが対象が異なり、重複しない。structured output mismatch に retry は適用せず、
> 必ず repair policy が扱う（責務の単一所属）。

### 3.3 レイヤー依存

`domain/workflow/value_objects/failure.rs` と `services/failure_policy.rs` は domain 内に閉じ、
外部 crate（tauri / serde を含む）に依存しない。domain 型は `as_str()` などの純粋な表現変換だけを持ち、
serde の `Serialize` / `Deserialize` 実装、wire 互換の default、イベント永続化上の既定値は
`adaptor/gateway/workflow/failure_wire.rs` など adaptor/protocol/gateway 側に置く。分類ロジック自体は
転送形式に依存しない。adaptor（runtime_engine_impl / event / runtime_session）がこれらを参照し、
infrastructure は gateway から注入された timeout 値・retry budget のような計算済み runtime 値だけを消費する。
依存方向は `adaptor → domain` で規約に沿う。

---

## 4. データモデルまたは型

### 4.1 失敗分類

```rust
// domain/workflow/value_objects/failure.rs

/// 失敗の発生源による分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStepFailureKind {
    StartupTimeout,            // Codex app-server 起動遅延
    StaleRuntimeTimeout,       // 重い判断 step の応答停止（stale）
    ModelRefusal,              // model refusal / provider policy 拒否
    StructuredOutputMismatch,  // structured output の contract 不整合
    ValidationFailure,         // 出力 validation の失敗
    UserAbort,                 // user による abort
    InfrastructureCrash,       // bridge EOF / プロセス crash 等
}

/// failure kind が取りうる扱い。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureDisposition {
    Retryable,
    Partial,
    Terminal,
    UserActionRequired,
}

/// timeout 起因の失敗の種別（telemetry の timeout_kind 用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Startup,
    Stale,
}

impl WorkflowStepFailureKind {
    /// 既定 disposition（最終的な扱いは適用ポリシーが決定しうる。behavior B1）。
    pub fn default_disposition(self) -> FailureDisposition { /* 下表 */ }

    /// timeout 起因なら対応する TimeoutKind を返す。
    pub fn timeout_kind(self) -> Option<TimeoutKind> { /* Startup/Stale */ }

    pub fn as_str(self) -> &'static str { /* telemetry / event 文字列化 */ }
}
```

### 4.2 failure kind → 既定 disposition の分類表（behavior の Examples と一致）

| failure kind | 既定 disposition | timeout_kind | 既存の表現元 | enforcement |
|---|---|---|---|---|
| StartupTimeout | Retryable | Startup | （新設。Codex 起動待ちの timeout） | RetryPolicy で再起動。上限超過で Terminal |
| StaleRuntimeTimeout | Retryable | Stale | `STALE_EXIT_CODE = 124` / `evaluate_turn_liveness` | TimeoutPolicy 値で監視、RetryPolicy で retry or Terminal |
| ModelRefusal | Partial | — | parallel child の `result`/`verdict`、refusal 判定 | ParallelFailurePolicy で aggregate 委譲（単独 step では Terminal） |
| StructuredOutputMismatch | Retryable | — | `ContractRepairRequested { attempt }` | StructuredOutputRepairPolicy で repair。上限超過で Terminal |
| ValidationFailure | Terminal | — | `UnexpectedNodeType` / contract 不能系 | retry/repair せず Terminal |
| UserAbort | UserActionRequired | — | `WorkflowExecutionState::Aborted` / `RunAborted` | retry せず、failed 系と区別 |
| InfrastructureCrash | Terminal | — | bridge EOF（exit -1）/ `InterruptReason::BridgeCrash` | retry せず Terminal（D6 参照） |

> **仮定 D4（disposition は固定でなく「既定」）**: behavior B1 の通り、各 kind は「取りうる disposition」を
> 識別できればよく、最終的な扱いは適用ポリシーが決める。例えば StartupTimeout は既定 Retryable だが、
> RetryPolicy の上限到達後は Terminal として終了する（behavior「retry 上限を超えても回復しない場合は terminal」）。
> ModelRefusal は parallel 文脈では Partial、単独 step では Terminal になりうる（ParallelFailurePolicy が決定）。

> **仮定 D6（InfrastructureCrash の扱い）**: bridge EOF / crash は現状 retry しない（recovery.rs は
> turn-complete で error 記録のみ）。本 Issue では現状挙動を後退させないため既定 Terminal とし、
> crash の自動 retry は導入しない（非スコープの「実行モデル変更」に踏み込まないため）。将来 retry 対象に
> するかは #965 / 後続課題で判断する。

### 4.3 ポリシー型

```rust
// domain/workflow/services/failure_policy.rs

pub struct RetryPolicy { /* kind ごとの上限テーブル */ }
impl RetryPolicy {
    pub fn default() -> Self;
    /// これまでの試行回数 attempts（0 始まり）を踏まえ、再試行すべきか。
    pub fn should_retry(&self, kind: WorkflowStepFailureKind, attempts: u32) -> bool;
    pub fn max_retries(&self, kind: WorkflowStepFailureKind) -> u32;
}

pub struct TimeoutPolicy { /* (model, node_kind, template) → 値 */ }
impl TimeoutPolicy {
    pub fn default() -> Self;
    pub fn with_stale_timeout_for_model(self, model: impl Into<String>, timeout: Duration) -> Self;
    pub fn with_stale_timeout_for_template(self, template: impl Into<String>, timeout: Duration) -> Self;
    pub fn with_stale_timeout_for_node_kind(self, node_kind: NodeType, timeout: Duration) -> Self;
    pub fn startup_timeout(&self, ctx: &TimeoutContext) -> Duration;
    pub fn stale_timeout(&self, ctx: &TimeoutContext) -> Duration;
}

pub enum ParallelPropagation { FailWorkflow, DelegateToAggregate }
pub struct ParallelFailurePolicy { /* node 単位の伝播既定 */ }
impl ParallelFailurePolicy {
    pub fn default() -> Self;
    pub fn on_child_failure(
        &self,
        kind: WorkflowStepFailureKind,
        aggregate: Option<&ParallelAggregate>,
    ) -> ParallelPropagation;
}

pub enum RepairDecision { Repair { attempt: u32 }, GiveUp }
pub struct StructuredOutputRepairPolicy { max_attempts: u32 }
impl StructuredOutputRepairPolicy {
    pub fn default() -> Self;  // max_attempts = 2（現状維持）
    pub fn decide(&self, prior_attempts: u32, has_session: bool) -> RepairDecision;
}
```

`TimeoutContext` は `model` / `node_kind` / `workflow_template` を持つ読み取り専用構造体。domain の
`TimeoutPolicy::default()` は汎用既定値と「渡された override map を評価する」純粋ロジックに留める。
具体的な provider model ID や builtin workflow template 名の override map は、
`adaptor/gateway/workflow/failure_policy_config.rs` のような workflow 定義を知る設定組み立て層で
`with_stale_timeout_for_*` により注入し、runtime がその policy と context を渡す。
`RetryPolicy` の startup retry 上限も同じく workflow gateway 側で `startup_max_retries` として
`WorkflowStepContext` に注入し、Codex infrastructure は計算済みの数値 budget だけを消費する。

### 4.4 既定値（現状からの後退回避）

| ポリシー | 既定値 | 根拠 |
|---|---|---|
| `RetryPolicy` | StartupTimeout: 上限 **2**、StaleRuntimeTimeout: 上限 **0**（= 現状通り即終了）、その他: 0 | startup は新規改善のため retry を入れる。stale は現状 retry なしのため後退回避で 0（D7） |
| `TimeoutPolicy.stale_timeout` | domain 既定 **180 秒**（= `STALE_TIMEOUT_SECS`）。adaptor/gateway の設定組み立て層から model/template/node kind 別の上書きを注入可能 | 現状値を既定として温存。具体的な provider model ID / builtin workflow 名は domain に置かない |
| `TimeoutPolicy.startup_timeout` | 既定 **30 秒**（仮、D8） | 現状 startup 専用 timeout は未分離。即 fail せず retry する余地を作る最小値 |
| `ParallelFailurePolicy` | model refusal は **DelegateToAggregate**、それ以外の child 失敗は既存挙動（aggregate 条件に従う） | behavior「review child の refusal は全体を巻き込まない」を満たす |
| `StructuredOutputRepairPolicy` | `max_attempts = 2`（= `MAX_CONTRACT_REPAIR_ATTEMPTS`） | 現状値を温存 |

> **仮定 D7（stale retry の既定 0）**: 現状 stale timeout（exit 124）は retry されず終了する。本 Issue の主眼は
> 「重い判断 step を timeout 値の適切な適用で失敗扱いにしない」ことであり（TimeoutPolicy 側で解決）、
> stale 後の自動 retry は現状未提供のため既定 0 とし observable behavior を後退させない。retry を入れると
> 二重処理リスクがあるため、有効化は後続課題に委ねる。
>
> **仮定 D8（startup timeout 既定値）**: Codex app-server 起動の現状 timeout は明示分離されていない。
> 30 秒は「即 node_failed にしない」ための仮の最小値であり、実測に基づく調整は実装時に行う。
> 値は Open Question ではなく実装で確定する（後退の懸念がない新規導入のため）。

---

## 5. 処理フロー

### 5.1 session error の分類（exit_code → failure kind）

```text
turn 完了（exit_code 取得）
  └ decide_turn_complete_action(exit_code)
       exit_code == 0 → 既存（AutoEvaluate / WaitApproval / ...）
       exit_code != 0 → classify_session_error(exit_code, interrupt_reason):
            124  + Timeout      → StaleRuntimeTimeout
            -1   + BridgeCrash  → InfrastructureCrash
            （refusal 判定あり） → ModelRefusal
            その他              → InfrastructureCrash（既定）
       → SessionError { node_name, exit_code, kind }
```

`classify_session_error` は domain の pure 関数。`InterruptReason`（recovery.rs）を domain へ渡せる形に
（enum 値のみ）写し、exit_code と合わせて kind を決める。

### 5.2 startup timeout の retry（improvement #1）

```text
Codex app-server 起動待ち（workflow gateway が TimeoutPolicy.startup_timeout(ctx) を startup_timeout_secs として注入）
  └ 経過 > startup_timeout_secs
       → kind = StartupTimeout
       → workflow gateway が RetryPolicy.max_retries(StartupTimeout) を startup_max_retries として session meta に注入
       → Codex infrastructure は attempts < startup_max_retries の間だけ再起動（attempts += 1）
            上限内で成功 → 失敗扱いにしない
            上限超過     → SessionError { kind: StartupTimeout } で node_failed（Terminal）
       → 失敗確定時は record_workflow_step_failure(StartupTimeout, ..., retry_count=attempts, Startup)
```

### 5.3 stale timeout の template 別適用（improvement #2）

```text
streaming 中の liveness 監視（recovery.rs, WATCHDOG_TICK_SECS=5）
  └ last_progress からの経過 > TimeoutPolicy.stale_timeout(ctx)
       → TimeoutKind::Stale として finalize（exit 124, InterruptReason::Timeout）
  経過が timeout 値以下 → 失敗扱いにしない（重い判断 step を救う）
```

`TimeoutPolicy.stale_timeout(ctx)` が node kind / template に応じた値を返すことで、
重い step に長い timeout を割り当てられる。既定 180 秒は維持。

### 5.4 parallel child failure の伝播（improvement #3）

```text
parallel child が失敗（例: review child の model refusal）
  └ kind = ModelRefusal
  └ ParallelFailurePolicy.on_child_failure(ModelRefusal, aggregate)
       DelegateToAggregate → workflow 全体は failed にしない。
                             child を partial として記録し、他 child と aggregate へ集約。
       FailWorkflow        → workflow 全体を failed にする。
  reduce（apply_reduce）の集約ロジックは不変（B3）
```

### 5.5 structured output repair（improvement #4）

```text
structured output が contract 不整合（kind = StructuredOutputMismatch）
  └ prior = contract_repair_attempt_count(run_id, node)
  └ StructuredOutputRepairPolicy.decide(prior, has_session)
       Repair { attempt } → ContractRepairRequested を append し repair turn 起動
       GiveUp             → fail_missing_required_output（Terminal, RunFailed）
       → 確定時 record_workflow_step_failure(StructuredOutputMismatch, ...)
```

### 5.6 telemetry 送出

node 失敗が**確定した時点**（retry/repair で回復しなかった、または partial として受容した時点）で
`record_workflow_step_failure(kind, disposition, retry_count, timeout_kind)` を 1 回呼ぶ。
`operation_status` counter に下記 attribute を付けて加算する。

| attribute key | 値 |
|---|---|
| `failure.kind` | `WorkflowStepFailureKind::as_str()` |
| `failure.disposition` | `FailureDisposition::as_str()` |
| `failure.retry_count` | retry が行われた場合のみ（`Option<u32>`） |
| `failure.timeout_kind` | timeout 起因の場合のみ（`startup`/`stale`） |

> **仮定 D9（送出形式）**: failure は新規 metric を増やさず、既存 `releash.operation.status` counter の
> attribute として送る（requirements A3 / behavior B5）。span status との二重計上を避け、ingest 量を
> 抑える #1209 の方針（counter 集約）に揃える。`#[cfg(test)]` の `record_test_metric` でテスト検証可能。

---

## 6. エラー処理

- **分類不能な exit_code**: `classify_session_error` は未知の exit_code を `InfrastructureCrash`（Terminal）に
  落とす。情報欠落で retry/partial に誤分類するより、安全側（Terminal）に倒す。
- **telemetry 未初期化**: `record_workflow_step_failure` は `METRICS` 未取得時に早期 return（no-op）。
  失敗処理本体は telemetry の成否に依存しない。
- **repair 中の session 喪失**: `StructuredOutputRepairPolicy.decide(_, has_session=false)` は `GiveUp` を返し、
  現状の `fail_missing_required_output`（"no active session" 理由）と同義の Terminal 終了にする。
- **retry 上限到達**: `RetryPolicy.should_retry` が false を返した時点で kind を保ったまま Terminal 化し、
  `NodeFailed { failure_kind, retry_count }` を append（観測者から「上限まで retry した」ことが分かる）。
- **user abort と failure の混同回避**: `UserAbort` は `WorkflowExecutionState::Aborted` / `RunAborted` 経路を
  維持し、`Failed { kind }` 系へ集約しない。telemetry の `failure.disposition` も `user-action-required` で区別。

---

## 7. テスト方針

Rust の `#[cfg(test)] mod tests` を各モジュールに置く。外部プロセス（実 Codex 起動・git push）は実行しない。

### 7.1 分類（failure.rs）

- 各 `WorkflowStepFailureKind` の `default_disposition()` が分類表（4.2）と一致する（behavior の Examples を網羅）。
- `timeout_kind()` が StartupTimeout→Startup / StaleRuntimeTimeout→Stale / その他→None。
- `as_str()` が安定した文字列を返す（telemetry/event の後方互換）。
- `UserAbort` の disposition が `UserActionRequired` で、retryable/terminal と区別される。

### 7.2 分類写像（transition.rs）

- `classify_session_error(124, Timeout)` → StaleRuntimeTimeout。
- `classify_session_error(-1, BridgeCrash)` → InfrastructureCrash。
- refusal 判定入力 → ModelRefusal。
- 未知 exit_code → InfrastructureCrash（安全側）。

### 7.3 各ポリシー（failure_policy.rs）

- `RetryPolicy::default().should_retry(StartupTimeout, n)`: n<2 で true、n>=2 で false。
- `RetryPolicy` の StaleRuntimeTimeout 上限が 0（現状後退なし）。
- `TimeoutPolicy.stale_timeout` の domain 既定が 180 秒であり、adaptor/gateway から注入された template 別上書きが効く。
- `TimeoutPolicy.startup_timeout` が既定値を返す。
- workflow gateway が `RetryPolicy.max_retries(StartupTimeout)` を `WorkflowStepContext.startup_max_retries` へ注入し、
  Codex infrastructure は domain policy ではなく注入済みの数値 budget を消費する。
- `ParallelFailurePolicy.on_child_failure(ModelRefusal, _)` = DelegateToAggregate。
- `StructuredOutputRepairPolicy.decide`: prior<2 かつ session あり→Repair、prior>=2→GiveUp、session なし→GiveUp。

### 7.4 enforcement 結合（runtime_engine_impl / parallel）

- startup timeout: 上限内の再起動成功で step が失敗扱いにならない／上限超過で Terminal + `failure_kind` 付き event。
- structured output mismatch: 上限内 repair で成功すれば失敗扱いにならない／上限超過で `RunFailed`。
- parallel: 単一 child の model refusal で workflow 全体が failed にならず、refusal child が partial として識別される。
- parallel: FailWorkflow 決定時は workflow が failed になる。

### 7.5 telemetry

- `record_workflow_step_failure` 呼び出しで `releash.operation.status` に `failure.kind` / `failure.retry_count` /
  `failure.timeout_kind` attribute が乗る（`record_test_metric` で検証）。
- `METRICS` 未初期化でも panic せず no-op（未稼働時の構造存在を担保）。

### 7.6 品質ゲート

- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

---

## 8. リスクと代替案

- **リスク 1: enforcement による observable behavior 変化**。startup retry / parallel 委譲は従来「即 failed」だった
  ケースを変える。対策として、変化箇所（startup timeout retry、model refusal の parallel 委譲、stale timeout の
  template 適用、structured output repair）に migration / behavior note を残す（§9）。既定値は現状からの後退を
  避ける方向（stale retry=0、repair=2、stale timeout=180s 維持）で設定する。
- **リスク 2: 二重処理**。startup retry は冪等な再起動に限定し、turn 実行中の retry は導入しない。stale retry は
  既定 0 にして二重実行リスクを回避する。
- **リスク 3: refusal 誤判定で正当な失敗を partial に握り潰す**。ModelRefusal の判定は既存判定結果のみを使い
  （非スコープ）、parallel 文脈に限って Partial 委譲する。単独 step の refusal は Terminal に倒す。
- **リスク 4: domain への外部依存混入**。ポリシーを pure に保ち、時刻・I/O・telemetry を持ち込まない。
  `Duration` は std のみ。enforcement 側で値を消費する。
- **代替案（不採用）**: failure ごとに新規 metric を増やす案は ingest 量増と #1209 の counter 集約方針に反するため
  不採用。attribute 付き単一 counter にする。
- **代替案（不採用）**: `WorkflowExecutionState::Failed` を kind だけにして `reason` を廃する案は、既存 UI / event
  log の人間可読理由を壊すため不採用。`reason` は残し `kind` を付加する。

---

## 9. Migration / Behavior note

本 Issue は次の observable behavior を変える。spec（本書）と event log・telemetry に分類情報が増えるため、
変更点を明記する（requirements 要求事項 7 / A4）。

1. **workflow step session の startup timeout が即 node_failed にならない**: Codex 起動遅延は
   `StartupTimeout` として gateway で注入された budget（既定は最大 2 retry）まで再起動を試みる。
   上限内で起動成功すれば step は失敗しない（従来は即失敗していた）。
   既存 app-server session に turn を送る経路の thread ready 待機も、旧実装の約 5 秒固定から
   注入済み `startup_timeout_secs`（未注入時は既定 30 秒）を使う挙動へ揃える。
2. **stale timeout が template/node kind 別になりうる**: 既定 180 秒は不変だが、template 別に延長可能。重い判断
   step が従来 180 秒で失敗していたものを救える。
3. **parallel child の model refusal が全体を巻き込まない**: 1 つの review child の refusal で workflow 全体が
   failed になっていた挙動を、aggregate 委譲（partial 受容）へ変更する。
4. **失敗 event / telemetry に分類が付く**: `NodeFailed` / `RunFailed` に `failure_kind` / `retry_count` が増え、
   telemetry の `operation.status` に `failure.*` attribute が乗る。既存フィールド（`reason` 等）は互換維持。
5. **`WorkflowExecutionState::Failed` に `kind` フィールド追加**: 直接 pattern match している箇所は `kind` を
   伴う形に追従する。`as_str()` / `is_terminal()` の戻り値は不変。

event log は append-only のため、既存 run の過去 event は変更しない。新規 run から分類情報を含む。

---

## 10. 仮定（まとめ）

- **D1**: 分類・ポリシーは domain（value_objects / services）に置き、pure に保つ。enforcement は adaptor/infra。
- **D2**: telemetry は #1209 の `other/telemetry` を再利用。failure 用 record と attribute のみ追加。未稼働でも no-op。
- **D3**: retry（同一処理再実行）と repair（出力修復）は対象が異なり責務重複しない。mismatch は repair のみ。
- **D4**: disposition は kind ごとの「既定」であり、最終的な扱いは適用ポリシーが決定（behavior B1）。
- **D5**: 既存の timeout 値（180 秒）/ repair 上限（2）は TimeoutPolicy / RepairPolicy の既定として温存。
- **D6**: InfrastructureCrash は現状通り retry せず Terminal（後退回避）。
- **D7**: stale timeout 後の自動 retry 既定は 0（現状維持）。本 Issue の改善は timeout 値の適切適用で達成。
- **D8**: startup timeout 既定 30 秒は仮値。実装時に実測で調整（後退の懸念がない新規導入）。
- **D9**: failure は新規 metric を増やさず既存 `operation.status` counter の attribute として送る。

## Open Questions

なし。
