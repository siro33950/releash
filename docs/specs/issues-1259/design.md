# Design

Issue #1259: WorkspaceList のステータス表示・集約ロジック再設計の実装仕様。

`requirements.md` の「確定ステータス仕様」と `behavior.md` の Gherkin を満たす実装方針を定義する。本書は `design.md` のみを対象とし、`requirements.md` / `behavior.md` は変更しない。

---

## 1. 概要

WorkspaceList 上で、各 Session の **稼働中(streaming) / 停止中** を判別可能にし、Step・Workflow の代表ステータスを新しい優先順位（`running > failed > error > waiting > aborted > completed > queued`）で集約する。

新規となる中核ロジックは次の 3 つで、いずれも Rust 側の純粋関数として実装する（rust-first-logic, 例外なし）。

1. **Session 結果導出**: `Step 進行ステータス × Session 稼働ステータス(AgentState) → 7 種代表ステータス`（`behavior.md` Rule 1 のクロス表）。
2. **Step 集約**: Step 配下 Session の結果のうち最強優先度を Step 代表とする（Rule 2）。
3. **Workflow 集約**: Step 代表のうち最強優先度を Workflow 代表とする（Rule 3）。

現状の課題は、Session 稼働状態（`AgentState`、`session-status-changed` で配信）が Step/Workflow 代表ステータスへ反映される経路が存在しないこと。本設計はこの経路を新設し、リアルタイム更新（R5）を既存イベント購読の枠内で満たす。

---

## 2. 現状整理（変更前）

| 関心事 | 実装 | データ源 | 更新契機 |
|---|---|---|---|
| Step 進行ステータス | `usecase/workflow/workspace_tree.rs::step_status_for_group` | `WorkflowStateSnapshot`（永続 NDJSON） | `list_workspace_worktree_nodes` 再フェッチ（`workflow-state-changed` / Session 増減） |
| Session 稼働ステータス | `usecase/agent_session/status.rs::derive_agent_state` | `AgentStatusCenter`（インメモリ） | `session-status-changed` イベント |
| Workspace 集約 | `usecase/agent_session/status.rs::aggregate*` | 同上 | `workspace-status-changed` イベント |

現状の集約優先順位（変更対象）:

- Step (`step_status_for_group`): `failed > aborted > waiting_approval > running > completed > queued`
- Workspace (`aggregate`): `error > waiting > running > done`

いずれも本 Issue の新優先順位（`running` 最強・`waiting_approval`→`waiting` 統合・`failed/error/aborted` を明示分離）とは一致せず、かつ Step 集約は Session の稼働状態を一切見ていない。

---

## 3. 変更対象

### Rust (`src-tauri/src/`)

| ファイル | 変更内容 |
|---|---|
| `domain/workflow/status_aggregation.rs`（新規）<br>※配置は「仮定 A6」参照 | 代表ステータス enum・Session 稼働入力 enum・クロス表・集約の純粋関数群。`behavior.md` の表を単一の真実として実装。 |
| `usecase/agent_session/status.rs` | `SessionStatus` に Step 進行ステータス（とグループキー）を追加。`AgentStatusCenter` に Step/Workflow 代表の算出・差分検出を追加。`aggregate*` の優先順位を新仕様へ更新。 |
| `agent_status_events.rs` | Step/Workflow 代表ステータスの emit を追加（新イベント `workflow-step-status-changed`）。 |
| `adaptor/gateway/workflow/state_notification_gateway.rs` | `WorkflowStateSnapshot` 反映時に、各 Session の Step 進行ステータス・グループキーを `SessionStatus` へ供給。 |
| `usecase/workflow/workspace_tree.rs` | DTO の `status` を 7 種代表ステータス語彙へ統一（`waiting_approval`→`waiting` 等の写像）。`step_status_for_group` の出力写像を `status_aggregation` 経由に揃える。Workflow 操作用に raw run lifecycle 由来の `canStop` を返す。 |
| `adaptor/controller/command/agent_session/status.rs` 他 | 必要に応じ DTO/型公開。 |

