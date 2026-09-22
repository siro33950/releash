## B-001: 起動方法にかかわらず実行木を Archive できる

GIVEN Workflow として起動した実行木と、単独 Session として起動した実行木がある
WHEN それぞれの実行木に Archive を要求する
THEN どちらも同じ Archive 操作で Archive 済みになる
AND どちらも通常の Workspace tree には表示されない

## B-002: 実行中の実行木を Archive すると終了する

GIVEN 終了していない実行木がある
WHEN その実行木に Archive を要求する
THEN 実行木は Aborted になった後に Archive 済みになる
AND 実行中のプロセスは停止する
AND その実行木は同じ worktree の新しい実行を妨げない

## B-003: 終了済みの実行木も Archive できる

GIVEN Completed または Aborted の実行木がある
WHEN その実行木に Archive を要求する
THEN 終了状態を変えずに Archive 済みになる

## B-004: Abort できなかった実行木は Archive 済みにならない

GIVEN 終了していない実行木がある
WHEN Archive に伴う Abort が完了しない
THEN その実行木は Archive 済みにならない

## B-005: 手動 Archive は事実ログから読める

GIVEN Archive されていない実行木がある
WHEN 利用者がその実行木を Archive する
THEN 事実ログから Archive 済みであること、Archive 時刻、理由 `manual` が読める
AND Workspace tree の表示と Archive 履歴はその事実ログから導出される
AND `workflow_execution_archives.json` は Archive の読み書きに使われない

## B-006: 既存の Archive 記録を移行する

GIVEN `workflow_execution_archives.json` に既存の Archive 記録がある
WHEN 変更後の Releash が既存データを読み込む
THEN 対応する実行木の Archive 状態、Archive 時刻、Archive 理由が事実ログへ移される
AND 移行後も対応する実行木は Archive 済みとして読める
AND 終了していなかった実行木は Aborted である

## B-007: 画面は Archive 操作を常に提示する

GIVEN Archive されていない実行木が画面に表示されている
WHEN 利用者がその実行木に行える操作を確認する
THEN 実行木の状態にかかわらず Archive が提示される

## B-008: 画面は実行中の Archive だけを確認する

GIVEN Archive されていない実行木が画面に表示されている
WHEN 利用者が Archive を選ぶ
THEN 実行木が終了していなければ、Archive により Abort されることを確認してから Archive を実行する
AND 利用者が確認を取り消した場合は Abort も Archive も行わない
AND 実行木が終了済みなら確認を挟まず Archive を実行する

## B-009: 画面以外の Archive は確認を要求しない

GIVEN Archive されていない実行木がある
WHEN 画面以外の入口から Archive を要求する
THEN 確認用の追加入力を要求せず Archive を実行する

## B-010: Git の一覧にある worktree を GC は Archive しない

GIVEN worktree が Git の worktree 一覧に載っている
WHEN GC が実行される
THEN その worktree の実行木を Archive しない
AND worktree フォルダが存在しない場合も結果は変わらない

## B-011: Git の一覧から消えた worktree を GC が Archive する

GIVEN worktree が Git の worktree 一覧に載っていない
WHEN GC が実行される
THEN その worktree の実行木は理由 `worktree_removed` で Archive 済みになる
AND 終了していなかった実行木は Aborted になる
AND worktree フォルダが存在する場合も結果は変わらない

## B-012: リポジトリを読めない場合は GC が実行木を維持する

GIVEN 対象リポジトリを読めず Git の worktree 一覧を確定できない
WHEN GC が実行される
THEN そのリポジトリの実行木を Archive しない

## B-013: Releash 内の worktree 削除は実行木を先に片付ける

GIVEN Releash から削除する worktree に実行木がある
WHEN 利用者がその worktree の削除を要求する
THEN その worktree の実行木は理由 `worktree_removed` で Git worktree とフォルダの削除より先に Archive 済みになる
AND 終了していなかった実行木は Aborted になる
AND 実行中だった Command プロセスはフォルダの削除より先に停止する

## B-014: worktree フォルダが無くても Abort できる

GIVEN 終了していない実行木があり、その worktree フォルダが存在しない
WHEN その実行木を Abort する
THEN 実行木は Aborted になる

## B-015: Retry は worktree フォルダを必要とする

GIVEN 終了していない Command Node があり、その実行 worktree のフォルダが存在しない
WHEN 利用者がその Command Node を Retry する
THEN Retry は受理されない

## B-016: Archive 済みの実行木を Restore する

GIVEN 所属 worktree が利用可能な Archive 済みの実行木がある
WHEN 利用者がその実行木を Restore する
THEN 実行木は Archive 済みではなくなり、通常の Workspace tree に表示される
AND 実行木は Restore 前の終了状態を維持する
AND 実行木のプロセスは自動で起動しない

## B-017: Restore した単独 Session は手動で Resume する

GIVEN Restore 済みでプロセスが起動していない単独 Session がある
WHEN 利用者がその Session を Resume する
THEN Session のプロセスが起動する

## B-018: 削除開始後の外部状態変更を拒否する

GIVEN Releash による worktree の削除が始まっている
WHEN 利用者または外部インターフェースがその worktree に対する状態変更を要求する
THEN 状態変更は受理されない
AND その要求によって worktree と実行木の状態は変わらない

## B-019: 削除中の worktree を読み取れる

GIVEN Releash による worktree の削除が始まっている
WHEN 利用者または外部インターフェースがその worktree の状態を読み取る
THEN 削除処理の進行中も読み取り要求は受理される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006 |
| R-005 | B-007, B-008 |
| R-006 | B-009 |
| R-007 | B-010, B-011 |
| R-008 | B-011, B-012 |
| R-009 | B-013 |
| R-010 | B-014, B-015 |
| R-011 | B-016, B-017 |
| R-012 | B-011, B-013, B-018, B-019 |
