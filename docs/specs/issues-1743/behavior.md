## B-001: Command node の Command 実行結果

GIVEN Command node が Artifact を産出して完了している
AND その Artifact に exit code、duration、stdout、stderr が揃っている
WHEN その node を Workspace ツリーから読み出す
THEN その node は Command 実行結果として、Artifact が持つ exit code、duration、stdout、stderr を持つ
AND Artifact を産出したことを表す表示が真になる

## B-002: Command node の Artifact に Command 実行結果の要素が欠ける場合

GIVEN Command node が Artifact を産出して完了している
AND その Artifact に exit code、duration、stdout、stderr のいずれかが欠けている
WHEN その node を Workspace ツリーから読み出す
THEN その node は Command 実行結果を持たない
AND Artifact を産出したことを表す表示が真になる

## B-003: Sequence node の Artifact が Command 実行結果と同じ形の場合

GIVEN Sequence node が完了している
AND その Artifact が exit code、duration、stdout、stderr を揃えた値である
WHEN その workspace の Workspace ツリーを読み出す
THEN 読み出しは成功し、corrupt_stored_state にならない

## B-004: Fanout node の Artifact が Command 実行結果と同じ形の場合

GIVEN Fanout node が完了している
AND その Artifact が exit code、duration、stdout、stderr を揃えた値である
WHEN その workspace の Workspace ツリーを読み出す
THEN 読み出しは成功し、corrupt_stored_state にならない

## B-005: Session node が Command 実行結果と同じ形の Artifact を submit した場合

GIVEN Session node の Contract が exit code、duration、stdout、stderr を宣言している
AND その Session node が4つを揃えた Artifact を submit して完了している
WHEN その workspace の Workspace ツリーを読み出す
THEN 読み出しは成功し、corrupt_stored_state にならない

## B-006: 非 Command node の Artifact の形が同じ workspace の他の実行木に波及しない

GIVEN 同じ workspace に複数の実行木がある
AND そのうち1つの実行木に、Command 実行結果と同じ形の Artifact を持つ非 Command node が含まれる
WHEN その workspace の Workspace ツリーを読み出す
THEN 読み出しは成功し、corrupt_stored_state にならない
AND その workspace の全実行木のノードが読める

## B-007: 非 Command node を含む workspace の node 単体読み出し

GIVEN 非 Command node が Command 実行結果と同じ形の Artifact を持つ
WHEN その node、または同じ workspace の別の node を単体で読み出す
THEN 読み出しは成功し、corrupt_stored_state にならない

## B-008: 非 Command node の Artifact 産出表示

GIVEN 非 Command node が Artifact を産出して完了している
WHEN その node を Workspace ツリーから読み出す
THEN Artifact を産出したことを表す表示が真になる

## B-009: 保存済み fact log からの再構築

GIVEN Command 実行結果と同じ形の Artifact を持つ非 Command node を含む実行の fact log が保存済みである
WHEN その workspace の Workspace ツリーを読み出す
THEN 事前のデータ移行なしに読み出しは成功する
AND 保存済みの fact log は書き換えられない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-001, B-002 |
| R-003 | B-003, B-004, B-005, B-006, B-007 |
| R-004 | B-001, B-002, B-008 |
| R-005 | B-009 |