### フロント (`src/`)

| ファイル | 変更内容 |
|---|---|
| `types/workspace-tree.ts` | `WorkspaceStepStatus` を 7 種代表ステータスへ更新。 |
| `components/workspace/RepresentativeStatusIcon.tsx`（新規 or `WorkflowStepStatusIcon` 拡張） | 7 種すべてのアイコン・色マッピング。 |
| `components/workspace/WorkspaceList.tsx` | Step 行・Workflow 行の代表ステータス表示を、新イベント購読値で上書き（live 優先・無ければ DTO 値）。 |
| `hooks/useWorktreeStepStatuses.ts`（新規 or 既存フック拡張） | `workflow-step-status-changed` 購読。 |
| `components/ui/agent-state-icon.tsx` | 変更最小（Session 行は raw `AgentState` 継続表示・「仮定 A4」）。 |

---

## 4. アーキテクチャと責務分割

### 4.1 ロジック配置（rust-first-logic）

- **クロス表・優先順位・集約は 100% Rust の純粋関数**（`status_aggregation.rs`）。フロントは一切合成しない。
- フロントは「Rust が算出済みの代表ステータス値を、アイコン・色へ写像して表示する」のみ。

### 4.2 代表ステータス算出の所有とリアルタイム経路（中核設計）

採用方針（**確定 = 案 A**, push イベント配信）:

- `AgentStatusCenter` を **Session→Workspace 集約に加えて Step/Workflow 代表集約も所有**する単一アグリゲータへ拡張する。
- `SessionStatus` に、その Session が属する Step の **進行ステータス**と**グループキー（worktree_path + execution_id + step_name + run_index）** を保持させる。
  - これらは `state_notification_gateway` が `WorkflowStateSnapshot` 反映時に供給（`workflow-state-changed` 系の更新）。
  - Session の純粋な稼働トグル（`session-status-changed`：Streaming↔Idle）では Step 進行は変わらないため、`SessionStatus` に保持済みの値を再利用する。これによりスナップショット非同伴のイベントでも正しく再算出できる。
- `AgentStatusCenter` は、いずれかの入力（`agent_state` または Step 進行）が変化した時に、影響する Step グループと Workflow について代表ステータスを再算出し、差分があれば emit する。
- 配信は新イベント **`workflow-step-status-changed`**（push）。フロントは購読して表示するのみ（新規ポーリングなし → R5 充足）。

この経路により、Session が streaming を開始/停止すると `session-status-changed` の発火と同じパイプライン内で Step/Workflow 代表が再算出・配信され、追加ポーリングなしに WorkspaceList が更新される（`behavior.md` Rule 5）。

### 4.3 静的ツリーとの統合（フォールバック）

- 開いている Session を持たない Step（履歴・完了済みで Session が Closed）は live 算出対象外（Closed は集約から除外＝既存方針）。これらは `list_workspace_worktree_nodes` の DTO `status`（`workspace_tree.rs` が `WorkflowStateSnapshot` から算出、7 種語彙へ写像済み）を表示源とする。
- 開いている Session を持つ Step は、`workflow-step-status-changed` の live 値が DTO 値を上書きする。
- フロントの統合規則: **Step グループキー一致の live 値があれば live 値、無ければ DTO `status`** を表示。両チャネルとも同じ 7 種語彙で揃えるため、フロントは単純な「あれば差し替え」だけで済む（合成ロジックを持たない）。

---

## 5. データモデル / 型

### 5.1 代表ステータス（Rust 新規）

```rust
// domain/workflow/status_aggregation.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepresentativeStatus {
    Running,    // 優先度 1（最強）
    Failed,     // 2
    Error,      // 3
    Waiting,    // 4
    Aborted,    // 5
    Completed,  // 6
    Queued,     // 7（最弱）
}

impl RepresentativeStatus {
    pub fn priority(self) -> u8 { /* 1..=7 */ }
    pub fn as_str(self) -> &'static str { /* "running" 等。serde/DTO と一致 */ }
}
```

