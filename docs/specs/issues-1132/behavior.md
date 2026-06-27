# Behavior

## 位置づけ

本 Issue (#1132) は clean architecture 配置への移行（リファクタリング）であり、外部から観測可能な振る舞い（CLI 出力契約 / Tauri command の I/O / 永続化フォーマット）を変更しない（requirements R9）。

したがって本 `behavior.md` は、新機能の振る舞いではなく **移行前後で不変であるべき観測可能な契約** を Gherkin で定義する。各 Scenario は「移行後もこの振る舞いを満たすこと」を意味し、既存テストの非退行検証として用いる。

層の再配置・モジュール削除・port 化といった構造的要求（requirements R1〜R8）は内部実装であり外部観測対象ではないため、Gherkin には持ち込まない（[[feedback_behavior_definition_granularity]]）。これらは design.md と受け入れ確認（`cargo test` / ディレクトリ構成の確認）で担保する。

---

## Feature: review comment thread のライフサイクル

review comment は thread 単位で作成・追記・解決・削除され、event 追記により状態が決まる。thread の状態は Open / Resolved / Deleted(soft) のいずれかをとる。

### Background

```gherkin
Background:
  Given 有効な worktree が解決できる session が存在する
  And その worktree には review comment の永続化ストアが紐づく
```

### Rule: thread は作成と同時に初回 comment を持つ

```gherkin
Scenario: 新規 thread を作成する
  When 有効な content で thread を作成する
  Then 新しい thread が返る
  And その thread の state は Open である
  And その thread は作成時の content を初回 comment として持つ

Scenario: file と行範囲を指定して thread を作成する
  When file path と line_number, end_line を指定して thread を作成する
  Then 返る thread はその file と行範囲を location として持つ

Scenario: location を指定せず thread を作成する
  When file/line を指定せず content だけで thread を作成する
  Then thread は general（location なし）として作成される
```

### Rule: Open な thread にのみ comment を追記できる

```gherkin
Scenario: Open な thread に comment を追記する
  Given Open な thread がある
  When 有効な content で comment を追記する
  Then その thread に comment が 1 件追加された状態が返る

Scenario: Resolved な thread への追記は拒否される
  Given Resolved な thread がある
  When comment を追記する
  Then AlreadyResolved エラーになる

Scenario: 存在しない thread への追記は拒否される
  When 存在しない thread_id へ comment を追記する
  Then NotFound エラーになる
```

### Rule: thread の解決は一度だけ行える

```gherkin
Scenario: Open な thread を解決する
  Given Open な thread がある
  When outcome と summary を指定して解決する
  Then その thread の state は Resolved になる
  And resolve の outcome と summary が thread に記録される

Scenario: 解決は参加者 identity に依存しない
  Given ある actor が作成した Open な thread がある
  When 別の actor が解決する
  Then 解決は成功する

Scenario: 解決済み thread の再解決は拒否される
  Given Resolved な thread がある
  When 再度解決する
  Then AlreadyResolved エラーになる

Scenario: 存在しない thread の解決は拒否される
  When 存在しない thread_id を解決する
  Then NotFound エラーになる
```

### Rule: thread の削除は human のみが行え、soft delete として記録される

```gherkin
Scenario: human が thread を削除する
  Given Open な thread がある
  When human actor が削除する
  Then 削除は成功する
  And その thread は list / get から見えなくなる

Scenario: 解決済み thread も削除できる
  Given Resolved な thread がある
  When human actor が削除する
  Then 削除は成功する

Scenario: agent による削除は拒否される
  Given thread がある
  When agent actor が削除する
  Then PermissionDenied エラーになる

Scenario: 存在しない / 既削除 thread の削除は拒否される
  When 存在しない、または既に削除済みの thread を削除する
  Then NotFound エラーになる
```

---

## Feature: thread の取得・一覧・フィルタ

### Rule: get は生きている thread のみを返す

```gherkin
Scenario: 存在する thread を取得する
  Given Open な thread がある
  When その thread_id で取得する
  Then その thread が返る

Scenario: 削除済み thread の取得は拒否される
  Given soft delete された thread がある
  When その thread_id で取得する
  Then NotFound エラーになる
```

### Rule: list は削除済みを除き、最新更新順で返す

```gherkin
Scenario: フィルタなしで一覧する
  Given 複数の thread がある
  When フィルタなしで一覧する
  Then 削除済みを除く全 thread が、更新時刻の新しい順で返る

Scenario: state でフィルタする
  Given Open な thread と Resolved な thread がある
  When state=Open でフィルタする
  Then Open な thread のみが返る

Scenario: file でフィルタする
  Given 異なる file に紐づく thread がある
  When 特定の file でフィルタする
  Then その file に一致する thread のみが返る

Scenario: author でフィルタする
  Given viewer 自身が参加する thread と他者のみの thread がある
  When author=self でフィルタする
  Then viewer が参加する thread のみが返る

Scenario: thread_id 列でフィルタする
  Given 複数の thread がある
  When 複数の thread_id を指定してフィルタする
  Then 指定された id に一致する thread のみが返る（OR 結合）

Scenario: 複数軸のフィルタは AND で結合される
  When file と state を同時に指定して一覧する
  Then 両条件を満たす thread のみが返る
```

### Rule: unread は viewer の最終投稿以降の他者投稿で決まる

```gherkin
Scenario: 他者が後から投稿した thread は unread
  Given viewer が投稿した後に他者が comment した thread がある
  When unread=true でフィルタする
  Then その thread が返る

Scenario: viewer が最後に投稿した thread は unread ではない
  Given viewer が最後に comment した thread がある
  When unread=true でフィルタする
  Then その thread は返らない

Scenario: resolve は unread の判定対象に含まれない
  Given viewer の最終投稿の後に、他者の comment はなく resolve のみが行われた thread がある
  When unread=true でフィルタする
  Then その thread は返らない
```

---

## Feature: 履歴と handoff

### Rule: history は thread の全 event を追記順で返す

```gherkin
Scenario: thread の履歴を取得する
  Given 作成・追記・解決・削除を経た thread がある
  When その thread の履歴を取得する
  Then ThreadCreated / CommentAppended / ThreadResolved / ThreadDeleted の各 event が
       追記された時系列順で返る

Scenario: 削除済み thread の履歴も取得できる
  Given soft delete された thread がある
  When その thread の履歴を取得する
  Then 削除 event を含む全 event が返る

Scenario: 存在しない thread の履歴は拒否される
  When 一度も作成されていない thread_id の履歴を取得する
  Then NotFound エラーになる
```

### Rule: handoff は CLI 経由で thread を参照する指示文を返す

```gherkin
Scenario: thread の handoff を生成する
  Given thread がある
  When その thread の handoff を生成する
  Then thread の内容確認を促すメッセージが返る
  And メッセージには現在の build profile に応じた CLI 名（releash / releash-dev）での
       review get 呼び出しが含まれる
  And CLI 名はハードコードされず build profile から解決される

Scenario: 存在しない thread の handoff は拒否される
  When 存在しない thread_id の handoff を生成する
  Then NotFound エラーになる
```

---

## Feature: 入力 validation

domain は不正な入力を受け付けず、副作用（永続化・通知）を起こす前に拒否する。

### Rule: content は非空かつサイズ上限内である

```gherkin
Scenario Outline: content の validation
  When content "<content>" で thread を作成 / 追記する
  Then 結果は <result> となる

  Examples:
    | content                  | result            |
    | 有効な本文               | 成功              |
    | （空文字列）             | InvalidInput      |
    | （空白のみ）             | InvalidInput      |
    | NUL byte を含む文字列    | InvalidInput      |
    | 65536 bytes 超の文字列   | InvalidInput      |
```

### Rule: file path は repo 相対の安全なパスである

```gherkin
Scenario Outline: file path の validation
  When file path "<path>" で thread を作成する
  Then 結果は <result> となる

  Examples:
    | path              | result        |
    | src/lib.rs        | 成功          |
    | （絶対パス）      | InvalidInput  |
    | ../outside        | InvalidInput  |
    | path/../traversal | InvalidInput  |
    | back\slash        | InvalidInput  |
    | NUL を含むパス    | InvalidInput  |
    | 4096 bytes 超     | InvalidInput  |
```

### Rule: 行指定は 1-indexed で範囲が整合する

```gherkin
Scenario Outline: 行指定の validation
  When line_number=<line>, end_line=<end> で thread を作成する
  Then 結果は <result> となる

  Examples:
    | line | end  | result        |
    | 1    | 5    | 成功          |
    | 1    | なし | 成功          |
    | なし | なし | 成功（general）|
    | 0    | なし | InvalidInput  |
    | なし | 5    | InvalidInput  |
    | 5    | 1    | InvalidInput  |
```

---

## Feature: 永続化フォーマットの不変性

state file のレイアウトと event JSON の構造は移行前後で変わらない。これは file を直接読む外部（別プロセス・移行前データ）との互換性契約である。

### Rule: event は worktree 単位の state file へ追記される

```gherkin
Scenario: 書き込みは worktree 単位の state file に永続化される
  When ある worktree の thread に対して書き込み操作を行う
  Then その worktree に対応する state file へ event が追記される
  And state file 名は worktree の解決パスから決定論的に導かれる

Scenario: event JSON の構造が保たれる
  When thread を作成・追記・解決・削除する
  Then 各 event は eventType / eventId / threadId / actor / at を含む既存スキーマで記録される
  And actor には participant 識別情報と既存スキーマの sessionId が記録される
```

### Rule: 書き込みは worktree 単位で排他され、原子的に置換される

```gherkin
Scenario: 並行する書き込みは worktree 単位で直列化される
  Given 同一 worktree に対して複数の書き込みが同時に発生する
  When それぞれが書き込みを試みる
  Then 書き込みは worktree 単位の lock により直列化され、event が失われない

Scenario: 書き込みは原子的に置換される
  When state file を更新する
  Then 一時ファイルへ書き出した後に原子的置換で反映される
  And 置換失敗時も元の state file は壊れない
```

### Rule: 欠損・破損 state file を安全に扱う

```gherkin
Scenario: state file が存在しない worktree を一覧する
  Given state file がまだ存在しない worktree がある
  When その worktree の thread を一覧する
  Then 空の一覧が返る（エラーにはならない）

Scenario: state file が破損している
  Given JSON として読めない state file がある
  When その worktree の thread を読み取る
  Then 既存実装と同じエラー挙動になる（黙って空にはしない）
```

---

## Feature: CLI `releash review` の出力契約

`releash review` サブコマンド（list / get / create / comment / resolve / history）の標準出力・終了コードは移行前後で変わらない。

### Rule: `--json` 指定時は機械可読 JSON を、未指定時は人間向け整形を出力する

```gherkin
Scenario: list を人間向けに出力する
  When `releash review list` を --json なしで実行する
  Then thread ごとに THREAD_ID / STATE / AUTHOR / UPDATED が桁揃えで出力される
  And thread が無い場合は "(no review threads)" が出力される

Scenario: list を JSON で出力する
  When `releash review list --json` を実行する
  Then thread 一覧が JSON として出力される

Scenario: get / create / comment / resolve を人間向けに出力する
  When 対象サブコマンドを --json なしで実行する
  Then thread_id / state / author / location / updated / comments が整形出力される
  And resolved な thread では resolve 行（outcome / actor / summary）も出力される

Scenario: history を人間向けに出力する
  When `releash review history` を --json なしで実行する
  Then event が時系列順に行単位で出力される
  And 履歴が無い場合は "(no review history)" が出力される
```

### Rule: CLI は失敗種別に応じた終了コードを返す

```gherkin
Scenario Outline: CLI の終了コード
  When review サブコマンドが <失敗種別> で失敗する
  Then 終了コードは <code> になる

  Examples:
    | 失敗種別                     | code |
    | 成功                         | 0    |
    | InvalidInput（入力不正）     | 2    |
    | NotFound（不在）             | 4    |
    | その他（I/O / serialize 等） | 1    |
```

### Rule: CLI は新しい usecase 境界経由で動作する

```gherkin
Scenario: CLI review コマンドが usecase 境界経由で動作する
  When `releash review` の各サブコマンドを実行する
  Then 移行前と同じ入力契約・出力・終了コードで結果が得られる
  And この振る舞いは内部実装が crate::review_comments へ直接依存しなくても保たれる
```

---

## 仮定

- A1: 本 `behavior.md` は新規振る舞いではなく、移行前の `review_comments` 実装が示す観測可能な契約を記述する。Scenario の期待値は実装ではなく既存仕様を正とし、移行で値を変えない（requirements A5）。
- A2: actor の participant 識別は `human` / `agent:{backend_id}:{model}` 形式で、session_id を含まない（unread / author フィルタ / public projection に共通する観測可能ルール）。永続化 event の private actor は移行前の schema 互換として optional `sessionId` フィールドを保持する。
- A3: 永続化フォーマット関連の Scenario（state file 名の導出規則、event JSON のフィールド名）は「既存と同一であること」を要求する契約であり、具体的なファイル名規則やフィールド名の文字列は design.md / 実装側を正本とする。behavior.md では「決定論的に導かれる」「既存スキーマで記録される」のレベルに留める。
- A4: 破損 state file の具体的エラー種別は既存実装の挙動に従う（requirements R9 の非退行範囲）。behavior.md は「黙って空一覧にフォールバックしない」点のみを契約として固定する。
- A5: 層配置・モジュール削除・port 化（R1〜R8）は外部観測対象でないため Gherkin に含めない。これらは `cargo test` / clippy / ディレクトリ構成確認で担保する（requirements の受け入れ基準参照）。

## Open Questions

なし。
