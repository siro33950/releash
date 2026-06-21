# Behavior

本書は #1220「WorkspaceサイドバーをRepository→Worktree→Session/Workflowのツリー構造に再編する」の外部から観測される振る舞いを定義する。実装詳細は `design.md`、要求と範囲は `requirements.md` を参照する。

## Assumptions

- `Session` は通常の Chat Session を指す。`workflowStepSession !== true` の Session は Worktree 直下に表示する。
- `WorkflowSession` は Workflow run の step に紐づく Chat Session を指す。`workflowStepSession === true` であり、Workflow run の state / history から親 run に紐づく。
- `Workflow` は run 単位で表示する。
- Worktree 親と Workflow 親は navigation target ではなく、tree group として扱う。
- 中央表示は独立した mode ではなく、現在の `CenterSelection` から導出される。
  - `CenterSelection.kind === "agentSession"` のとき中央は該当 Session を表示する。
  - `CenterSelection.kind === "workflowRun"` のとき中央は該当 Workflow run を表示する。
- Session を持たない終端 Workflow run は `auto_no_sessions` として自動 archive され、ツリーから除外されるが WorkflowHistory には残る。
- それ以外の終端状態の行は、ユーザーの明示操作なしに即時非表示にしない。

## Feature: Workspace tree navigation

### Background

```gherkin
Background:
  Given Releash デスクトップアプリが起動している
  And ユーザーが少なくとも1つの repository を Workspace に登録している
```

## Rule: Sidebar は Repository -> Worktree -> Session / Workflow -> WorkflowSession を表示する

```gherkin
Scenario: Worktree 配下に通常 Session と Workflow が並ぶ
  Given Worktree に通常 Session と Workflow run が存在する
  When ユーザーが Workspace サイドバーを見る
  Then Repository の下に Worktree が表示される
  And Worktree の下に Session と Workflow が表示される
  And Workflow の下に WorkflowSession が表示される

Scenario: Worktree 配下は Session 優先、同種内は名前順で表示される
  Given Worktree に複数の Session と Workflow が存在する
  When ユーザーが Worktree を展開する
  Then Session が Workflow より上に表示される
  And Session 同士は名前順で表示される
  And Workflow 同士は名前順で表示される

Scenario: 取得済みの子要素は省略されない
  Given Worktree に5件以上の Session または Workflow が存在する
  When ユーザーが Worktree を展開する
  Then 取得済みの子要素は全件表示される
  And `もっと表示する` は表示されない
```

## Rule: Worktree 行は group として振る舞う

```gherkin
Scenario: Worktree 行クリックは展開状態のみを切り替える
  Given Worktree 行が表示されている
  When ユーザーが Worktree 行をクリックする
  Then その Worktree の子要素が展開または折りたたまれる
  And CenterSelection は変更されない
  And 中央の表示対象 Session または Workflow は変更されない

Scenario: Worktree 行は hover 時だけ名前の右に開閉 chevron を表示する
  Given Worktree 行が表示されている
  When ユーザーが Worktree 行へマウスオーバーする
  Then Worktree 名の右に展開状態を示す chevron が表示される

Scenario: Worktree 行は main と non-main で左アイコンが違う
  Given main worktree と通常 worktree が表示されている
  Then main worktree には Home icon が表示される
  And 通常 worktree には worktree icon が表示される
```

## Rule: Session 行は agentSession selection target である

```gherkin
Scenario: 通常 Session を選択する
  Given Worktree 配下に通常 Session が表示されている
  When ユーザーがその Session 行をクリックする
  Then 該当 Worktree が開かれる
  And CenterSelection は agentSession になる
  And 中央には該当 Session が表示される

Scenario: Session 行 hover で閉じるボタンが表示される
  Given Session 行が表示されている
  When ユーザーが Session 行へマウスオーバーする
  Then 行の右端に閉じるボタンが表示される

Scenario: Session の閉じるボタンを押す
  Given Session 行の閉じるボタンが表示されている
  When ユーザーが閉じるボタンをクリックする
  Then 該当 Session が close される
  And 行クリックによる Session 選択は発火しない
  And close 済み Session は明示削除または archive まで履歴から復帰できる
```

## Rule: Workflow 親は group として振る舞う

```gherkin
Scenario: Workflow 親クリックは展開状態のみを切り替える
  Given Worktree 配下に Workflow が表示されている
  When ユーザーが Workflow 行をクリックする
  Then その Workflow の WorkflowSession が展開または折りたたまれる
  And CenterSelection は変更されない

Scenario: Workflow 行は hover 時だけ名前の右に開閉 chevron を表示する
  Given Workflow 行が表示されている
  When ユーザーが Workflow 行へマウスオーバーする
  Then Workflow 名の右に展開状態を示す chevron が表示される
```

