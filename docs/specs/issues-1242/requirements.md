# Requirements

## Type

Workflow UI の再整理。

関連: #1242 / #1220 / #1023

## Goal

Workflow 実行中の `Session` / `Step` / `Workflow` を、左の WorkspaceList と中央パネルで迷わず扱える UI にする。

中央パネルは表示対象を `Session` と `Step` のみに絞る。WorkspaceList は `Session` と `Step` のナビゲーションを担い、`Session` / `Step` / `Workflow` それぞれの状態を確認できる場所にする。

`Parallel` は独立した表示対象にせず、複数 `Session` を含む 1 つの `Step` として扱う。

## Terms

- `Session`: 通常の Chat Session。
- `Workflow`: Workflow run 全体。
- `Step`: Workflow run 内の実行単位。
- `Parallel Step`: 複数 `Session` を含む 1 つの `Step`。
- `Pane`: Step 画面の grid 内で 1 つの `Session` を表示する領域。
- `BaseBranchUI`: 現在 branch と base branch の関係を表示・変更する UI。
- `Step Session`: Workflow `Step` によって作られ、その `Step` の子として表示される `Session`。
- `Step Run`: 同一 `Step` 名が retry / loop で複数回実行される場合の 1 回分の実行。`stepName + runIndex` で区別する。

## Scope

- 中央パネルの表示単位を `Session` / `Step` に整理する。
- WorkspaceList から通常 `Session` と Workflow `Step` を選択できるようにする。
- WorkspaceList 上で `Session` / `Step` / `Workflow` の状態を確認できるようにする。
- `Parallel` を 1 つの `Step` として表示・選択する。
- `Parallel Step` 配下の複数 `Session` を同じ Step 画面内に grid 表示する。
- Workflow Step 表示時の中央パネルヘッダーに Step 情報と action を置く。
- Workflow 行に Workflow 操作用の menu と archive 操作を置く。
- Workflow / Step / Session の親子関係を UI の主データとして扱う。
- WorkflowTrace から、ナビゲーションや表示/非表示に関わる UI を外す。
- Step detail パネルと Event log パネルを外す。
- `BaseBranchUI` を右パネルヘッダーへ配置する。

## Non-goals

- Workflow engine の実行モデル変更。
- Workflow 定義や YAML schema の変更。
- `Parallel` を独立した中央表示対象にすること。
- `Parallel` child を WorkspaceList の主ナビゲーション単位にすること。
- WorkflowTrace を WorkspaceList の代替ナビゲーションにすること。
- 中央パネルに任意の表示/非表示管理 UI を置くこと。
- 既存 Chat UI の振る舞いを Step grid 用に作り替えること。
- Source Control / Editor / Terminal など Workflow 以外の画面再設計。

## Requirements

### Central Panel

- 中央パネルは `Session` または `Step` のどちらかを表示すること。
- 通常 `Session` を選択した場合、中央パネルはその `Session` を既存 Chat UI として表示すること。
- Workflow `Step` を選択した場合、中央パネルはその `Step` 画面を表示すること。
- `Step` を切り替える場合、中央パネルは Step 画面全体を切り替えること。
- Workflow `Step` 表示時、中央パネルヘッダーは Step 情報と Step action を表示すること。
- 通常 `Session` 表示時、中央パネルは既存のヘッダー構造を維持すること。
- 中央パネルヘッダーに `BaseBranchUI` を置かないこと。
- Workflow 行クリックでは中央パネルを `Workflow` 表示に切り替えないこと。
- Workflow 行クリックでは現在表示中の `Session` / `Step` を変更しないこと。
- Workflow 行クリックで特定の `Step` を自動選択しないこと。

### Right Panel Header

- `BaseBranchUI` は右パネルヘッダーに表示すること。
- `BaseBranchUI` は現在 branch から base branch への関係を表示すること。
- `BaseBranchUI` から base branch を変更できること。
- 右パネルヘッダーの既存 toggle 操作は維持すること。

### Step Grid

