## B-001: delegate 親 Session を経由する作業場所の解決

GIVEN delegate の child 部分木にある Session Node が実行されている
AND その Node と祖先に個別の worktree 指定がない
WHEN その Session の作業場所を読み取る
THEN 実行木の作業場所が返る
AND 祖先が合成子でないことを理由とする読み取りエラーにならない

## B-002: 読み側と実行側の作業場所の一致

GIVEN delegate の child 部分木にある Session Node が実行されている
WHEN その Session の作業場所を読み取る
THEN その Session の起動に使われた作業場所と同じ値が返る

## B-003: delegate 部分木の Session 通知の受理

GIVEN delegate の child 部分木にある Session Node が provider session を持つ
WHEN その Session の開始通知または終了通知を受け取る
THEN 通知は受理される
AND 対応する事実が記録される

## B-004: 提出と終了が揃ったレビューの完了

GIVEN delegate の child 部分木の Fanout 配下にある Session が結果を提出している
WHEN それぞれの Session の終了が記録される
THEN 各 Session Node は完了する
AND Fanout の後続 Node が開始する

## B-005: Sequence / Fanout 配下と隔離継承の維持

GIVEN Sequence または Fanout だけを祖先に持つ Session Node が実行されている
WHEN その Session の作業場所を読み取る
THEN 変更前と同じ作業場所が返る
AND 祖先に隔離 worktree がある場合は、最も近い隔離祖先の worktree が返る

## B-006: 破損した祖先関係の拒否

GIVEN 祖先関係が循環している、または別の実行木の Node を祖先に含む事実が保存されている
WHEN その Node の作業場所を読み取る
THEN 読み取りは失敗する
AND 作業場所は返らない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
