## B-001: 起動時にプロセスの喪失が記録されない

GIVEN プロセスを起動した記録があり、終わっていない Node を持つ実行木がある
WHEN アプリケーションを再起動する
THEN その Node に、プロセスを失ったことを表す事実は追加されない

## B-002: 記録が先に進んだ実行木への操作が受け付けられる

GIVEN 実行木の最新の事実が、エンジンが直前に読んだ内容より進んでいる
WHEN 利用者が承認、Retry、または Session の Resume を行う
THEN 可否は最新の事実に基づいて判定される
AND 実行木がメモリ上の一覧に無いこと、またはメモリ上の一覧が古いことを理由に拒否されない

## B-003: 再開した Session Node への Submit

GIVEN 終わっていない Session Node のプロセスが居ない状態で、利用者がその Session を再開した
WHEN その Node へ Submit する
THEN Submit は受け付けられる
AND submit の事実が記録される

## B-004: 再開した Session Node への provider の Stop

GIVEN 終わっていない Session Node のプロセスが居ない状態で、利用者がその Session を再開した
WHEN その Node に対する provider の Stop が届く
THEN Stop は受け付けられる
AND stop の事実が記録される

## B-005: 途切れた前進が起動時に続けられる

GIVEN 実行中の Node が 1 つも無く、次の Node を起動する前に記録が途切れた実行木がある
WHEN アプリケーションを起動する
THEN その実行木の次の Node が起動する

## B-006: 起動時の前進の失敗

GIVEN 起動時の前進が失敗する実行木がある
WHEN アプリケーションを起動する
THEN その実行木に対する前進は繰り返されない
AND その実行木は失敗の理由を伴って Aborted になる

## B-007: 1 つの実行木の失敗が他へ波及しない

GIVEN 起動時の前進が失敗する実行木と、前進できる別の実行木がある
WHEN アプリケーションを起動する
THEN 前進できる実行木の次の Node が起動する

## B-008: worktree の排他が記録から計算される

GIVEN ある worktree に、実行中の workflow の実行木が記録上 1 件ある
WHEN その worktree で別の workflow を起動する
THEN 起動は拒否される
AND 直前にアプリケーションを再起動していても、同じ結果になる

## B-009: 同一 worktree への同時起動

GIVEN ある worktree に対する複数の起動要求が同時に行われる
WHEN それらが処理される
THEN 成功するのは 1 件だけである

## B-010: 起動を拒否したときのエラー

GIVEN ある worktree で workflow の実行木が実行中である
WHEN その worktree で別の workflow を起動し、排他によって拒否される
THEN エラーは、その worktree を塞いでいる実行を識別できる情報を含む
AND エラーは、排他の単位が worktree であることを示す

## B-011: 反映できない command の結果

GIVEN command の結果が届いた時点で、記録の最新の状態では対象の Node が既に確定している
WHEN その結果を反映しようとする
THEN 実行木の記録は変わらない
AND 反映しなかったことと理由がログに記録される

## B-012: 終了時に止める command の対象

GIVEN 実行中の command があり、その実行木がエンジンのメモリ上の一覧に載っていない
WHEN アプリケーションを終了する
THEN その command は停止する

## B-013: 定義を解釈できない実行の起動時の Abort

GIVEN 完了または Abort の事実が無く、保存定義を現行コードで解釈できない実行木がある
WHEN アプリケーションを起動する
THEN その実行木は定義を解釈できない理由を伴って Aborted になる

## B-014: メモリ上の一覧に無い実行木への command の結果

GIVEN command が走っている実行木が、エンジンのメモリ上の一覧に載っていない
WHEN その command が終了する
THEN 結果が対象の実行木へ反映され、対応する事実が記録される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003, B-004 |
| R-003 | B-003, B-004 |
| R-004 | B-005, B-013 |
| R-005 | B-006 |
| R-006 | B-007 |
| R-007 | B-008 |
| R-008 | B-009 |
| R-009 | B-010 |
| R-010 | B-011, B-014 |
| R-011 | B-012 |