- `priority()` 昇順が「強さ」。集約は「最小 priority」を選ぶ。
- 文字列表現はフロント `WorkspaceStepStatus`（7 種）と一致させ、serde で素直にシリアライズ。

### 5.2 Step 進行ステータス（既存の語彙を enum 化）

クロス表の入力に使う Step 進行ステータス（`behavior.md` の `step`）:

```rust
pub enum StepProgress { Failed, WaitingApproval, Running, Aborted, Completed, Queued }
```

既存の文字列定数（`STEP_STATE_*`、`WorkflowExecutionState`）からの写像を一箇所に集約する。

### 5.3 Session 稼働ステータス

既存 `AgentState`（`Running / Waiting / Done / Error`、`usecase/agent_session/status.rs`）を入力元として使い、`AgentStatusCenter` が domain 側の `SessionActivity`（`Running / Waiting / Done / Error`）へ変換してから `status_aggregation` に渡す。新たな稼働状態は追加しない。

### 5.4 `SessionStatus` への追加フィールド（Rust）

```rust
pub struct SessionStatus {
    // 既存: chat_session_id, worktree_id, worktree_path, agent_state,
    //       turn_phase, session_state, pending_permission, last_activity_at,
    //       workflow_step, workflow_execution_state ...
    pub workflow_step_progress: Option<StepProgressRepr>, // 追加: その Session の Step 進行
    pub workflow_run_index: Option<u32>,                  // 追加: parallel グループ識別
    pub workflow_execution_id: Option<String>,           // 追加（既存に無ければ）: グループキー用
}
```

- グループキー = `(worktree_path, workflow_execution_id, workflow_step, workflow_run_index)`。
- これらは表示用フィールドではなく集約用の内部入力。フロントへ露出しても良いが、フロントは利用しない。

### 5.5 配信ペイロード（新イベント）

`workflow-step-status-changed`（Step 単位）:

```jsonc
{
  "worktreePath": "…",
  "executionId": "…",
  "stepName": "…",
  "runIndex": 0,            // parallel 識別、無ければ null
  "representative": "running" // 7 種のいずれか
}
```

Workflow 代表は、Step 代表群から導出してフロントで「あれば差し替え」する設計とし、必要なら同イベントに `workflowRepresentative` を併載 or 別イベント `workflow-status-changed` 拡張で配信する（実装時に粒度確定。テストはどちらでも Rust 純粋関数で担保）。

### 5.6 フロント型

```ts
// types/workspace-tree.ts
export type WorkspaceStepStatus =
  | "running" | "failed" | "error" | "waiting"
  | "aborted" | "completed" | "queued";
```

`AgentState`（`running | done | error | waiting`）は Session 行用に維持（仮定 A4）。Workflow 行の Stop 可否は代表ステータスではなく、DTO の `canStop`（raw `RunStatus::Running | WaitingApproval`）を用いる。

---

## 6. 処理フロー

### 6.1 Session 結果導出（Rule 1, 純粋関数）

```rust
pub fn session_result(step: StepProgress, activity: SessionActivity) -> RepresentativeStatus
```

`behavior.md` の Examples をそのまま分岐:

- `agent == Running` → 常に `Running`（Step 不問）。
- `agent == Waiting`: `Failed→Failed`、それ以外（`WaitingApproval/Running/Aborted/Completed/Queued`）→ `Waiting`。
- `agent == Done`: `Failed→Failed, WaitingApproval→Waiting, Running→Running, Aborted→Aborted, Completed→Completed, Queued→Queued`。
- `agent == Error`: `Failed→Failed`、それ以外 → `Error`。

### 6.2 Step 集約（Rule 2）

```rust
pub fn aggregate(results: impl IntoIterator<Item = RepresentativeStatus>) -> Option<RepresentativeStatus>
```

