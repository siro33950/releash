# issues-1011: Workflow Engine Evolution - Run Store / Run ID

関連: [GitHub Issue #1011](https://github.com/siro33950/releash/issues/1011) / マイルストーン [03] / [`workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md) / [`workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md)

## 要求

**種別**: 新機能

**ゴール**: workflow 実行を `run_id` を主語として参照できる状態を作る。具体的には以下が満たされること。

- workflow 1 回分の実行を `WorkflowRun`（および `WorkflowRunSummary`）として識別・記録できる。
- active な run と非 active な run を metadata / logs から一覧できる。
  - **用語定義**: 「非 active」= 終了済み（terminal）で、`completed` / `failed` / `aborted` の 3 つの terminal status を包含する。`completed` status は terminal の 1 種（成功終了）であり、非 active 全体の総称ではない。本ドキュメント中の「completed run」表現は文脈上、特に区別しない限り「非 active = 終了済み run」を指す。
- `worktree_path -> active run_id` の active lookup と、`run_id -> worktree_path` の reverse lookup が成立する。
- 既存 `WorkflowState.execution_id` を `run_id` として扱える（新規 ID 採番ではなく既存 ID を昇格する形）。
- run metadata を永続化する保存先（候補: `workflow_runs/{run_id}.json`）を持つ。
- 既存の worktree-scoped state / Tauri command 入口・出口形に対する破壊的変更を許容する（worktree 主語の API は run_id 主語へ完全に置換してよく、後方互換 wrapper は要求しない）。

**背景**: 現状の workflow 実行は主に chat session と worktree-scoped state として表現されており、step session が通常の自由対話 session と同列の tab として見えやすい。今後 CLI / Skill / structured output / 承認操作を足していくと、UI 上の主語と engine 上の主語のずれが拡大する。マイルストーン [04] 以降で導入する `WorkflowCommand` / `WorkflowEvent` / mutating CLI / Workflow Panel / OutputForm CLI / Main Agent Mediation はいずれも「`run_id` を主語にできる」ことを前提とするため、その土台として Run Store を先に確立する必要がある。

**スコープ**: マイルストーン [03] に限定する。`WorkflowCommand` typed 入口の導入（[04]）、read-only run APIs / CLI（[05]）、mutating CLI（[06]）、Workflow Panel UI（[07]）等は本 issue のスコープ外。

**前提**: マイルストーン [02] Normalized Workflow が完了済み（旧 `Step` / 旧 YAML / 旧 JSON 互換は破棄済み、新 `Workflow` / `NodeDefinition` schema が一次入口）。

## 振る舞い定義

```gherkin
Feature: Workflow Run の Run ID 主語化
  workflow の実行を、固有の識別子（run_id）を主語として識別・参照・一覧できる状態を提供する。

  Rule: workflow を 1 回起動するたびに、その実行は固有の識別子で記録される
    Scenario: 新しい workflow 実行が記録される
      Given 利用者が workflow を起動できる状態にある
      When 利用者が workflow を起動する
      Then その実行は固有の識別子を持つ実行インスタンスとして記録される
      And その実行の概要（どの workflow を／どの作業対象で／いつ起動したか）が後から参照できる形で残る

  Rule: 既に進行している worktree 上の実行は、新たな識別子を採番せずそのまま実行インスタンスとして扱われる
    Scenario: 進行中の worktree-scoped 実行が実行インスタンスとして参照できる
      Given 既存の workflow 実行が worktree 上で進行している
      When その実行を実行インスタンスとして参照する
      Then 既存実行が保持する識別子がそのまま実行インスタンスの識別子として扱われる
      And 別の識別子が追加で採番されることはない

  Rule: 進行中の実行と終了した実行を区別して一覧できる
    Scenario: 進行中の実行を一覧する
      Given 1 つ以上の workflow 実行が進行している
      When 利用者が進行中の実行の一覧を要求する
      Then 進行中の各実行が、その識別子・対象 workflow・作業対象とともに列挙される

    Scenario: 終了した実行を一覧する
      Given 過去に完了・失敗・中断（completed / failed / aborted）した workflow 実行が存在する
      When 利用者が終了した実行の一覧を要求する
      Then 終了した各実行（completed / failed / aborted を含む）が、その識別子・対象 workflow・結果状態とともに列挙される

  Rule: worktree と実行インスタンスは双方向に解決できる
    Scenario: worktree から進行中の実行を解決する
      Given ある worktree で workflow 実行が進行している
      When その worktree を起点に進行中の実行を問い合わせる
      Then その worktree に紐づく実行インスタンスの識別子が返る

    Scenario: 実行インスタンスから worktree を解決する
      Given workflow 実行インスタンスの識別子が分かっている
      When その識別子から作業対象を問い合わせる
      Then その実行が紐づいている worktree が返る

  Rule: 同一 worktree に進行中の実行が存在する間は、新たな workflow 起動は拒否される
    Scenario: 進行中の実行がある worktree への重複起動が拒否される
      Given ある worktree で workflow 実行が進行している
      When 同じ worktree に対して新たな workflow を起動しようとする
      Then その起動は拒否され、新しい実行インスタンスは作成されない
      And 既存の進行中実行はそのまま継続する

  Rule: workflow の起動・一覧・参照・操作は、同一デバイス所有者または認証済みリモートセッションから行われる前提とする
    Scenario: 既存の認証境界内からの操作が受理される
      Given 操作主体が同一デバイス所有者（デスクトップ UI）または認証済みリモートセッションである
      When その主体が workflow を起動する、または実行インスタンスを一覧・参照・操作する
      Then その操作は受理される

  Rule: 永続化された実行 metadata の一部が破損していても、実行インスタンスの一覧は継続して提供される
    Scenario: 破損した metadata エントリが含まれる状態で一覧を要求する
      Given 永続化された実行 metadata の中に破損または形式不正なエントリが含まれている
      When 利用者が終了した実行の一覧を要求する
      Then 破損したエントリは一覧から除外される
      And 有効な実行 metadata の一覧は通常通り返される
```

## アーキテクチャ概要

**起動済みの workflow 実行を参照・操作する主語は `run_id` のみとする**。起動 command だけは例外で、`run_id` を生成して返す経路となる（入力は workflow / 実行コンテキストの指定であり、既存実行を参照する主語ではない）。それ以外の API・engine 内部キー・persistence のすべてのレイヤーで `run_id` を一次キーにする。`worktree_path` は `WorkflowRun` の属性および双方向 lookup の片側として保持されるが、状態キー・参照系 API の主語にはならない。

### 責務配置

- `src-tauri/src/workflow/run.rs`（新設）: `WorkflowRun` / `WorkflowRunSummary` の型定義、run metadata の永続化（`workflow_runs/{run_id}.json`）、active/reverse lookup を担う Run Store。担当しない: 状態遷移ロジック、node 実行、UI 整形。
- `src-tauri/src/workflow/engine.rs`（既存・改修）: in-memory な実行表を `run_id` キーに置換し、run 開始通知・終了通知を Run Store に送る。`worktree_path` は WorkflowExecution の属性として保持するが、状態キーには使わない。担当しない: run metadata の永続化、ファイル I/O。
- `src-tauri/src/workflow/state.rs`（既存）: `WorkflowState.execution_id` を `run_id` の供給源として扱う（新規 ID 採番は行わない）。担当しない: run metadata の persistence shape の定義。
- `src-tauri/src/workflow/log.rs`（既存）: NDJSON event log は run 終了時の completed run 列挙の補助情報源として利用する。担当しない: run metadata の一次 source-of-truth。
- `src-tauri/src/workflow/commands.rs`（既存・改修）: Tauri command の入口・出口を `run_id` 主語に**完全置換する**。`worktree_path` 主語の API は保持しない。担当しない: lookup や persistence の詳細。
- フロントエンド（`src/`）: 受け取った `WorkflowRunSummary` の表示と、`run_id` を引数とした invoke のみ。run の管理ロジックは持たない（[`rust-first-logic`](../../.claude/rules/rust-first-logic.md) に従う）。

### データ/通信フロー

- **run 開始**: UI → 起動 Tauri command（入力: `workflow_name` + `task` + `worktree_path` + `trigger_source`、出力: `run_id`） → `engine` が WorkflowExecution を生成（`execution_id` を `run_id` として採用） → Run Store に「active run 登録 + metadata 初期保存」を依頼 → `workflow_runs/{run_id}.json` 作成 → 呼び出し側に `run_id` を返す。
- **run 終了（completed / failed / aborted）**: `engine` が終了状態に遷移 → Run Store に「active から除外 + metadata 更新（`completed_at` / `status` 等）」を依頼 → `workflow_runs/{run_id}.json` 更新。
- **active run 一覧**: Tauri command → Run Store の active set を走査 → 各 run の `WorkflowRunSummary` を返す。
- **completed run 一覧**: Tauri command → Run Store の persisted metadata 一覧（`workflow_runs/` 配下）を走査 → status で active を除外 → `WorkflowRunSummary` を返す。
- **worktree → run 解決（補助 lookup）**: 呼び出し側 → Run Store の `worktree_path -> active run_id` index を引く。API の主語ではなく、active な run が存在するか／その run_id は何かを問い合わせるための補助手段。
- **run → worktree 解決（補助 lookup）**: 呼び出し側 → Run Store が `WorkflowRun.worktree_path` を返す。run_id を主語とした属性照会。

### 状態 Owner

- **run_id をキーとする active run 集合**: Run Store（`workflow/run.rs`）が一次 owner。in-memory map として保持し、engine からの開始/終了通知で更新する。
- **`worktree_path -> active run_id` の secondary index**: Run Store。active set からの派生 index として保持する。
- **run metadata の永続化（`workflow_runs/{run_id}.json`）**: Run Store。
- **進行中の `WorkflowState`（run の中身）**: `engine.executions`（既存）。Run Store は WorkflowState の中身を所有しない。**ただしキーは `run_id` に統一する**。
- **WorkflowState 内の `execution_id` の値**: `engine` / `state.rs`（既存）。Run Store はこれを参照するのみで採番しない。
- **NDJSON event log**: `log.rs`（既存）。Run Store は metadata の補完情報として読みうるが、event log の所有はしない。

### 境界

- **既存実行を参照・操作する API の主語は `run_id` のみ**: 起動以外の Tauri command（abort / approve / reject / submit / 状態取得 / 一覧 / log 取得 等）は `run_id` を主語に取る。`worktree_path` 主語の参照 API は保持しない。
- **起動 command は run_id を払い出す唯一の入口**: 起動 command の入力は `workflow_name` + `task` + `worktree_path` + `trigger_source`、出力が `run_id`。`run_id` の生成主体は engine の `WorkflowExecution.execution_id` 採番ロジックであり、起動 command 自身は採番せず、engine が生成した `execution_id` を `run_id` として呼び出し側に払い出す。Run Store も採番しない（line 104 と整合）。`worktree_path` はここで起動先 worktree の指定として渡されるが、これは「実行コンテキストの指定」であって既存実行を参照する主語ではない。
- **engine 内部キーも `run_id`**: `engine.executions` の HashMap キーは `run_id` に置換する。worktree_path はキーから外し、WorkflowExecution / WorkflowRun の属性として持つ。
- **Run Store は状態遷移を所有しない**: run の status 更新は engine からの通知を受けて metadata に反映するだけで、独自に遷移を起こさない。状態遷移の権威は引き続き `engine`。
- **Run Store は run_id を採番しない**: 既存の `WorkflowState.execution_id` を `run_id` として「昇格」させる。run_id の生成主体は engine（line 102 参照）であり、Run Store は engine から払い出された値を受け取って管理するのみで、独自に新規 ID を採番することはない。
- **persistence と in-memory の責務分離**: active な情報は in-memory lookup から引け、completed 情報は file system から取得する。Run Store の中でこの境界を内包する。
- **フロント側に run 管理ロジックを置かない**: lookup / 列挙 / persist はすべて Rust 側で完結し、フロントは invoke 経由で結果を受け取る。
- **操作主体の前提**: active / 非 active run の一覧、worktree → run、run → worktree の参照、および起動以外の run 操作 API は、同一デバイス所有者（デスクトップ UI）または認証済みリモートセッションが呼び出す前提とする。workflow 起動 command も同様に、同一デバイス所有者（デスクトップ UI）または認証済みリモートセッションが呼び出す前提とし、未認証経路からの起動は想定しない。本 issue では新たな認可モデルを導入しない。
- **run metadata 露出範囲**: `workflow_name` / `task` / `worktree_path` / `trigger_source` は既存の認証境界（デスクトップ + 認証済みリモートセッション）内で露出してよい。`task` はユーザー入力としてそのまま保持・表示する（追加のマスキングや変換はしない）。
- **Run Store の信頼境界**: UI（デスクトップ + 認証済みリモートセッション）からの入力は Tauri command の型検査を通過した後は信頼してよい。一方、`workflow_runs/` 配下の metadata JSON および NDJSON event log は外部改変の可能性がある外部入力として扱い、読込時に形式検証を行う。欠損・破損したエントリは warn ログ出力のうえスキップし、Run Store 全体の動作を停止させない。

### 実装に委ねること

- `WorkflowRun` / `WorkflowRunSummary` のフィールド名と細目（必須/任意、`Option` の分布、serde 表現）。[`workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md) の暫定フィールドリストを起点に判断する。
- Run Store の in-memory データ構造（`HashMap<run_id, _>` を一次に、`HashMap<worktree_path, run_id>` を secondary index として持つ等）。
- `workflow_runs/` 配下のディレクトリ作成タイミング、I/O 失敗時のリトライ・ロギング戦略。
- `engine` から Run Store への通知経路の具体的シグネチャ（直接メソッド呼び出しか、内部 event か）。
- completed run 一覧時のソート順、件数制限、欠損 metadata の取り扱い（warn してスキップ等）。
- 既存の `WorkflowExecution` から `WorkflowRun` metadata を組み立てる helper の所在（`run.rs` 内 vs `engine.rs` 内）。
- `engine.executions` のキー置換に伴う既存呼び出し元の追従手段（既存の worktree_path 引数を受ける内部関数を `run_id` 解決経由に書き換える、等）。
- テストの配置（`workflow/run.rs` 内 `#[cfg(test)]` モジュールでの単体テスト、`engine` 連携の統合テスト）と具体的なテストケース。

