# Releash CLI

`releash` CLI は、workflow の状態を読み、Node の Artifact を提出し、review の Thread を操作するための入口です。人間が terminal から使う場合と、agent が instruction に従って呼ぶ場合の両方を対象にします。

## 前提

### インストール

- Releash の Settings を開き、Background の「Install CLI command」を選ぶと、`/usr/local/bin/releash` に同梱の `releash-backend` への symlink を設置します。起動時には設置しません。
  - 書き込み権限が無い場合は、管理者権限を求めるダイアログが出ます。
  - アプリが translocate された状態（ダウンロード直後の隔離状態など）で起動した場合は張りません。
  - `/usr/local/bin/releash` に symlink ではないファイルがある場合は上書きしません。
- Releash が起動する terminal（Session の provider TUI、Terminal パネルの shell）では、`{data_dir}/bin` が `PATH` の先頭に入り、そこに置かれた `releash` wrapper が優先されます。wrapper は `RELEASH_DATA_DIR` が未設定のときだけアプリの data dir を設定してから、アプリ本体を実行します。
- Command Node の `PATH` は異なります。[環境変数](#環境変数) を参照してください。

### 起動の分岐

- 引数なしの `releash`、または `releash --hidden` だけの場合は GUI が起動します。
- それ以外の引数があると CLI として動き、GUI は起動しません。
- CLI の説明は `releash --help` と各サブコマンドの `--help` で表示します。`-h` は要約表示です。
- `--version` はありません（指定すると引数エラーで終了コード 2）。

### data dir

- data dir は `RELEASH_DATA_DIR` で指定します。未設定または空文字の場合は `~/Library/Application Support/com.releash.app` です。
- 時刻を表す値は、すべて UNIX epoch からの秒数（小数）です。

## コマンド一覧

| コマンド | 内容 | アプリ起動 |
|---|---|---|
| [`workflow diagnostics`](#releash-workflow-diagnostics) | workflow 定義と Facet を診断する | 必須 |
| [`workflow status`](#releash-workflow-status) | WorkflowExecution の現在状態を表示する | 不要 |
| [`workflow output submit`](#releash-workflow-output-submit) | NodeExecution の Artifact または完了を提出する | 必須 |
| [`workflow output get`](#releash-workflow-output-get) | 提出済み Artifact を取得する | 不要 |
| [`review list`](#releash-review-list) | Thread 一覧を表示する | 不要 |
| [`review get`](#releash-review-get) | Thread 詳細を表示する | 不要 |
| [`review create`](#releash-review-create) | 初回 Comment とともに Thread を作成する | 不要 |
| [`review comment`](#releash-review-comment) | open Thread に Comment を追記する | 不要 |
| [`review resolve`](#releash-review-resolve) | open Thread を resolve する | 不要 |
| [`review history`](#releash-review-history) | Thread 履歴を表示する | 不要 |

結果は stdout に出ます。失敗時のメッセージは stderr に出ます。

### `releash workflow diagnostics`

```sh
releash workflow diagnostics [--dir <PATH>] [--json]
```

| 引数 | 説明 |
|---|---|
| `--dir <PATH>` | 診断対象の workflow source directory。相対 path は CLI を実行した directory 基準で解決する。Facet base は `<PATH>/facets` が directory ならそこ、無ければ `<PATH>`。省略時は適用済み Workflow の config directory |
| `--json` | 診断結果 JSON をそのまま出力する |

既定の出力は 1 診断 1 行で、空行のあとに件数を出します。位置と対象は、値がある場合だけ出ます。

```text
<severity> <code> <source>:<line>:<col> [workflow=<name>, node=<name>, facet=<kind>/<key>, field=<field>]: <message>

<n> error, <m> info
```

`--json` の shape（snake_case）:

| キー | 内容 |
|---|---|
| `items` | 診断の配列。要素は `code`、`severity`（`error` / `info`）、`stage`（`parse_shape` / `resolve` / `typecheck` / `control_flow`）、`message` と、値がある場合だけ `span`（`source`、`start_line`、`start_col`、`end_line`、`end_col`）、`workflow_name`、`node_name`、`facet_key`、`facet_kind`、`field` |
| `workflow_summaries` | workflow 名 → `{ "error_count", "info_count" }` |
| `facet_summaries` | `"<kind>/<key>"` → `{ "error_count", "info_count" }` |
| `facet_usage` | Facet key → 参照元 `{ "workflow_name", "node_name", "slot" }` の配列 |

終了コードは、severity `error` の診断が 1 件以上あれば 3、0 件なら 0 です。`--dir` が存在しない場合は 4 です。

### `releash workflow status`

```sh
releash workflow status <EXECUTION_ID> [--json]
```

| 引数 | 説明 |
|---|---|
| `<EXECUTION_ID>` | WorkflowExecution の id（UUID 形式。形式が不正なら終了コード 2） |
| `--json` | 現在状態を JSON で出力する |

既定の出力:

```text
execution_id:  <id>
workflow:      <workflow 名>
status:        <running | completed | aborted>
current_node:  <node 名。無ければ空>
updated_at:    <時刻>
input_tokens:  <数>
output_tokens: <数>
```

`--json` の shape（camelCase。列挙値は snake_case）:

| キー | 内容 |
|---|---|
| `id` | WorkflowExecution の id |
| `workflowName` | workflow 名 |
| `status` | `running` / `completed` / `aborted` |
| `currentNode` | 現在の node 名、または `null` |
| `worktreePath` | 実行対象の worktree path |
| `createdFrom` | `desktop_ui` / `cli` / `agent` / `api` |
| `startedAt` / `updatedAt` / `completedAt` | 時刻（`completedAt` は未完了なら `null`） |
| `errorReason` | 失敗理由、または `null` |
| `totalTokenUsage` | `{ "inputTokens", "outputTokens" }` |
| `nodeExecutions` | NodeExecution の一覧。id、node 名、kind、attempt、status、Session id、Artifact、失敗情報などを持つ |
| `artifacts` | 提出済み Artifact の一覧 |
| `fanouts` | Fanout ごとの親 NodeExecution と child NodeExecution の一覧 |
| `approvalTarget` | 承認待ちの NodeExecution、または `null` |

execution が見つからない場合は終了コード 4 です。

### `releash workflow output submit`

```sh
releash workflow output submit --node-execution <NODE_EXECUTION_ID>
releash workflow output submit --node-execution <NODE_EXECUTION_ID> --type <CONTRACT> --json <JSON>
releash workflow output submit --node-execution <NODE_EXECUTION_ID> --type <CONTRACT> --file <PATH>
```

| 引数 | 説明 |
|---|---|
| `--node-execution <NODE_EXECUTION_ID>` | 提出先の NodeExecution の id（必須） |
| `--type <CONTRACT>` | 提出する Artifact の contract。指定時は `--json` か `--file` のどちらか一つが必須 |
| `--json <JSON>` | 提出する Artifact の値（JSON 文字列）。`--type` が必須 |
| `--file <PATH>` | 提出する Artifact の値を読む JSON ファイル。`--type` が必須 |

- `--type` を付けない場合は、Artifact を持たない完了の提出になります。
- Session に渡る instruction の「完了時の必須アクション」に、この NodeExecution の id を埋めたコマンドが書かれています。Command Node では `RELEASH_NODE_EXECUTION_ID` に入っています。
- 成功時の出力は次の 1 行です。

```text
submitted: node_execution_id=<id> type=<contract>
submitted: node_execution_id=<id>
```

JSON の読み込みや parse に失敗した場合は終了コード 2 です。

### `releash workflow output get`

```sh
releash workflow output get <EXECUTION_ID> --node <NODE_NAME> [--json]
```

| 引数 | 説明 |
|---|---|
| `<EXECUTION_ID>` | WorkflowExecution の id（UUID 形式） |
| `--node <NODE_NAME>` | 対象の node 名（NodeExecution の id ではない） |
| `--json` | 結果を JSON で出力する |

既定の出力（`submitted_at` と `request_id` は値がある場合だけ出ます）:

```text
submitted: node=<node> contract=<contract。無ければ none>
submitted_at: <時刻>
request_id: <id>
timestamp: <時刻>
artifact:
<Artifact の値の JSON>
```

```text
not_submitted: node=<node>
```

`--json` の shape（snake_case）:

```json
{
  "status": "submitted",
  "contract": "review-verdict",
  "artifact": {
    "verdict": "LGTM"
  },
  "submitted_at": 1757740000.25,
  "request_id": "…",
  "timestamp": 1757740000.25
}
```

```json
{
  "status": "not_submitted"
}
```

`contract` は contract の無い Artifact では `null` です。`submitted_at` と `request_id` は値が無い場合キーごと出ません。

### review コマンド共通

- review コマンドはアプリを経由せず、data dir を直接読み書きします。data dir が存在しない場合は終了コード 4 です。
- `--session-id` には AgentSession の id を渡します。Session が属する workspace の worktree を解決し、その worktree の Thread を対象にします。隔離 worktree で動く Session でも、対象は workspace 側の worktree です。
- `--session-id` が空なら終了コード 2、Session が見つからなければ終了コード 4 です。
- `create` / `comment` / `resolve` は、lifecycle が open の Session だけを受け付けます（それ以外は終了コード 2）。`list` / `get` / `history` は paused / archived の Session でも使えます。
- `--content`、`--outcome`、`--summary` は空白だけにできず、NUL を含められず、65,536 bytes までです。

### `releash review list`

```sh
releash review list [--session-id <SESSION_ID>] [--file <FILE>] [--state <STATE>] \
  [--author <AUTHOR>] [--unread <UNREAD>] [--thread-id <THREAD_ID>]... [--json]
```

| 引数 | 説明 |
|---|---|
| `--session-id <SESSION_ID>` | 対象 worktree を解決する Session。省略時は `RELEASH_WORKTREE_PATH` を使う。どちらも無ければ終了コード 2 |
| `--file <FILE>` | Thread 対象の file path（repo 相対、完全一致） |
| `--state <STATE>` | `open` / `resolved` |
| `--author <AUTHOR>` | `self`（作成者が `--session-id` の Session と同じ participant）/ `other`（それ以外）。`--session-id` が必須 |
| `--unread <UNREAD>` | `true` / `false`。未読は「`--session-id` の Session と同じ participant の最後の Comment より後に、別 participant の Comment がある」こと。resolve は Comment に含めない。`--session-id` が必須 |
| `--thread-id <THREAD_ID>` | 指定 id の Thread に絞る。複数回指定でき、指定した id のどれかに一致すればよい |
| `--json` | Thread の配列を JSON で出力する |

participant は Session 単位ではなく、actor の kind・provider・model の組で決まります。CLI から作成した Comment の actor は model を持たないため、同じ provider の別 Session が CLI で作成した Thread や Comment も同じ participant として扱われます。

複数の絞り込みを指定した場合は、すべてを満たす Thread を返します。

既定の出力（Thread が無い場合は `(no review threads)`）:

```text
THREAD_ID                             STATE      AUTHOR                UPDATED
<id>                                  open       codex                 <時刻>
```

`--json` の要素は [Thread の JSON](#thread-の-json) です。

### `releash review get`

```sh
releash review get <THREAD_ID> --session-id <SESSION_ID> [--json]
```

既定の出力（`resolve:` 行は resolved の Thread だけに出ます。Comment 本文は `--json` でだけ出ます）:

```text
thread_id: <id>
state:     <Open | Resolved>
author:    <作成者の表示名>
location:  <file>:L<line>-L<end_line> | <file>:L<line> | <file> | (general)
updated:   <時刻>
comments:  <Comment 数>
resolve:   <outcome> by <resolve した actor の表示名> (<summary>)
```

`--json` は [Thread の JSON](#thread-の-json) です。Thread が見つからない場合は終了コード 4 です。

### `releash review create`

```sh
releash review create --session-id <SESSION_ID> --content <CONTENT> \
  [--file <FILE>] [--line <LINE>] [--end-line <END_LINE>] [--json]
```

| 引数 | 説明 |
|---|---|
| `--content <CONTENT>` | 初回 Comment の本文 |
| `--file <FILE>` | 対象 file。repo 相対で `/` 区切り、絶対 path・`.`・`..` を含めない、4,096 bytes まで。省略すると file を持たない Thread（`(general)`）になる |
| `--line <LINE>` | 開始行（1 以上） |
| `--end-line <END_LINE>` | 終了行（1 以上、`--line` 以上）。指定時は `--line` が必須 |

作成者は `--session-id` の Session の Agent です。出力は `review get` と同じ形式で、作成した Thread を表示します。

### `releash review comment`

```sh
releash review comment <THREAD_ID> --session-id <SESSION_ID> --content <CONTENT> [--json]
```

open Thread に `--session-id` の Session の Agent として Comment を追記します。resolved の Thread には追記できません（終了コード 2）。出力は `review get` と同じ形式です。

### `releash review resolve`

```sh
releash review resolve <THREAD_ID> --session-id <SESSION_ID> --outcome <OUTCOME> --summary <SUMMARY> [--json]
```

| 引数 | 説明 |
|---|---|
| `--outcome <OUTCOME>` | 解決状況を表す文字列（自由記述。例: `resolved`、`wontfix`、`duplicate`） |
| `--summary <SUMMARY>` | 対応内容の要約 |

`--session-id` の Session の Agent を resolve した actor として記録します。Thread の作成者以外の Session からも resolve できます。resolved の Thread は resolve できません（終了コード 2）。出力は `review get` と同じ形式です。

### `releash review history`

```sh
releash review history <THREAD_ID> --session-id <SESSION_ID> [--json]
```

既定の出力は履歴 1 件 1 行です（形式は [現状の不揃い](#現状の不揃い) を参照）。履歴が無い場合は `(no review history)` です。

`--json` の shape（camelCase の配列）。各要素は `kind` で種類を表します。

| `kind` | キー |
|---|---|
| `thread_created` | `id`、`threadId`、`commentId`、`actor`、`target`、`content`、`at` |
| `comment_appended` | `id`、`threadId`、`commentId`、`actor`、`content`、`at` |
| `thread_resolved` | `id`、`threadId`、`actor`、`outcome`、`summary`、`at` |
| `thread_deleted` | `id`、`threadId`、`actor`、`at` |

`actor` と `target` の形は [Thread の JSON](#thread-の-json) と同じです。

### Thread の JSON

`review list`（配列の要素）、`get`、`create`、`comment`、`resolve` の `--json` は次の形です（camelCase）。

```json
{
  "id": "5f0c…",
  "worktreeName": "/path/to/repo",
  "author": {
    "kind": "agent",
    "backendId": "codex",
    "model": null,
    "displayName": "codex"
  },
  "target": {
    "filePath": "src/main.rs",
    "lineNumber": 3,
    "endLine": 5
  },
  "state": "resolved",
  "comments": [
    {
      "id": "9a41…",
      "threadId": "5f0c…",
      "author": {
        "kind": "agent",
        "backendId": "codex",
        "model": null,
        "displayName": "codex"
      },
      "content": "Claim",
      "createdAt": 1757740000.25
    }
  ],
  "resolve": {
    "actor": {
      "kind": "agent",
      "backendId": "claude",
      "model": null,
      "displayName": "claude"
    },
    "outcome": "accepted",
    "summary": "done",
    "resolvedAt": 1757740100.5
  },
  "createdAt": 1757740000.25,
  "updatedAt": 1757740100.5,
  "version": 2,
  "canResolve": false
}
```

| キー | 内容 |
|---|---|
| `worktreeName` | Thread が属する workspace の worktree path |
| `author` / `actor` | `kind` は `agent` / `human`。人間の場合は `backendId` と `model` が `null`、`displayName` が `Human` |
| `target` | file を持たない Thread では 3 キーとも `null` |
| `state` | `open` / `resolved` |
| `resolve` | open の Thread では `null` |
| `version` | Thread に適用された履歴の件数 |
| `canResolve` | open なら `true` |

## 終了コード

| コード | 意味 |
|---|---|
| 0 | 成功。`--help` の表示を含む |
| 1 | その他の失敗。アプリ起動が必要な操作でアプリが起動していない、local API discovery file が不正または古い、local API の認証失敗、I/O や serialize の失敗など |
| 2 | 引数や入力が不正。構文エラー、UUID 形式や空文字の検証、JSON の parse 失敗、review の入力検証、resolved の Thread や open でない Session による拒否 |
| 3 | `workflow diagnostics` で severity `error` の診断が 1 件以上ある |
| 4 | 対象が見つからない。WorkflowExecution、Thread、Session、data dir、`--dir` の directory |

アプリの local API を経由するコマンドでは、local API が返した HTTP status を 404 → 4、400 / 409 / 422 → 2、401 とそれ以外 → 1 に変換します。

stderr のメッセージは、終了コード 4 ではメッセージだけ、1 と 2 では `error: <message>` です。構文エラーは clap の形式で出ます。

## アプリ未起動時の挙動

| コマンド | アプリ起動中 | アプリ未起動 |
|---|---|---|
| `workflow diagnostics` | local API 経由 | 終了コード 1（`この操作には Releash アプリの起動が必要です`） |
| `workflow status` | local API 経由 | data dir の event store を直接読む |
| `workflow output submit` | local API 経由 | 終了コード 1（`この操作には Releash アプリの起動が必要です`） |
| `workflow output get` | local API 経由 | data dir の event store を直接読む |
| `review` の全コマンド | data dir を直接読み書き | data dir を直接読み書き |

- 「アプリ未起動」と判定するのは、data dir に local API discovery file が無い場合です。アプリを正常に終了すると discovery file は削除されます。
- discovery file が残っていて、それが指すプロセスが存在しない、別のインスタンスを指している、接続できない、といった場合は、event store を直接読まずに終了コード 1 で失敗します。アプリが異常終了した直後に起こります。
- local API の認証に失敗した場合も、event store を直接読まずに終了コード 1 で失敗します。

## 環境変数

### CLI が読む環境変数

| 変数 | 対象 | 内容 |
|---|---|---|
| `RELEASH_DATA_DIR` | 全コマンド | data dir。未設定または空文字なら既定値 |
| `RELEASH_WORKTREE_PATH` | `review list` | `--session-id` を省略したときの対象 worktree |

### Releash が子プロセスに注入する環境変数

| 変数 | Session（provider TUI） | Terminal パネルの shell | Command Node |
|---|---|---|---|
| `PATH` | `{data_dir}/bin` を先頭に追加 | `{data_dir}/bin` を先頭に追加 | アプリのプロセスから継承（アプリ起動時に login shell から取り込んだ `PATH`）。`{data_dir}/bin` は入らないため、`releash` は `/usr/local/bin` の symlink で解決される |
| `RELEASH_DATA_DIR` | data dir | data dir | アプリのプロセスから継承した data dir |
| `RELEASH_SESSION_ID` | AgentSession の id | — | — |
| `RELEASH_BASE_BRANCH` | worktree の base branch（解決できた場合） | — | — |
| `RELEASH_WORKFLOW_EXECUTION_ID` | — | — | WorkflowExecution の id |
| `RELEASH_NODE_EXECUTION_ID` | — | — | NodeExecution の id |
| `RELEASH_WORKTREE_PATH` | — | — | Node が実行される worktree path |
| `RELEASH_PTY_ID` | terminal の id | terminal の id | — |
| `RELEASH_PROVIDER_LIFECYCLE_*` | provider hook 用 | — | — |

Command Node には、workflow 定義の `env` も加わります。

### agent からの使い方

CLI は `RELEASH_SESSION_ID` などを自動では読みません。instruction やコマンドで明示的に渡します。

Session から:

```sh
releash review list --session-id "$RELEASH_SESSION_ID" --state open --json
releash review create --session-id "$RELEASH_SESSION_ID" --content "根拠" --file src/main.rs --line 3 --json
```

Command Node から:

```sh
releash workflow status "$RELEASH_WORKFLOW_EXECUTION_ID" --json
releash review list --state open --json
```

2 行目は `--session-id` を省略しているため、`RELEASH_WORKTREE_PATH` の worktree が属する workspace の Thread を対象にします。

## 現状の不揃い

- `--json` のキー命名が揃っていません。`workflow status` と `review` の全コマンドは camelCase、`workflow output get` と `workflow diagnostics` は snake_case です。列挙値はどれも snake_case です。
- `workflow output submit` の `--json` は出力形式の指定ではなく、提出する Artifact の値の入力です。出力は常に `submitted: …` の 1 行です。
- `review history` の既定の出力は、内部の debug 表現（`ThreadCreated { id: "…", thread_id: "…", … }` のような形）です。機械処理には `--json` を使ってください。
- 既定の出力で、Thread の state の表記が揃っていません。`review list` は `open` / `resolved`、`review get` / `create` / `comment` / `resolve` は `Open` / `Resolved` です。

## `releash hook receive`

`releash hook receive --provider <claude | codex>` は、Releash が provider（Claude / Codex）に設定する hook から呼ばれる専用の入口です。`--help` の一覧には出ず、手動で使うものではありません。
