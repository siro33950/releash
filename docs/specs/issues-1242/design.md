# Design

本書は #1242「Workflow UI 再整理」の実装設計を定義する。要求は `requirements.md`、外部から観測される振る舞いは `behavior.md` を参照する。

## 概要

Workflow UI の主語を `Session` / `Step` / `Workflow` に整理する。

- `Session`: 中央パネルで既存 Chat UI として表示する作業単位。
- `Step`: Workflow run 内の実行単位。中央パネルでは Step 画面として表示する。
- `Workflow`: WorkspaceList 上で Workflow 全体の状態と Step 一覧をまとめる group。

中央パネルは `Session` または `Step` のみを表示する。`Workflow` は中央表示対象ではなく、WorkspaceList 上の group と状態確認対象として扱う。

`Parallel` は独立した UI 主語にしない。複数 `Session` を含む 1 つの `Step` として扱い、中央パネルでは同一 Step 画面内の grid に複数 Pane を並べる。

## 変更対象

### WorkspaceList

WorkspaceList は `Session` / `Step` / `Workflow` の navigation と status overview を担う。

| 行 | 役割 | 中央パネルへの影響 |
|---|---|---|
| `Session` row | 通常 Session の状態表示と選択 | 該当 Session の Chat UI を表示 |
| `Step` row | Workflow Step の状態表示と選択 | 該当 Step 画面を表示 |
| `Workflow` row | Workflow 全体の状態表示、Step 一覧の group、Workflow 操作 | 中央パネルの表示対象は変更しない |

Workflow row は以下を持つ。

- Workflow status overview。
- Workflow 名の右側に置く展開/折りたたみ矢印。
- `Stop` に到達する menu。
- archive 用の `x` button。

Workflow row menu は Worktree row menu と同じ位置決め・表示方式を使う。menu open 中も trigger の位置が安定し、hover が外れても menu が画面左上へ飛ばない。

Step row は既存の Step status icon を使う。raw status 文字列、進捗文字、独自 dot、status icon の背景丸は追加しない。

### 中央パネル

中央パネルは選択された `Session` または `Step` だけを表示する。

- `Session` 選択時: 既存 Chat UI を表示する。
- `Step` 選択時: Step header と Step grid を表示する。
- `Step` 切り替え時: Step 画面全体を切り替える。
- `Workflow` row クリック時: 中央パネルは変化しない。

通常 `Session` 表示時は既存の中央ヘッダー構造を維持する。Workflow `Step` 表示時は中央パネルヘッダーに Step 情報と Step action を置く。

### Step grid

Step grid は Step 内の `Session` を均等な `Pane` として並べる。

```text
Central Step Header
------------------------------------------------
Pane A | Pane B
```

grid のルール:

- iTerm の grid に近い動作にする。
- Pane は UI 内に等分配置する。
- Pane は最小の縦横サイズを持つ。
- Pane は最小サイズより小さくならない。
- 最小サイズを維持できない場合は縦方向 overflow で扱う。
- 横スクロールは発生させない。
- Pane 自体は内容に引きずられて無制限に伸びない。
- Session 本文は Pane 内でスクロールする。
- Pane 内には既存 Chat UI を表示し、Chat 入力欄も既存 Chat UI と同じように表示する。
- 選択中 Pane は枠線のみで示し、専用背景色は入れない。

Step grid には tab bar、grid 内の独自 Step header、resize handle、Step 内 Session close UI を置かない。

### 中央パネルヘッダー

Workflow `Step` 表示時の中央パネルヘッダーには以下を置く。

- Step status icon。
- Step name。
- Step type。
- `Approve` action。
- `Reject` action。
- Approval action error icon。

Step 情報と Step action は下部 footer に置かない。Step action 用 footer は表示しない。

中央パネルヘッダーの左右 padding は Step grid の外側余白と一致させる。これにより、ヘッダー内の Step 情報と grid 内 Pane の開始・終了位置を揃える。

### 右パネルヘッダー

`BaseBranchUI` は右パネルヘッダーに置く。

`BaseBranchUI` は現在 branch から base branch への関係を表示し、base branch の変更操作を提供する。中央パネルヘッダーには置かない。右パネルの既存 toggle 操作は維持する。

### WorkflowTrace