- Step 画面は grid として表示されること。
- Step 画面の grid は、Step 内の `Session` を均等な `Pane` として配置すること。
- Step 画面の grid は、iTerm の grid に近い動作にすること。
- Step 画面にはタブバーを置かないこと。
- Step grid 内には独自ヘッダーを置かないこと。
- Step 画面にはサイズ変更 UI を置かないこと。
- 同一 Step 内の `Session` は個別に閉じられないこと。
- Step 画面には Step 内 `Session` の close UI を置かないこと。
- Step 画面内の `Pane` は UI 内に等分で配置されること。
- Step 画面内の `Pane` は grid のセルとして扱われること。
- Step 画面内の `Pane` は最小の縦横サイズを持つこと。
- `Pane` は最小サイズより小さくならないこと。
- `Pane` の最小サイズを維持できない場合、横スクロールではなく縦方向の overflow で扱うこと。
- `Pane` 自体は内容に引きずられて無制限に伸びないこと。
- `Session` の中身は `Pane` 内でスクロールできること。
- Step grid 内の各 `Pane` は既存 Chat UI を表示すること。
- Step grid 内の `Session` には Chat 入力欄が表示され、既存 Chat UI と同じように入力できること。
- 選択中 `Pane` は枠線で分かること。
- 選択中 `Pane` に専用の背景色を入れないこと。
- 中央パネルヘッダーの左右 padding は Step grid の外側余白と揃えること。

### WorkspaceList

- WorkspaceList は通常 `Session` と Workflow `Step` のナビゲーションを担うこと。
- WorkspaceList の `Session` 行は Session の状態を確認できること。
- WorkspaceList の `Step` 行は Step の状態を確認できること。
- Workflow 行自体には状態表示を持たず、配下 `Step` 行の状態から Workflow 全体の実行状況を把握できること。中央パネルの表示対象そのものを `Workflow` にしないこと。
- Workflow 行クリックは配下の Step 一覧の展開/折りたたみだけを行うこと。
- Workflow 行には名前の右側に展開/折りたたみ矢印を置くこと。
- Workflow 行には archive 用の `x` ボタンを置くこと。
- Workflow 行には menu ボタンを置くこと。
- Workflow 行 menu には `Stop` を置くこと。
- `Stop` は停止可能な Workflow に対して実行できること。
- Workflow 行 menu は Worktree 行 menu と同じ位置決め・表示方式で開くこと。
- Workflow 行 menu は Workflow 行の menu ボタンに隣接して表示されること。
- Workflow 行 menu が画面左上など、menu ボタンから離れた位置に表示されないこと。
- Workflow 行の menu ボタンは menu open 中も位置参照できる状態で残ること。
- Step 行クリックで中央パネルは該当 `Step` 画面へ切り替わること。
- Step 行の状態表示は既存の Step status icon を使うこと。
- Step 行の状態 icon には不要な背景丸を付けないこと。
- Step 行に raw status 文字列、進捗文字、独自 dot を追加しないこと。
- `Parallel` child は WorkspaceList の主ナビゲーション単位にならないこと。

### Workflow / Step / Session Parentage

- Workflow UI の親子関係は `WorkflowRun -> StepRun -> Session` として扱うこと。
- `StepRun` は `WorkflowRun` に属し、0 個以上の `Session` を持つこと。
- `Step Session` は 1 つの `StepRun` に属すること。
- 同一名 `Step` が複数回実行される場合、各 `StepRun` は `runIndex` で区別されること。
- `Parallel Step` では、parallel child の `Session` は WorkspaceList の独立 Step 行ではなく、親 `Parallel Step` の `StepRun` 配下に表示されること。
- `Step` の completed / failed / aborted / waiting_approval / running は、親子関係を削除する理由にならないこと。
- `Session` の Active / Closed / Archived / Deleted 相当のライフサイクルは、Workflow / Step の実行状態とは別軸として扱うこと。
- WorkspaceList と Step 画面は、`Session` が属する `WorkflowRun` と `StepRun` を Rust DTO から受け取り、フロントエンド側で親子関係を推測しないこと。
- 現状実装の親子関係復元ロジックと制約は `behavior.md` の Current Behavior に詳細を記載すること。

### WorkflowTrace / Removed Panels

- WorkflowTrace は WorkspaceList と同じナビゲーション責務を持たないこと。
- WorkflowTrace に表示/非表示の操作を置かないこと。
- WorkflowTrace に `Hide` を置かないこと。
- WorkflowTrace に `Viewing` を置かないこと。
- Step session の表示/非表示トグルは機能として削除すること。
- Step detail パネルは置かないこと。
- Event log パネルは置かないこと。

### Step Actions