- 各 `RepresentativeStatus.priority()` が最小（最強）のものを返す。
- 空集合 → `None`（live 算出では emit しない＝DTO フォールバック）。
- 単一 Session の Step はその結果がそのまま代表（Rule 2 の単一ケース）。

### 6.3 Workflow 集約（Rule 3）

Step 代表群に同じ `aggregate` を適用して最強を Workflow 代表とする。

### 6.4 リアルタイム更新パイプライン（Rule 5）

1. Session の `turn_phase`/`session_state` 変化 → `AgentStatusCenter::update_session`（既存）。
2. 同 worktree の Workspace 集約（既存）に加え、当該 Session のグループキーから **Step 代表**を再算出。`SessionStatus.workflow_step_progress`（前回スナップショットで供給済み）と新 `agent_state` を `session_result` に通し、同 Step グループの全 open Session で `aggregate`。
3. さらに当該 Workflow の全 Step 代表で `aggregate` し **Workflow 代表**を更新。
4. 差分があれば `workflow-step-status-changed`（必要なら Workflow 代表も）を emit。
5. `WorkflowStateSnapshot` 変化時（`state_notification_gateway`）は、各 Session の `workflow_step_progress`/グループキーを更新 → 同様に再算出・emit。
6. フロント `useWorktreeStepStatuses` が購読し、`WorkspaceList` の Step/Workflow 行へ live 値を反映。

---

## 7. エラー処理

- **クロス表は全域関数**: `StepProgress × SessionActivity` の全組み合わせを網羅（戻り値必ず確定）。`match` を網羅的にし `_` フォールバックを置かない（仕様逸脱をコンパイル時に検出）。
- **空集約**: open Session の無い Step/Workflow は `aggregate` が `None` → emit せず DTO フォールバック。フロントは live 不在時に DTO 値を使う既定経路で安全。
- **未知文字列の写像**: 既存 `STEP_STATE_*` / `WorkflowExecutionState` から `StepProgress` への写像で未知値が来た場合は保守的に `Queued`（最弱）へ寄せる。ログは既存方針に従い過剰出力しない。
- **Closed/Archived Session**: 既存どおり集約対象外（仮定 A4 / `behavior.md` A4）。
- **イベント取りこぼし**: live 値が来ない Step は DTO フォールバックで表示が破綻しない（縮退して整合）。

---

## 8. テスト方針

### 8.1 Rust 単体（`status_aggregation.rs`）— `behavior.md` の表を直接検証

- `session_result`: `behavior.md` Rule 1 の 4 つの Examples（running/waiting/done/error × 6 Step）= 24 ケースを表駆動テストで全網羅。
- `aggregate`（Step, Rule 2）: Examples の `results → representative`（7 ケース）+ 単一 Session ケース + 空集合 `None`。
- `aggregate`（Workflow, Rule 3）: Examples の `steps → representative`（7 ケース）。
- 優先順位: `running > failed > error > waiting > aborted > completed > queued` を `priority()` 昇順で検証。

### 8.2 Rust 単体（`agent_session/status.rs` / `workspace_tree.rs`）

- `AgentStatusCenter`: Session 稼働トグル（Streaming↔Idle）で Step/Workflow 代表が再算出・差分 emit されること。Step 進行非同伴イベントで保持済み `workflow_step_progress` を再利用すること。
- Parallel Step: 1 つでも `running` の Session があれば代表 `running`（Rule 2 シナリオ）。
- `workspace_tree.rs`: DTO `status` が 7 種語彙へ写像されること（`waiting_approval`→`waiting` 等）。既存テスト `workflow_step_status_uses_ref_state_priority_*` を新優先順位・新語彙へ更新。

### 8.3 フロント（Vitest）

- `RepresentativeStatusIcon`: 7 種それぞれのアイコン・色マッピング、`running` が「動作中」として停止系と視覚的に区別される（Rule 4）。
- `WorkspaceList` 統合表示: live 値が DTO 値を上書きすること、live 不在時に DTO 値が出ること（Tauri `listen` をモック）。
- `useWorktreeStepStatuses`: `workflow-step-status-changed` 購読での Map 更新と worktree フィルタ。