WorkflowTrace は Workflow の流れと状態を読むための表示として残す。WorkspaceList と同じ navigation や表示/非表示操作は持たない。

WorkflowTrace から削除するもの:

- `Hide`。
- `Viewing`。
- Step session の表示/非表示 toggle。
- Step detail パネル。
- Event log パネル。

削除するのは操作 UI と詳細パネルであり、Workflow の流れや Step 状態を読むための情報は維持する。

## アーキテクチャと責務分割

### Rust

Rust は Workflow / Step / Session の実データと状態を持つ。フロントは Rust が返す DTO を描画し、操作時は既存または追加される Tauri command を呼ぶ。

UI 検証中に仮データを使う場合も、仮実装は Rust 通信境界の stub に限定する。表示コンポーネント側に Workflow / Step / Session の親子関係推測、状態推測、sort などのロジックを持たせない。

### Parentage Model

Workflow UI の構造上の親子関係は次の形で扱う。

```text
WorkflowRun
  StepRun(stepName, runIndex)
    Session
```

`WorkflowRun` は実行インスタンス、`StepRun` はその run 内の 1 Step 実行、`Session` は Step 実行に紐づく Chat Session である。同一 Step 名が retry / loop 等で複数回出る場合は `runIndex` で別の `StepRun` として扱う。

`Parallel` は UI 上の独立親子階層を作らない。parallel child の `Session` は親 `Parallel Step` の `StepRun` 配下にまとめる。必要に応じて child 自身の `stepName` / `runIndex` は Session row の表示名や詳細情報として残すが、WorkspaceList の主ナビゲーション単位は親 `Parallel Step` である。

親子関係は Session lifecycle や Workflow execution status とは別の軸である。Step が completed / failed / aborted になっても親子関係は消えない。Session が Closed になっても親子関係は消えない。Archived / Deleted は Session の表示場所または存在に影響するが、Step の実行状態とは独立して扱う。

### Current Parentage Implementation

現状の実装は、上記の親子関係を Session 自体に明示保存していない。`ChatSession` / `SessionSummary` には `workflow_step_session: bool` はあるが、`runId` / `stepName` / `runIndex` / parent Step 情報はない。

そのため WorkspaceList は、次の派生情報を突き合わせて親子関係を復元している。

- `SessionStore` から取得した workflow step session 一覧。
- `WorkflowRunSummary` の run 一覧。
- `WorkflowStateSnapshot.current_session_id`。
- `WorkflowStateSnapshot.step_history[*].session_id`。
- `WorkflowStateSnapshot.step_history[*].child_outputs[*].session_id`。
- `WorkflowStateSnapshot.active_parallel_steps[*].session_id`。

この復元は現状の互換実装であり、親子関係の正本ではない。`step_history` は本来 Workflow 実行記録であって、Session 所属の正本ではないため、長期設計では `Session` 側または専用 read model に `WorkflowStepContext` 相当の親子関係を保存する。

本実装では、Step Session の所属を `ChatSession.workflowStepContext` に保存する。
これは WorkspaceList / Step detail 向けの read model であり、Workflow 実行状態
（`WorkflowStateSnapshot.step_history/current_session_id/active_parallel_steps`）とは別の正本として扱う。

`workflow_step_session` は legacy 識別フラグとして残すが、親子関係の正本にはしない。
新規 Workflow Step Session は作成時に必ず `workflowStepContext` を持つ。
既存 Session で `workflowStepContext` が無いものだけ、互換目的で `WorkflowStateSnapshot` の
session id 参照から最小限の fallback を行う。

保存する context は少なくとも次を表現する。

```ts
type WorkflowStepContext = {
	runId: string;
	workflowName: string;
	stepName: string;
	runIndex: number;
	parentStepName?: string;
	parentRunIndex?: number;
	order: number;
};
```

`parentStepName` / `parentRunIndex` は parallel child を親 `Parallel Step` に group するために使う。通常 Step では `parentStepName` は不要で、`stepName` 自身が group になる。

WorkspaceList / Step detail の投影方針は、Session の親子関係を `WorkflowStepContext` から読み、WorkflowState は Step status / Step type / approval 可否 / sessionless Step の互換表示に限定して使うことである。

Projection の優先順位:

