# Behavior

本書は #1242 Workflow UI 再整理の外部から観測される振る舞いを定義する。要求と範囲は `requirements.md` を参照する。

通常の振る舞いシナリオでは、実装方式、データ取得方法、コンポーネント分割、Rust との接続方法は扱わない。現状実装の記録が必要な箇所は Current Behavior に限定して記載する。

## Assumptions

- 中央パネルの表示対象は `Session` または `Step` のどちらかである。
- WorkspaceList は `Session` と `Step` の選択場所であり、`Workflow` は Step 一覧の group と Workflow 状態表示として扱われる。
- `Parallel` は複数 `Session` を含む 1 つの `Step` である。
- Step grid 内の `Session` は既存 Chat UI と同じ入力・スクロールの振る舞いを持つ。
- Workflow Step 表示時、Step の情報と action は中央パネルヘッダーに表示される。
- BaseBranchUI は右パネルヘッダーに表示される。
- WorkflowTrace はナビゲーションや表示/非表示操作の場所ではない。

## Current Behavior

この節は、この worktree の現状実装が外部からどう見えるか、およびその背景にある投影ルールを記録する。理想モデルは `design.md` の Parentage Model を参照する。

### Session lifecycle と WorkspaceList への入力

現状の `ChatSession` / `SessionSummary` は `workflow_step_session: bool` を持つが、`runId` / `stepName` / `runIndex` は持たない。

WorkspaceList 用 command は、対象 worktree について次の Session を集める。

- `SessionState::Active` / `Idle` / `Done` / `Error` の Session。
- `SessionState::Closed` のうち `workflow_step_session = true` の Session。

通常 Session の `Closed` は WorkspaceList の通常一覧には入らない。`Archived` は WorkspaceList 用入力には入らない。削除済み Session は SessionStore に存在しないため入力に入らない。

`workflow_step_session = false` の Session は、WorkspaceList の通常 Session row として表示される。`workflow_step_session = true` の Session は、通常 Session row にはならず、Workflow / Step 側へ紐付けられる候補になる。

### Workflow row の表示

WorkspaceList は対象 worktree の Workflow run 一覧を取得し、manual archive されていない run を Workflow row として表示する。

Workflow row は、対応する Step Session が 0 件でも表示される。terminal 化した Workflow run も、manual archive されていなければ表示される。

manual archive された Workflow run は WorkspaceList から外れ、Workflow history 側に表示される。Workflow run の archive は Workflow row の表示場所を変える操作であり、個々の Session の `SessionState::Archived` とは別である。

### Step row の生成

現状の Step row は、Workflow 定義の全 node から直接作られるわけではない。Step row は `WorkflowStateSnapshot` から作る `StepSessionRef` をもとに生成される。

`StepSessionRef` の元になるものは次の通り。

- `step_history[*]`: 完了済みまたは中断済みとして記録された Step。
- `step_history[*].child_outputs[*]`: parallel parent の履歴に含まれる child Step 出力。
- `current_session_id`: 現在実行中 Step の Session。
- `active_parallel_steps[*]`: 現在実行中の parallel child。
- 実行中または approval 待ちで `current_session_id` がない current Step。

そのため、未起動の定義上の node は Step row にならない。Session を持たない bash Step などでも、`step_history` または current Step として `WorkflowStateSnapshot` に現れていれば Step row になる。

`WorkflowStateSnapshot` が取得できない run では、Workflow row は表示されるが Step row は生成されない。

### Step / Session の親子付け

新規 Workflow Step Session は `workflowStepContext` を持つ。
WorkspaceList と Step detail は、この context を親子関係の主入力として使う。

`workflowStepContext` がある Session の扱い:

1. `workflowStepContext.runId` が Workflow row の run と一致する場合、その Workflow 配下に表示される。
2. `parentStepName` / `parentRunIndex` がある場合、Session は parent Step の Step row に group される。
3. `parentStepName` がない場合、Session は `stepName + runIndex` の Step row に group される。
4. 同一 `stepName` でも `runIndex` が異なれば別 Step row として表示される。
5. `WorkflowStateSnapshot` 上の `current_session_id` / `step_history` / `active_parallel_steps` が異なる step 名を指していても、context の親子関係は上書きされない。

