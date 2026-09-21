## B-001: 実行木の操作は Abort だけである

GIVEN 終わっていない実行木がある
WHEN 利用者が画面・CLI・local API・Connect からその実行木に行える操作を確認する
THEN 実行の状態を変える操作として提示されるのは Abort だけである
AND 実行木を止める操作と実行木を再開する操作は、どの入口にも無い

## B-002: フォルダが無い worktree の実行木も Abort できる

GIVEN 終わっていない実行木があり、その実行 worktree のフォルダが存在しない
WHEN 利用者がその実行木を Abort する
THEN 実行木は Aborted になる

## B-003: 一度も起動できなかった Session Node の Resume

GIVEN 終わっていない Session Node が、provider session を一度も持てていない
WHEN 利用者がその Session Node を Resume する
THEN 新しい attempt として provider session が起動する
AND その Node の最初の指示が送られる

## B-004: 隔離 worktree の実体が失われた Session Node の Resume

GIVEN Session Node が隔離 worktree で動いており、その worktree の実体が失われている
WHEN 利用者がその Session Node を Resume する
THEN 新しい attempt の隔離 worktree が作られ、その中で provider session が起動する
AND その Node の最初の指示が送られる

## B-005: 会話が残っている Session Node の Resume

GIVEN 終わっていない Session Node の provider のプロセスが居らず、再開できる会話が残っている
WHEN 利用者がその Session Node を Resume する
THEN provider session が既存の会話のまま再開する
AND Releash から会話の続きを促す指示は送られない

## B-006: Session Node に手動 Retry が無い

Session Node には、利用者が手動で Retry する操作が提示されない。

## B-007: 未注入の child の結果は Session の Resume で渡る

GIVEN delegate の親 Session Node に、まだ親へ渡していない child の結果がある
WHEN 利用者がその親 Session Node を Resume する
THEN 未注入の child の結果が親 Session へ渡る
AND 新しい attempt での Resume でも、未注入の結果は一度だけ渡る

## B-008: 欠番

## B-009: プロセスが居ない Command Node の Retry

GIVEN 終わっていない Command Node の、その command のプロセスが居ない
WHEN 利用者がその Node に行える操作を確認する
THEN Retry が提示される

## B-010: 終わっていない Node は実行中である

GIVEN 終わっていない Session Node の provider のプロセスが終了した
WHEN 実行木の状態を読む
THEN その Node の状態は実行中である
AND Node の状態が取りうる値に、中断して再開を待つことを表す値と、失敗を表す値は含まれない

## B-011: プロセスの在否が状態と別に読める

GIVEN 実行木に、終わっていない Session Node と Command Node がある
WHEN 実行木の状態を読む
THEN 各 Node について、状態とは別にプロセスが居るかどうかが読める

## B-012: プロセスの異常終了も起動の失敗も Node の失敗にならない

GIVEN Session Node の provider のプロセスが異常終了した、または Session Node / Command Node の起動が失敗した
WHEN 実行木の状態を読む
THEN その Node の状態は失敗ではない

## B-013: 完了信号の偏りは Retry の理由にならない

GIVEN 終わっていない Node が、Submit と provider Stop のうち片方だけを受け取っている
WHEN 利用者がその Node に行える操作を確認する
THEN その完了信号の偏りを理由とする Retry は提示されない

## B-014: 取り除いた proto の番号と名前は再利用されない

GIVEN proto から取り除いた RPC、message、oneof の項目、field がある
WHEN 同じ service、message、oneof へ新しい定義を追加する
THEN 取り除いた番号と名前は使えない

## B-015: 失敗時の扱いを宣言する構文は受け付けられない

GIVEN children エントリに、child が失敗したときの扱い（自動のやり直し、失敗の無視）を宣言した workflow 定義がある
WHEN その定義を読み込む
THEN 定義の誤りとして拒否される

## B-016: 正本とガイドが変更後の操作・状態・構文に一致する

`docs/glossary/DOMAIN.md`、`docs/glossary/WORKFLOW.md`、`docs/guide/workflow/concepts.md`、`docs/guide/workflow/yaml.md`、`docs/guide/workflow/common.md`、`docs/guide/workflow/lua.md` は、利用者が行える操作として実行木を止める操作と再開する操作を挙げず、Node が持つ状態として中断して再開を待つ値と失敗を表す値を挙げず、child が失敗したときの扱いを宣言する構文を挙げない。

## B-017: 起動が失敗した Node は自動で起動し直される

GIVEN Session Node または Command Node がある
WHEN その Node の起動が失敗する
THEN 規定回数まで自動で起動し直される
AND 起動し直しは 1 回ごとに新しい attempt を作る
AND 試行の間隔は回を追うごとに長くなる
AND 起動を発生させた操作の応答は、自動再試行の待機・完了を待たない
AND Abort またはアプリケーションの終了時は待機を打ち切る

## B-018: 規定回数を使い切った Node は利用者の操作を待つ

GIVEN 起動の失敗が規定回数繰り返された Node がある
WHEN 実行木の状態とその Node に行える操作を読む
THEN その Node の状態は実行中である
AND その Node のプロセスが居ないことが読める
AND Session Node には Resume が、Command Node には Retry が提示される

## B-019: プロセスが居ない Node は介入待ちとして読める

GIVEN 終わっていない Node の、その Node のプロセスが居ない
WHEN 実行木の状態分類を読む
THEN その Node は利用者の介入を待つものとして読める

## B-020: 0 以外の終了コードで終わった Command は完了する

GIVEN Command Node の command が 0 以外の終了コードで終わった
WHEN 実行木の状態とその Node の Artifact を読む
THEN その Node は完了している
AND その Artifact の `ok` は false である

## B-021: child 実行中に親を新しい attempt で Resume する

GIVEN delegate の child が実行中で、親 Session Node は新しい attempt での Resume を必要とする
WHEN 利用者が親 Session Node を Resume する
THEN 未完了の委任ラウンドの child、発火回数、提出 Artifact と完了信号を新しい親 attempt が引き継ぐ
AND child の記録された親IDと worktree の継承先は変わらない
AND child の完了と再試行は、最新の親 attempt の継続状態を更新する
AND 親の複数回の Resume と事実ログからの復元でも同じ結果になる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-004, B-005, B-018, B-021 |
| R-004 | B-006 |
| R-005 | B-003, B-005 |
| R-006 | B-007, B-021 |
| R-007 | B-009, B-018 |
| R-008 | B-010, B-012 |
| R-009 | B-009, B-011, B-018 |
| R-011 | B-013 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-014 | B-016 |
| R-015 | B-017, B-018 |
| R-016 | B-019 |
| R-017 | B-020 |