1. `workflowStepContext.runId` が対象 Workflow run に一致する Session を、その context の所属として投影する。
2. `parentStepName` がある Session は parent StepRun に group し、Session row の表示名には child `stepName` を使う。
3. `workflowStepContext` がない既存 Step Session だけ、`workflow_step_session = true` と `WorkflowStateSnapshot` 内の session id 参照を突き合わせる legacy fallback を使う。
4. Session を持たない bash Step / current Step は、互換表示として `WorkflowStateSnapshot` から Step row だけを作ってよい。
5. `workflowStepContext` を持つ Session の親子関係を `WorkflowStateSnapshot` の step 名で上書きしない。

### Frontend

Frontend は次だけを担当する。

- Rust DTO の表示。
- クリック、hover、menu、popover などの入力受付。
- `CenterSelection` に基づく中央表示の切り替え。
- 表示用の薄い formatting。

Frontend は Workflow engine の実行モデル、Step の状態遷移、Session と Step の所属解決を決めない。

### MainLayout / App

中央表示は `CenterSelection` から導出する。`centerMode` のような独立した Agent / Workflow mode state は持たない。

```ts
type CenterSelection =
	| { kind: "agentSession"; worktreePath: string; sessionId: string }
	| { kind: "newAgentSession"; worktreePath: string }
	| {
			kind: "workflowStep";
			worktreePath: string;
			runId: string;
			stepId: string;
			stepName: string;
	  };
```

render derivation:

```text
agentSession / newAgentSession -> AgentChatPanel
workflowStep                   -> StepView
```

Workflow row は selection target にしない。Workflow row click は Step 一覧の展開/折りたたみだけを行い、中央パネルの表示対象を変えない。

### StepView

StepView は Step 表示時に中央パネルの Step 画面を描画する。設計上の主語は `Step` であり、既存 component 名や Workflow 全体表示の都合には依存しない。

- Step header を中央パネルヘッダーとして表示する。
- Step 内 Session を grid Pane として表示する。
- 各 Pane には既存 Chat UI を表示する。
- Step action の pending / error / reject comment popup など、表示中 Step の UI 状態だけを持つ。
- Step status の変化だけを理由に Step 画面を閉じたり非表示にしたりしない。

### WorkspaceList

WorkspaceList は Workflow row / Step row / Session row を描画し、selection request を発行する。

- Session row click: `agentSession` selection。
- Step row click: `workflowStep` selection。
- Workflow row click: expand / collapse only。
- Workflow row menu: Workflow 操作。`Stop` を置く。
- Workflow row `x`: archive 操作。

Step session の表示/非表示 state は WorkspaceList にも WorkflowTrace にも置かない。Step の表示可否を自動で切り替える仕組み自体を削除する。

## データモデルまたは型

新しい Workflow 実行モデルは追加しない。UI が扱う主な型は既存の Workspace tree / Workflow state の read model で表現する。

ただし現状の `workflow_step_session: bool` だけでは `WorkflowRun -> StepRun -> Session` の親子関係を表現できない。#1242 の UI 要求を満たす過渡実装では `WorkflowStateSnapshot` から親子関係を復元しているが、これは互換投影であり、永続モデルとしては `WorkflowStepContext` 相当の明示的な所属情報へ移行する。

Step status は既存の状態を使う。

```ts
type WorkspaceStepStatus =
	| "queued"
	| "running"
	| "waiting_approval"
	| "completed"
	| "failed"
	| "aborted";
```

Step status の意味や状態遷移は変更しない。表示は既存の Step status icon を使い、見た目だけを #1242 の UI 方針に合わせる。

`Parallel` は型や UI 上の主 navigation target にはしない。複数 Session を持つ `WorkspaceWorkflowStepDetail.sessions` のような Step detail として扱う。

## 処理フロー

### Session 選択

1. ユーザーが WorkspaceList の Session row をクリックする。
2. WorkspaceList が `CenterSelection.kind = "agentSession"` を発行する。
3. MainLayout が中央パネルに `AgentChatPanel` を表示する。
4. `AgentChatPanel` が該当 Session の既存 Chat UI を表示する。

### Step 選択

1. ユーザーが WorkspaceList の Step row をクリックする。
2. WorkspaceList が `CenterSelection.kind = "workflowStep"` を発行する。
3. MainLayout が中央パネルに StepView を表示する。
4. StepView が該当 Step の header と grid を表示する。
5. Step 内の各 Session は Pane として等分配置される。