- Workflow Step 表示時の中央パネルヘッダーには Step の識別に必要な情報と Step action を置くこと。
- 中央パネルヘッダーには Step の status icon、Step 名、Step type を表示すること。
- 中央パネルヘッダーには approval 状態の Step に対して `Approve` action を置くこと。
- 中央パネルヘッダーには reject 可能な approval Step に対して `Reject` action を置くこと。
- Step 画面下部に Step action 用 footer を置かないこと。
- `Reject` action は `canReject` を反映すること。
- `canReject` が false の Step では `Reject` action を表示しないこと。
- `Reject` 押下時は comment 入力の popup または menu を表示すること。
- `Reject` は comment が入力されるまで確定できないこと。
- `Reject` comment は Reject 操作の内容として扱われること。
- Approval 操作が失敗した場合、中央パネルヘッダーに error icon を表示すること。
- Error icon をクリックすると error 内容の popup を表示すること。
- Error popup は `x` で閉じられること。
- Error popup を閉じても error icon は残り、icon クリックで再度表示できること。

### Visibility / Lifecycle

- `Session` / `Step` / `Workflow` は、状態変化だけでユーザーに分からない形で勝手に消えないこと。
- 完了状態になったことだけを理由に WorkspaceList から自動非表示にしないこと。
- `Step` を状態変化だけで自動的に閉じないこと。
- `Step` を状態変化だけで自動的に非表示にしないこと。
- `Step` を勝手に閉じる、または勝手に非表示にするロジックは削除すること。
- 表示/非表示の責務を中央パネルや WorkflowTrace に持たせないこと。

## Constraints

- UI 上の主語は `Session` / `Step` / `Workflow` に揃えること。
- `Parallel` は UI 上の主語にしないこと。
- ナビゲーション責務は WorkspaceList に寄せること。
- WorkflowTrace と WorkspaceList で、同じナビゲーションや表示操作を重複して持たないこと。
- 中央パネルは選択された対象を表示する場所であり、一覧や表示管理の場所にしないこと。
- 既存 Chat UI の基本操作を Step grid 内で変えないこと。
- 本要求文書では実装方式を定義しないこと。

## Acceptance Criteria

- WorkspaceList から通常 `Session` を選択し、中央パネルでその `Session` を確認できる。
- WorkspaceList から Workflow `Step` を選択し、中央パネルでその `Step` 画面を確認できる。
- Parallel Step を選択した場合、中央パネルでは 1 つの `Step` 画面として表示される。
- Parallel Step 内の複数 `Session` が、同じ Step 画面内に表示される。
- Step 画面が grid として表示される。
- Step 画面の grid で、各 `Session Pane` が均等に配置される。
- Step 画面の grid が iTerm の grid に近い動作をする。
- Step grid 内の各 Chat UI で Session の本文をスクロールできる。
- Step grid 内の各 Chat UI に Chat 入力欄が表示される。
- `Pane` が最小サイズを下回らない。
- Step 画面で横スクロールが発生しない。
- WorkspaceList 上で Session の状態を確認できる。
- WorkspaceList 上で Step の状態を確認できる。
- WorkspaceList 上で Workflow 全体の実行状況を確認できる。
- Workflow 行クリックで配下の Step 一覧を展開/折りたたみできる。
- Workflow 行クリックで中央パネルの表示対象が変わらない。
- Workflow 行の menu から `Stop` に到達できる。
- Workflow 行の menu が Workflow 行の menu ボタンに隣接して表示される。
- Workflow 行の menu が画面左上へ飛ばない。
- Workflow 行の `x` から archive 操作に到達できる。
- 右パネルヘッダーに `BaseBranchUI` が表示される。
- 中央パネルヘッダーに `BaseBranchUI` が表示されない。
- 中央パネルに Workflow Step のタブバーが表示されない。
- Step grid 内に Workflow Step の独自ヘッダーが表示されない。
- Workflow Step 表示時の中央パネルヘッダーに Step の status icon、Step 名、Step type が表示される。
- Workflow Step 表示時の中央パネルヘッダーの左右 padding が Step grid の外側余白と揃っている。
- 中央パネルに Workflow Step のサイズ変更 UI が表示されない。
- 中央パネルに Step 内 Session の close UI が表示されない。
- WorkflowTrace に `Hide` / `Viewing` / 表示切り替え操作が表示されない。
- Step detail パネルが表示されない。
- Event log パネルが表示されない。
- Approval Step で `Approve` できる。
- Reject 可能な Approval Step で comment 入力後に `Reject` できる。
- Reject 不可の Step では `Reject` が表示されない。
- Approval 操作失敗時に中央パネルヘッダーの error icon から error 内容を確認できる。
- Step が completed / failed / aborted などへ変化しても、勝手に閉じたり非表示にならない。
