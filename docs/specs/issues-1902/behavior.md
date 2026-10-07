## B-001: `releash` で届く先

GIVEN Releash がインストールされ、サーバが起動している
WHEN 利用者が `/usr/local/bin/releash`、agent が `{data_dir}/bin/releash`（dev は `releash-dev`）、または `.app` に同梱された CLI を実行する
THEN どれもサーバとは別の実行ファイル `releash` が動く
AND `.app` には `releash` とサーバの実行ファイルの両方が含まれる

## B-002: CLI の crate の依存

GIVEN CLI の crate `releash` の依存の一覧
WHEN 依存の木を調べる
THEN サーバの crate（`releash-backend`）は含まれない

## B-003: 互換なサーバへの呼び出し

GIVEN サーバが起動していて、`GetServerInfo` の protocol が CLI の protocol と同じである
WHEN 利用者が CLI のコマンドを実行する
THEN コマンドはサーバを呼び出して結果を出す

## B-004: 互換でないサーバ

GIVEN サーバの protocol が CLI の protocol と異なる
WHEN 利用者が CLI のコマンドを実行する
THEN コマンドはサーバの操作を呼び出さずに終了コード 1 で失敗する
AND エラーの code は `failed_precondition` で、メッセージにサーバと CLI のどちらが古いかと双方の release が含まれる

## B-005: サーバに届かない

GIVEN 発見ファイル `client-api.json` が無い、指すプロセスが居ない、同一性が合わない、壊れている、または接続できない
WHEN 利用者が `workflow status`、`workflow output get`、`review list` など任意のコマンドを実行する
THEN コマンドは終了コード 1 で失敗し、エラーの code は `unavailable` である
AND data dir の記録は読み書きされない

## B-006: 読み取りのコマンド

GIVEN サーバが起動していて、指定した実行・Session・Thread がある
WHEN 利用者が `workflow status <id>`、`workflow output get <id> --node <node>`、`review list`、`review get <thread> --session-id <session>`、`review history <thread> --session-id <session>` を実行する
THEN サーバが持つその時点の状態が表示され、終了コード 0 で終わる

## B-007: 読み取りの対象が無い

GIVEN サーバが起動していて、指定した実行・Session・Thread が無い
WHEN 利用者が読み取りのコマンドを実行する
THEN コマンドは終了コード 1 で失敗し、エラーの code は `not_found` である

## B-008: 変更と診断のコマンド

GIVEN サーバが起動していて、open な Session と実行中の node-execution がある
WHEN agent が `workflow output submit`、`review create`、`review comment`、`review resolve`、または利用者が `workflow diagnostics` を実行する
THEN サーバで変更または診断が行われ、結果が表示される
AND review の書き手はその Session の agent として記録される

## B-009: review list（Session を指定）

GIVEN Session があり、その worktree に Thread がある
WHEN agent が `review list --session-id <session> --state open --author self --unread true` を実行する
THEN その Session の worktree の Thread のうち、サーバが絞り込んだものが表示される

## B-010: review list（Session を指定しない）

GIVEN `RELEASH_WORKTREE_PATH` が Node の隔離 worktree の path を指している
WHEN Command Node が `review list --state open --json` を実行する
THEN その path が属する workspace の worktree の Thread が表示される

## B-011: `--author`・`--unread` は `--session-id` を要る

GIVEN `--session-id` を指定していない
WHEN 利用者が `review list --author self` または `review list --unread true` を実行する
THEN コマンドは終了コード 2 で失敗する

## B-012: 値の妥当性はサーバが判定する

GIVEN サーバが起動している
WHEN 利用者が形の不正な execution ID、`--state` の不正な値、または存在しない `--dir` を指定してコマンドを実行する
THEN コマンドは終了コード 1 で失敗し、エラーの code はサーバが返したもの（`invalid_argument` または `not_found`）である

## B-013: `--json` の形

