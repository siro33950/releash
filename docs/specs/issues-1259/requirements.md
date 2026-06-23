# Requirements

## Type

WorkspaceList のステータス表示・集約ロジックの再設計。

関連: #1259 / #1242 / #1220

## 背景と目的

Issue #1259 の要求:

> Step の状態も見たいが、それ以上に中の Session が動いてるのか止まっているのかも見たい。
> 優先順位を設定し適切なステータスになるようにする。

現状、WorkspaceList では次の 2 系統のステータスが別々に表示されている。

- **Step ステータス** (`WorkspaceStepStatus`): `queued / running / waiting_approval / completed / failed / aborted`
  - `WorkflowStepStatusIcon` でアイコン + 色表示。
  - Parallel Step では複数 Session を `failed > aborted > waiting_approval > running > completed > queued` の優先順位で 1 つに集約 (`step_status_for_group`)。
- **Session ステータス** (`AgentState`): `running / done / error / waiting`
  - `AgentStateIcon` で Bot アイコン + 色表示。
  - `running` = モデル応答の streaming 中、`waiting` = 承認待ち / idle、`done` = 完了、`error` = エラー。

現状の課題:

- Step の集約ステータスを見ても、その Step 配下の Session が **実際に処理を進めている (動いている)** のか、**入力待ち / 承認待ちで停止している (止まっている)** のかが直感的に分からない。`running` という Step ステータスが、streaming 中なのか単にアクティブなだけなのかを区別しない。
- 集約の優先順位が「異常・完了状態」中心 (`failed > aborted > waiting_approval > running > completed`) であり、「今この Workspace に人の操作が必要か / 自動で進んでいるか」という観点でユーザーが知りたい代表ステータスと一致しないことがある。

目的: WorkspaceList 上で、Step ステータスに加えて **配下の Session が動作中か停止中か** を一目で判別できるようにし、複数 Session/Step を集約する際の優先順位を「ユーザーが次に注目すべき状態」が代表ステータスになるよう再設計する。

## スコープ

- WorkspaceList における Step 行・Session 行・Workflow 行のステータス表示の再設計。
- 「Session が動いている (streaming) / 止まっている (idle・承認待ち・完了・エラー)」を判別できる表示の追加。
- 複数 Session を含む Step、複数 Step を含む Workflow のステータス集約優先順位の再定義。
- 上記に必要な Rust 側のステータス導出・集約ロジック (`workspace_tree.rs` / `agent_session/status.rs`) の変更。

## 非スコープ

- Workflow engine の実行モデル・実行順序の変更。
- Workflow 定義や YAML schema の変更。
- 中央パネル (Chat UI / Step 画面) の表示内容の再設計。
- WorkspaceList のナビゲーション構造 (Session/Step の選択単位) 自体の変更 (#1242 で確定済みの構造を維持)。
- Source Control / Editor / Terminal など Workflow 以外の画面。

## 要求事項

### R1. Session の稼働状態の可視化

- WorkspaceList 上で、各 Session について **動作中 (streaming)** か **停止中** かを判別できること。
- 「動作中」「停止中」の判定は Rust 側で導出し、フロントは表示に徹すること (rust-first-logic)。

### R2. Step の稼働状態の反映

- Step 行のステータス表示で、配下に動作中の Session があるか否かを判別できること。
- Parallel Step の場合も、配下のいずれかの Session が動作中であることが分かること。

### R3. 集約優先順位の再定義

- 複数 Session を含む Step、複数 Step を含む Workflow の代表ステータスを決める優先順位を、下記「確定ステータス仕様」のとおりに定義すること。
- 優先順位は Rust 側の単一ロジックで一貫して適用され、フロントの表示と乖離しないこと。

### R4. 既存ステータス種別との整合

- 代表ステータスは下記の 7 種 (`running / failed / error / waiting / aborted / completed / queued`) に統一する。`waiting` は従来の Step `waiting_approval` と Session の入力/許可待ち (`AgentState == waiting`) を 1 つに統合したものとする。
- Session 行の `AgentStateIcon` は既存どおり raw `AgentState` を表示し、7 種の代表ステータス集約の対象外とすること。
- Step / Workflow 行の `WorkflowStepStatusIcon` の表示は、この 7 種の代表ステータスに整合させること。

### R5. リアルタイム更新

- Session の稼働状態は `session-status-changed` イベント購読 (`useWorktreeSessionStatuses`) によりリアルタイムに更新されること。
- Step / Workflow の代表ステータスは `workflow-step-status-changed` イベント購読 (`useWorktreeStepStatuses`) によりリアルタイムに更新されること。
- 新たなポーリング機構を追加しないこと。

## 確定ステータス仕様

### 1. 状態一覧（強い順 = 優先度）

| 優先 | 状態 | 意味 |
|---|---|---|
| 1 | `running` | Session が動作中 (streaming) |
| 2 | `failed` | 失敗 |
| 3 | `error` | 実行時エラー |
| 4 | `waiting` | 承認待ち・入力待ち・許可待ち |
| 5 | `aborted` | 中止 |
| 6 | `completed` | 完了 |
| 7 | `queued` | 未着手・順番待ち |

### 2. Session 1 個の結果（Step 進行 × Session 稼働）

縦 = Step 進行ステータス、横 = その Session の稼働ステータス (`AgentState`)。セル = その Session の結果ステータス。

| Step ↓ \ Session → | `running` | `waiting` | `done` | `error` |
|---|---|---|---|---|
| `failed` | running | failed | failed | failed |
| `waiting_approval` | running | waiting | waiting | error |
| `running` | running | waiting | running | error |
| `aborted` | running | waiting | aborted | error |
| `completed` | running | waiting | completed | error |
| `queued` | running | waiting | queued | error |

- Session 稼働が `running` の場合、Step 進行が何であれ結果は常に `running`。

### 3. 複数 Session（Parallel Step）の集約

各 Session を上記「2.」の表に通して結果を求め、その中で**優先度が一番強い（1 に近い）もの**を Step の代表ステータスとする。

### 4. Workflow の集約

配下の全 Step の代表ステータスを集め、同じく**優先度が一番強いもの**を Workflow の代表ステータスとする。

## 受け入れ基準の概要

- WorkspaceList で、`running` の Session を含む Step と、全 Session が停止系状態の Step を視覚的に区別できる。
- Parallel Step 配下に 1 つでも `running` の Session があれば、その Step の代表は `running` になる。
- 集約優先順位 (`running > failed > error > waiting > aborted > completed > queued`) が Rust 側テスト (`workspace_tree.rs` / `status.rs` の単体テスト) で検証され、確定仕様どおりに代表ステータスが決まる。
- 「確定ステータス仕様 2.」の Step × Session の各セルの結果が、実装と一致する。
- `cargo clippy -- -D warnings` / `cargo test` / `pnpm lint` / `pnpm test` が通る。

## 仮定

- (A1) Session 稼働の判定は既存の `AgentState` を用いる。`running` = `turn_phase == Streaming`、`waiting` = 承認/入力/許可待ち、`done` = ターン完了、`error` = 実行時エラー。
- (A2) Spec ディレクトリ名は既存慣習に合わせ `docs/specs/issues-1259` とする。
- (A3) #1242 で確定した WorkspaceList のナビゲーション構造 (Session/Step を選択単位とし、Parallel は 1 Step として扱う) は変更しない。本 Issue はその上のステータス表示・集約のみを対象とする。

## Open Questions

なし。