`WorkflowStateSnapshot` は Step status / Step type / approval action 可否 / sessionless Step の互換表示に使う。
Session 所属の正本にはしない。

#### Legacy fallback

既存 Session には `workflowStepContext` が存在しない場合がある。その場合に限り、互換目的で `session_id` の一致による fallback を行う。

1. WorkspaceList 用入力から `workflow_step_session = true` の Session を `session_id -> Session` の map にする。
2. `WorkflowStateSnapshot` から `StepSessionRef` を作る。
3. `StepSessionRef.session_id` が map に存在する場合、その Session を該当 Step row の `sessions` に入れる。
4. `StepSessionRef.session_id` がない場合でも、Step row 自体は sessionless Step として表示される。
5. 入力 Session 側に存在しても、どの `StepSessionRef` からも参照されない workflow step Session は表示されない。

この fallback は legacy Session 向けの最小互換であり、新規 Session の親子関係には使わない。
`workflow_step_session = true` だけでは WorkspaceList 上の親子関係を決めない。

### Parallel Step の扱い

Parallel child は WorkspaceList の主ナビゲーション単位にはならない。parallel child の Session は、親 `Parallel Step` の Step row に group される。

現状の group ルールは次の通り。

- `active_parallel_steps[*]` は current Step を group parent として扱う。
- `step_history[*].child_outputs[*]` は、その history entry の Step を group parent として扱う。
- child 自身の `step_name` は Session row の title として使われる。
- Step row の title は parent Step 名になる。

parallel が完了すると `active_parallel_steps` は空になる。完了後も child Session を WorkspaceList に残すには、projection が parent `step_history[*].child_outputs` に child Session 参照を保存している必要がある。

### Step order

Step row の順序は現状、Workflow 定義順ではなく実行記録由来である。

1. `step_history` の順序。
2. その後に current Step。
3. current Step 配下の active parallel child は current Step と同じ order。

同一名 Step が複数回実行された場合は `runIndex` で別 Step row として区別される。Step row id は `runId:stepName:runIndex` 形式である。

### Step status / type / action 可否

Step row の status は次の優先順で決まる。

1. 表示対象 Step が current Step かつ `runIndex` も一致する場合、Workflow execution state を使う。
2. `state.current_step_name != step_name` で、かつ `step_states[stepName]` がある場合、それを使う。
   同名 Step で `state.current_step_name == step_name` だが `runIndex` が current と異なる過去 run の行は、`step_states` を参照せず refs 由来 status を使う。
3. Step row に含まれる `StepSessionRef` の状態に failed / aborted / waiting_approval / running があれば、その優先順で使う。
4. 参照があるが上記に当たらない場合は completed。
5. 参照がない場合は queued。

Step type は `workflow_definition.nodes[*].node_type` から解決する。解決できない場合は `agent` として扱う。

`canReject` は current Step であり、`runIndex` が一致し、Workflow execution state が `WaitingApproval` の場合だけ返る。それ以外の Step row では `canReject` はない。

### Known limitations in current behavior

現状の親子関係復元は、Workflow実行記録とSession所属を兼用している。

- `step_history` は本来実行履歴であり、Session所属の正本ではない。
- `workflow_step_session = true` の Session でも、`WorkflowStateSnapshot` に対応する `session_id` 参照がなければ WorkspaceList に出ない。
- `Archived` Session は WorkspaceList 用入力に入らないため、Step row 配下には表示されない。
- manual archive された Workflow run は、Step Session が存在しても WorkspaceList から外れる。
- projection が parallel completion 時に `child_outputs` を作らない場合、完了後に parallel child Session への到達経路が失われる。
- Workflow run の状態と Session の状態は別軸だが、現状の投影では WorkflowState に Session所属の情報が寄っている。

