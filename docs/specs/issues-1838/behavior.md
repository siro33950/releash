## B-001: Workflow 実行の状態は3値である

GIVEN 任意の Workflow 実行
WHEN CLI または Connect API でその状態を取得する
THEN 状態は `running` / `completed` / `aborted` のいずれかである
AND 状態が取りうる値の定義に `waiting_approval` と `interrupted` は含まれない

## B-002: Workflow 実行の状態は中断中・承認待ちへ変わらない

GIVEN 実行中の Workflow 実行
WHEN Workflow 実行の状態が変わる
THEN 変わり先は完了または Abort だけである
AND Workflow 実行の状態を中断中・承認待ちにし、そこから再開する遷移は存在しない

## B-003: 中断理由と再開位置が読み取り結果に現れない

GIVEN 任意の Workflow 実行
WHEN CLI の `--json` 出力または Connect API の応答を受け取る
THEN 中断理由（`interruptionReason`）と再開位置（`resumeFromNode`）のキーは含まれない

## B-004: 取り除いた proto の番号と名前は再利用されない

GIVEN proto から取り除いた項目または列挙値がある
WHEN 同じ message または enum へ新しい定義を追加する
THEN 取り除いた番号と名前は使えない

## B-005: Workflow 実行の状態はファイルへ保存されない

GIVEN Workflow を起動し、完了または Abort させる
WHEN アプリケーションのデータディレクトリを確認する
THEN Workflow 実行の状態を保持するファイルは作られていない

## B-006: 到達しない実装が残らない

Workflow 実行ストアは、本番の実行経路から呼ばれないメソッドを持たない。Workflow ホストは、値が書き込まれることのない実行時状態を持たない。

## B-007: コメントが存在しない記録先を実装として説明しない

コメントは、読み書きするコードが存在しない記録先を、現在の実装として説明しない。

## B-008: CLI ガイドが本番で出ない値を載せない

GIVEN `docs/guide/cli.md` の `releash workflow status` の節
WHEN 出力の説明を読む
THEN 記載されている Workflow 実行の状態値は `running` / `completed` / `aborted` だけである
AND 中断理由と再開位置の項目は記載されていない

## B-009: 本番の振る舞いと出力値が変わらない

GIVEN 変更前と同じ workflow 定義と worktree
WHEN Workflow を起動し、完了または Abort させる
THEN 実行の進行、記録される事実、状態の遷移は変更前と同じである
AND 変更前に値を持って出力されていた項目は、同じ値を出力する

## B-010: Workflow 実行ストアの責務の説明が実体と一致する

Workflow 実行ストアの責務を説明するコメントは、そのストアが提供しない一覧取得の管理と、取り除いたファイル永続化の仕組みを、現在の責務として説明しない。

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-006 |
| R-008 | B-007 |
| R-009 | B-008 |
| R-010 | B-009 |
| R-011 | B-010 |