### Workflow row 操作

1. ユーザーが Workflow row をクリックする。
2. WorkspaceList が Step 一覧を展開または折りたたむ。
3. `CenterSelection` は変更しない。
4. 中央パネルに表示中の Session / Step は維持される。

### Step approval

1. approval 待ち Step を表示する。
2. 中央パネルヘッダーに `Approve` を表示する。
3. reject 可能な Step では `Reject` も表示する。
4. `Reject` 押下時は comment 入力 popup または menu を表示する。
5. approval 操作が失敗した場合、中央パネルヘッダーに error icon を表示し、icon から error popup を再表示できるようにする。

### Visibility lifecycle

Step / Workflow / Session の状態変化は表示状態を勝手に変更しない。

- Step completed による自動 close はしない。
- Step failed / aborted による自動 close はしない。
- Step status による自動 hide はしない。
- Workflow completed による自動非表示はしない。
- WorkflowTrace の Step session 表示/非表示 toggle は持たない。

## エラー処理

- Step detail の取得に失敗した場合、中央パネル内で対象 Step の表示エラーとして扱う。WorkspaceList 全体は壊さない。
- Approval 操作が失敗した場合、中央パネルヘッダーに error icon を残す。error popup は `x` で閉じられるが、icon クリックで再表示できる。
- Reject comment が空の場合、Reject 確定は実行できない。
- Stop や archive が実行できない Workflow では、操作を disabled にするか、既存のエラー表示方針に従って失敗を表示する。
- Workflow row menu の位置決めに失敗して画面左上に飛ぶ状態は許容しない。Worktree row menu と同じ表示方式に揃える。

## テスト方針

本 Issue は UI 再整理が中心であり、Workflow engine の新規ロジックは追加しない。既存テストの緑維持と、必要な範囲での UI 回帰確認を行う。

Frontend 確認観点:

- Session row click で既存 Chat UI が表示される。
- Step row click で Step grid が表示される。
- Workflow row click で中央パネルの表示対象が変わらない。
- Step grid に tab bar / resize handle / Step 内 Session close UI が表示されない。
- Pane が等分配置され、最小サイズ未満にならず、横スクロールが発生しない。
- Pane 内の Chat UI で本文スクロールと入力欄表示が維持される。
- Step header の左右 padding が grid 外側余白と一致する。
- Step status icon に背景丸、raw status、進捗文字、独自 dot が表示されない。
- Workflow row menu が Worktree row menu と同じ位置に開き、左上へ飛ばない。
- `BaseBranchUI` が右パネルヘッダーにあり、中央パネルヘッダーにない。
- Step completed / failed / aborted 後も Step row と表示中 Step 画面が勝手に消えない。

Rust / engine 確認観点:

- #1242 では Workflow engine の状態遷移を変更しない。
- Rust DTO の親子関係・状態がフロントで推測されていないことを確認する。

## リスクと代替案

- **リスク 1: WorkspaceList と WorkflowTrace の責務重複**。表示/非表示や navigation を両方に持たせると UI の意味が重複する。対策として、navigation は WorkspaceList に寄せ、WorkflowTrace から表示/非表示操作を削除する。
- **リスク 2: Step grid の最小サイズとスクロールの混線**。Pane 自体が内容で伸びると無制限高さになり、Pane 内スクロールが死ぬ。対策として、grid のセルサイズと Pane 内スクロールを分離する。
- **リスク 3: Workflow row を中央表示対象にしてしまうこと**。Workflow row は group であり、選択して Workflow 画面を表示する対象ではない。クリックは expand / collapse に限定する。
- **リスク 4: 仮データが表示コンポーネントへ残ること**。UI 検証のための stub は Rust 通信境界に限定し、最終状態では Rust DTO を描画する。

## 仮定

- 既存 Chat UI は Step grid 内でも同じ表示・入力・スクロールの単位として再利用できる。
- `Parallel` は複数 Session を含む 1 つの Step として Rust DTO から表現できる。
- Step action の可否は Step detail の状態と `canReject` から判断できる。
- Workflow row の `Stop` / archive は既存または追加される Workflow 操作 command に接続できる。

## Open Questions

なし。