GIVEN 変更前の CLI の `--json` の出力
WHEN 変更後の CLI で同じ状態に同じコマンドを `--json` 付きで実行する
THEN キー名と値は変更前と同じである
AND `workflow output get --json` は `status` と、提出済みなら `contract`・`artifact`（例: `.artifact.tasks`）を持つ
AND `review list --json` は Thread の配列で、各要素は `id`・`state`・`author`・`target`・`comments` を持つ
AND `workflow status --json` は `id`・`workflowName`・`status`・`currentNode` を持つ

## B-014: 人向けの表示

GIVEN サーバが起動している
WHEN 利用者が `--json` を付けずに読み取りのコマンドを実行する
THEN 人向けの表示が出る
AND `review history` の表示にプログラム内部の型の表現は出ない

## B-015: エラーの形

GIVEN コマンドが失敗する状態
WHEN 利用者が `--json` 付きでコマンドを実行する
THEN stderr に `{"error":{"code":"<code>","message":"<message>"}}` が出る
AND `code` は Connect の標準コード名である

## B-016: 診断が error を検出した

GIVEN `--dir` の workflow 定義に severity error の診断がある
WHEN 利用者が `workflow diagnostics --dir <PATH>` を実行する
THEN 診断の結果が表示され、終了コード 3 で終わる

## B-017: 補完スクリプト

WHEN 利用者が `releash completion zsh` を実行する
THEN zsh の補完スクリプトが stdout に出て、終了コード 0 で終わる

## B-018: data dir の指定

GIVEN `RELEASH_DATA_DIR` が data dir A を、`--data-dir` が data dir B を指し、B のサーバが起動している
WHEN 利用者が `releash --data-dir B workflow status <id>` を実行する
THEN B のサーバの状態が表示される

## B-019: hook の信号の送信

GIVEN サーバが起動していて、agent の env に hook の token と slot・binding・capability・agent session がある
WHEN provider が `releash hook receive --provider claude` を payload 付きで起動する
THEN payload は解釈されずにサーバへ送られ、サーバが処理する
AND hook は stdout に `{}` を出して exit 0 で終わる
AND health ファイルは読み書きされない

## B-020: hook の失敗

GIVEN サーバに届かない、サーバが互換でない、payload が 65,536 byte を超える、またはサーバが信号を拒否する
WHEN provider が `releash hook receive` を起動する
THEN hook は stderr に失敗を出し、exit 0 で終わる
AND payload が上限を超えるときと互換でないときは、サーバへ送られない

## B-022: Tauri アプリ

GIVEN サーバが `client-api.json` だけを書く
WHEN 利用者が Tauri アプリを起動する
THEN アプリはサーバを起動し、接続して画面を表示する

## B-023: `render_long_help()`

GIVEN 変更後のコード
WHEN CLI の help を組み立てる関数を探す
THEN 本番から呼ばれない `render_long_help()` は無い

## B-024: CI

GIVEN CLI と共有 crate を変更した PR
WHEN CI が走る
THEN CLI と共有 crate の fmt・clippy・単体テストと、CLI の統合テストが、crate ごとの job の組で検査される

## B-025: 文書

GIVEN AGENTS.md、`docs/guide/cli.md`、`docs/guide/workflow/`
WHEN CLI の構成・終了コード・エラーの形・サーバ未起動時の挙動の記述を読む
THEN 記述は変更後のコードと一致し、HTTP `/v1`・終了コード 4・アプリ未起動時のファイルの直読みには触れていない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-001 |
| R-003 | B-003 |
| R-004 | B-003, B-004 |
| R-005 | B-005 |
| R-006 | B-006, B-007 |
| R-007 | B-008 |
| R-008 | B-009, B-010, B-011 |
| R-009 | B-012 |
| R-010 | B-013 |
| R-011 | B-014 |
| R-012 | B-015 |
| R-013 | B-011, B-012, B-016 |
| R-014 | B-017 |
| R-015 | B-018 |
| R-016 | B-019, B-020 |
| R-017 | B-022 |
| R-018 | B-022 |
| R-019 | B-023 |
| R-020 | B-024 |
| R-021 | B-025 |