したがって、現状実装は `WorkflowRun -> StepRun -> Session` の親子関係を明示保存する最終形ではない。最終形では Session または専用 read model に親子関係を保存し、WorkflowState は status / type / approval などの補助情報に限定して使う。

## Feature: Workflow UI Navigation

### Background

```gherkin
Background:
  Given Releash デスクトップアプリが起動している
  And WorkspaceList に Worktree が表示されている
```

## Rule: 中央パネルは Session または Step のみを表示する

```gherkin
Scenario: 通常 Session を選択する
  Given WorkspaceList に通常 Session 行が表示されている
  When ユーザーがその Session 行をクリックする
  Then 中央パネルには該当 Session の Chat UI が表示される
  And Chat 入力欄が表示される
  And Session の本文は Chat UI 内でスクロールできる

Scenario: Workflow Step を選択する
  Given WorkspaceList に Workflow の Step 行が表示されている
  When ユーザーがその Step 行をクリックする
  Then 中央パネルには該当 Step 画面が表示される
  And 以前表示していた Session または Step は中央パネルから置き換わる

Scenario: Workflow 行をクリックする
  Given WorkspaceList に Workflow 行が表示されている
  And 中央パネルに Session または Step が表示されている
  When ユーザーが Workflow 行をクリックする
  Then Workflow 行配下の Step 一覧が展開または折りたたまれる
  And 中央パネルの表示対象は変わらない
  And 特定の Step は自動選択されない
```

## Rule: BaseBranchUI は右パネルヘッダーに表示される

```gherkin
Scenario: 右パネルヘッダーを見る
  Given ユーザーが worktree を表示している
  When ユーザーが右パネルヘッダーを見る
  Then 現在 branch から base branch への関係が表示される
  And base branch を変更する操作に到達できる
  And 右パネルの toggle 操作は従来どおり表示される

Scenario: 中央パネルヘッダーを見る
  Given ユーザーが worktree を表示している
  When ユーザーが中央パネルヘッダーを見る
  Then BaseBranchUI は表示されない
```

## Rule: Parallel は 1 つの Step として表示される

```gherkin
Scenario: Parallel Step を選択する
  Given Workflow に複数 Session を持つ Parallel Step が存在する
  When ユーザーが WorkspaceList でその Step 行をクリックする
  Then 中央パネルには 1 つの Step 画面が表示される
  And Step 画面内に複数の Session Pane が表示される
  And Parallel child は中央パネルの独立した表示対象として扱われない

Scenario: Parallel Step 内の Session は同じ Step 画面内に並ぶ
  Given Parallel Step に review-opus と review-gpt55 の Session が存在する
  When ユーザーがその Parallel Step を表示する
  Then review-opus の Chat UI が 1 つの Pane に表示される
  And review-gpt55 の Chat UI が別の Pane に表示される
  And どちらの Pane にも Chat 入力欄が表示される
```

## Rule: Step grid は Pane を等分配置する

```gherkin
Scenario: Step に複数 Session がある
  Given Step に複数 Session が存在する
  When Step 画面が表示される
  Then 各 Session は grid の Pane として表示される
  And 各 Pane は利用可能な Step 画面内で均等に配置される
  And ユーザーが操作するサイズ変更 handle は表示されない

Scenario: Pane の最小サイズを維持できる
  Given Step 画面の表示領域が各 Pane の最小サイズを満たしている
  When Step 画面が表示される
  Then 各 Pane は最小サイズ以上で表示される
  And Step 画面に横スクロールは発生しない

Scenario: Pane の最小サイズを縦方向に維持する
  Given Step 画面の高さが各 Pane の最小高さを満たせない
  When Step 画面が表示される
  Then 各 Pane は最小高さより小さくならない
  And Step 画面は縦方向に overflow する
  And Step 画面に横スクロールは発生しない

Scenario: Session の内容が増える
  Given Step grid 内の Pane に Chat UI が表示されている
  When Session の本文が Pane の表示領域より長くなる
  Then Pane 自体は内容に引きずられて無制限に伸びない
  And Session の本文領域が Pane 内でスクロールできる
```