### 8.4 受け入れ確認（requirements 受け入れ基準）

- `cargo clippy -- -D warnings` / `cargo test` / `pnpm lint` / `pnpm test` 通過。
- `behavior.md` Rule 1 の各セルが実装と一致（8.1 で機械的に担保）。

---

## 9. リスクと代替案

### リスク

- **R-1 結合増加**: `AgentStatusCenter`（`usecase/agent_session`）が Step 進行という Workflow 由来の情報を保持する。依存方向は「gateway が snapshot から `SessionStatus` へ値を流し込む」形にし、status center 側は domain の純粋関数にのみ依存する（純粋関数 `status_aggregation` は `SessionActivity` + `StepProgress` enum のみを入力に取る）。
- **R-2 parallel run_index の整合**: グループキーの `run_index` 取り違えで別グループの Session を混在集約する恐れ。`collect_step_session_refs`（既存）と同じグループ定義（`group_step_name` + `group_run_index`）を `SessionStatus` 供給時に厳密適用する。
- **R-3 履歴 Step と live のフォールバック境界**: open Session の有無で表示源が切り替わる。切替時の一瞬の不整合を避けるため、フロントは「live があれば live、無ければ DTO」を単純規則で適用し、両者を同語彙に揃える。
- **R-4 アイコン語彙の移行**: 現状 `WorkflowStepStatusIcon`(error 無し) と `AgentStateIcon`(failed/aborted/queued 無し) が分離。7 種統一アイコンを新設し Step/Workflow 行で使用、Session 行は raw `AgentState` 用に `AgentStateIcon` 継続。

### 代替案

- **代替-B（pull 型コマンド・不採用）**: クロス表・集約を Tauri コマンドとして公開し、フロントが `session-status-changed` 毎に invoke して再計算。ロジックは Rust に残るが、フロントが「いつ何を invoke するか」を統制する点で表示専従から逸脱気味。新規ポーリングではないため R5 は満たすが、push（案 A）の方が責務分割が明快なため不採用。
- **代替-C（ツリー再フェッチ）**: `session-status-changed` で `list_workspace_worktree_nodes` を毎回再フェッチして DTO 側で集約。実装は最小だが、稼働トグルのたびに全ツリー再構築＝既存の最適化（status 更新では refetch しない）を逆行させ、重い。不採用。

---

## 10. 仮定

- (A1) Session 稼働判定は既存 `AgentState` を使用（`running = Streaming` / `waiting = 承認・入力・許可待ち` / `done = ターン完了` / `error = 実行時エラー`）。新稼働状態は追加しない。
- (A2) Spec ディレクトリは `docs/specs/issues-1259`。
- (A3) #1242 で確定した WorkspaceList のナビゲーション構造（Session/Step を選択単位・Parallel は 1 Step）は変更しない。
- (A4) **Session 行は raw `AgentState`（4 種、running=streaming を pulse 強調）を継続表示**し、「中の Session が動いているか」を直接示す。クロス表の「1 Session の結果ステータス（7 種）」は Step/Workflow 集約の内部入力として用い、Session 行表示には用いない。Step・Workflow 行は 7 種代表ステータスを表示。
- (A5) Closed/Archived Session は live 集約対象外（既存除外条件を踏襲）。
- (A6) `status_aggregation` の配置は `domain/workflow/` 配下とする。純粋関数は domain の `SessionActivity` と `StepProgress` enum のみに依存し、`AgentState` から `SessionActivity` への型変換は呼び出し側（status center）が担う。これにより `agent_session` と `workflow` の usecase 間依存を作らない。
- (A7) リアルタイム配信は push 型新イベント `workflow-step-status-changed` を採用（案 A 確定）。`AgentStatusCenter` を拡張し、Step 進行を `SessionStatus` に保持、稼働変化時に Step/Workflow 代表を再算出して push 配信する。フロントは表示専従。

---

## 11. Open Questions

なし。