## Rule: WorkflowSession 行は workflowRun selection target である

```gherkin
Scenario: WorkflowSession を選択する
  Given Workflow 配下に WorkflowSession が表示されている
  When ユーザーがその WorkflowSession 行をクリックする
  Then 該当 Worktree が開かれる
  And CenterSelection は workflowRun になる
  And 中央には該当 Workflow run が表示される
  And 可能であれば該当 WorkflowSession に対応する step / transcript が選択される
```

## Rule: Worktree menu から Worktree 関連操作へ到達できる

```gherkin
Scenario: Worktree menu を開く
  Given Worktree 行が表示されている
  When ユーザーが Worktree 行右側の menu ボタンをクリックする
  Then menu が表示される
  And `SessionHistory` が表示される
  And `WorkflowHistory` が表示される
  And `PRリンク` が表示される
  And `削除` が表示される

Scenario: SessionHistory に hover する
  Given Worktree menu が開いている
  When ユーザーが `SessionHistory` にマウスオーバーする
  Then Session history の子 menu が表示される

Scenario: WorkflowHistory に hover する
  Given Worktree menu が開いている
  When ユーザーが `WorkflowHistory` にマウスオーバーする
  Then Workflow history の子 menu が表示される

Scenario: PRリンクを選択する
  Given Worktree に PR URL が存在する
  When ユーザーが Worktree menu の `PRリンク` を選択する
  Then その PR URL が開かれる

Scenario: Worktree 削除を選択する
  Given main worktree ではない Worktree が表示されている
  When ユーザーが Worktree menu の `削除` を選択する
  Then 既存の Worktree 削除確認 dialog が表示される
```

## Rule: Worktree create menu から作業単位を開始できる

```gherkin
Scenario: NewWorkflow を起動する
  Given Worktree 行が表示されている
  And workflow definition が存在する
  When ユーザーが Worktree 行右側の create menu を開く
  And `NewWorkflow` サブメニューから workflow を選択する
  And task を入力して Start する
  Then `start_workflow` が該当 Worktree と task で呼び出される
  And CenterSelection は workflowRun になる
  And 中央には開始された Workflow run が表示される
```

## Rule: Workspace header は Add Worktree のみを表示する

```gherkin
Scenario: Header に Add Worktree 操作が表示される
  When ユーザーが Workspace サイドバーの header を見る
  Then `Workspaces` label が表示される
  And Add Worktree 操作が表示される
  And Group 操作は表示されない
  And Filter 操作は表示されない
```

## Rule: AgentChatPanel の Session タブバーは廃止される

```gherkin
Scenario: AgentChatPanel 上部に Session tab bar が表示されない
  Given ユーザーが agentSession selection で Session を表示している
  Then 中央 AgentChatPanel 上部に Session tab bar は表示されない
  And Session の切替は左 Workspace tree から行う

Scenario: 新規 Session は Worktree 行の action から開始する
  Given Worktree 行が表示されている
  When ユーザーが Worktree 行右側の新規 Session ボタンをクリックする
  Then 該当 Worktree に新規 Session が作成される
  And CenterSelection は作成された Session の agentSession になる
  And 中央には新規 Session が表示される
```

## Rule: 中央表示は CenterSelection から導出される

```gherkin
Scenario: Agent / Workflow toggle は表示されない
  When ユーザーが中央 ViewToolbar を見る
  Then Agent / Workflow を切り替える toggle は表示されない

Scenario: agentSession selection は AgentChatPanel を表示する
  Given CenterSelection が agentSession である
  Then 中央には AgentChatPanel が表示される
  And 該当 Session が表示される

Scenario: workflowRun selection は WorkflowView を表示する
  Given CenterSelection が workflowRun である
  Then 中央には WorkflowView が表示される
  And 該当 Workflow run が表示される
```

## Rule: 表示に仮データ由来のノイズは出ない

```gherkin
Scenario: 相対時刻を表示しない
  When ユーザーが Workspace tree を見る
  Then `1日` や `4時間` のような仮の相対時刻は表示されない

Scenario: 仮状態テキストを表示しない
  When ユーザーが Workspace tree を見る
  Then `Open` や `Closed` のような仮状態テキストは表示されない

Scenario: Folder icon や色付き dot を表示しない
  When ユーザーが Workspace tree を見る
  Then Worktree / Session / Workflow の行に不要な folder icon や色付き dot は表示されない
```

## Open Questions

- WorkflowSession 行に close hover action を表示するか。
- History 子 menu の中身を既存 History popover そのものにするか、menu 用の軽量リストにするか。
- Workflow 親の menu action を Worktree menu とは別に持つか。