## Rule: Step 画面には余計な切替 UI を置かない

```gherkin
Scenario: Step 画面を見る
  Given ユーザーが Workflow Step を表示している
  Then Step 画面に tab bar は表示されない
  And Step grid 内に独自 header は表示されない
  And Step 画面に resize UI は表示されない
  And Step 内 Session の close UI は表示されない

Scenario: Step 内の Session を閉じようとする
  Given Step 画面に複数 Session Pane が表示されている
  When ユーザーが各 Pane を見る
  Then 個別 Session を閉じる操作は表示されない
```

## Rule: WorkspaceList は状態確認とナビゲーションを担う

```gherkin
Scenario: Session 行を見る
  Given WorkspaceList に通常 Session 行が表示されている
  Then Session 行から Session の状態を確認できる

Scenario: Step 行を見る
  Given WorkspaceList に Workflow Step 行が表示されている
  Then Step 行から Step の状態を確認できる
  And Step 状態は既存の Step status icon で表示される
  And Step status icon に不要な背景丸は表示されない
  And raw status 文字列、進捗文字、独自 dot は表示されない

Scenario: Workflow 行を見る
  Given WorkspaceList に Workflow 行が表示されている
  Then Workflow 行に Workflow 単体の状態表示は出ない
  And 配下 Step 行の状態から Workflow 全体の実行状況を把握できる
  And Workflow 名の右に展開状態を示す矢印が表示される
```

## Rule: Workflow 行から Workflow 操作に到達できる

```gherkin
Scenario: Workflow を archive する
  Given WorkspaceList に Workflow 行が表示されている
  When ユーザーが Workflow 行の x ボタンをクリックする
  Then 該当 Workflow に対する archive 操作が実行される
  And Workflow 行クリックによる展開/折りたたみは発火しない

Scenario: Workflow menu を開く
  Given WorkspaceList に Workflow 行が表示されている
  When ユーザーが Workflow 行の menu ボタンをクリックする
  Then Workflow 用 menu が表示される
  And menu 内に Stop が表示される
  And Workflow 用 menu は Workflow 行の menu ボタンに隣接して表示される
  And Workflow 用 menu は画面左上に表示されない

Scenario: Workflow menu を開いたまま Workflow 行から hover が外れる
  Given Workflow 用 menu が表示されている
  When ユーザーの pointer が Workflow 行から外れる
  Then Workflow 用 menu は menu ボタンに隣接した位置を維持する
  And Workflow 用 menu は画面左上へ移動しない

Scenario: 停止可能な Workflow を Stop する
  Given Workflow が停止可能な状態である
  And Workflow menu が表示されている
  When ユーザーが Stop をクリックする
  Then 該当 Workflow に対する Stop 操作が実行される

Scenario: 停止できない Workflow の Stop を見る
  Given Workflow が停止できない状態である
  When ユーザーが Workflow menu を開く
  Then Stop は実行できない状態で表示される
```

## Rule: 中央パネルヘッダーは Step 情報と Step action を扱う

```gherkin
Scenario: Approval Step を表示する
  Given Step が approval 待ちである
  When ユーザーがその Step を表示する
  Then 中央パネルヘッダーに Step の status icon が表示される
  And 中央パネルヘッダーに Step 名が表示される
  And 中央パネルヘッダーに Step type が表示される
  And 中央パネルヘッダーに Approve action が表示される
  And Step 画面下部に Step action footer は表示されない

Scenario: Reject 可能な Approval Step を表示する
  Given Step が approval 待ちである
  And Step が Reject 可能である
  When ユーザーがその Step を表示する
  Then 中央パネルヘッダーに Reject action が表示される

Scenario: Reject 不可の Approval Step を表示する
  Given Step が approval 待ちである
  And Step が Reject 不可である
  When ユーザーがその Step を表示する
  Then 中央パネルヘッダーに Reject action は表示されない
  And Approve action は表示される

Scenario: Step header の左右余白を見る
  Given ユーザーが Workflow Step を表示している
  When 中央パネルヘッダーと Step grid を見る
  Then 中央パネルヘッダーの左 padding は Step grid の外側余白と一致する
  And 中央パネルヘッダーの右 padding は Step grid の外側余白と一致する
```

## Rule: Reject には comment 入力が必要である

```gherkin
Scenario: Reject を押す
  Given Reject 可能な Approval Step が表示されている
  When ユーザーが Reject をクリックする
  Then Reject comment 入力用の popup または menu が表示される
  And comment 未入力の状態では Reject を確定できない

Scenario: Reject comment を入力して確定する
  Given Reject comment 入力用の popup または menu が表示されている
  When ユーザーが comment を入力する
  And Reject を確定する
  Then comment 付きで Reject 操作が実行される
  And Reject comment 入力 UI は閉じる

Scenario: Reject comment 入力を取り消す
  Given Reject comment 入力用の popup または menu が表示されている
  When ユーザーが Cancel をクリックする
  Then Reject 操作は実行されない
  And Reject comment 入力 UI は閉じる
```

## Rule: Approval 操作の失敗は中央パネルヘッダーから確認できる

```gherkin
Scenario: Approval 操作が失敗する
  Given Approval Step が表示されている
  When ユーザーが Approve または Reject を実行する
  And 操作が失敗する
  Then 中央パネルヘッダーに error icon が表示される
  And error 内容の popup が表示される

Scenario: Error popup を閉じる
  Given 中央パネルヘッダーに error icon が表示されている
  And error popup が表示されている
  When ユーザーが popup の x をクリックする
  Then error popup は閉じる
  And 中央パネルヘッダーの error icon は残る

Scenario: Error popup を再表示する
  Given 中央パネルヘッダーに error icon が表示されている
  And error popup が閉じている
  When ユーザーが error icon をクリックする
  Then error popup が再度表示される
  And popup 内に error 内容が表示される
```

## Rule: WorkflowTrace は表示/非表示操作を持たない

```gherkin
Scenario: WorkflowTrace を見る
  Given WorkflowTrace が表示されている
  Then Hide は表示されない
  And Viewing は表示されない
  And Step session の表示/非表示トグルは表示されない

Scenario: WorkflowTrace 上の Step を見る
  Given WorkflowTrace に Step が表示されている
  Then WorkflowTrace は WorkspaceList と同じ表示/非表示操作を提供しない
```

## Rule: Step detail と Event log は表示しない

```gherkin
Scenario: Workflow Step を表示する
  Given ユーザーが Workflow Step を選択している
  Then Step detail パネルは表示されない
  And Event log パネルは表示されない
```

## Rule: 状態変化だけで勝手に消えない

```gherkin
Scenario: Step が completed になる
  Given WorkspaceList に Workflow Step 行が表示されている
  When Step が completed になる
  Then Step 行は状態を更新する
  And 状態変化だけを理由に自動で非表示にならない
  And 表示中の Step 画面は状態変化だけを理由に自動で閉じられない

Scenario: Step が failed または aborted になる
  Given WorkspaceList に Workflow Step 行が表示されている
  And 中央パネルにその Step 画面が表示されている
  When Step が failed または aborted になる
  Then Step 行は状態を更新する
  And Step 行は状態変化だけを理由に自動で非表示にならない
  And 中央パネルの Step 画面は状態変化だけを理由に自動で閉じられない

Scenario: Step の自動 close / 自動 hide が存在しない
  Given ユーザーが Workflow Step を表示している
  When Step の状態が変化する
  Then Step を勝手に閉じる動作は発生しない
  And Step を勝手に非表示にする動作は発生しない

Scenario: Workflow が completed になる
  Given WorkspaceList に Workflow 行が表示されている
  When Workflow が completed になる
  Then Workflow 行は状態を更新する
  And 状態変化だけを理由に自動で非表示にならない
```
