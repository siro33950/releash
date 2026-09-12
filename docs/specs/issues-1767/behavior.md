## B-001: Session handle への delegate 宣言

GIVEN `artifact` を宣言した Session と、Artifact を持つ別の Node が定義された Lua の workflow 定義
WHEN その Session の handle に `child` / `when` / `max_iterations` を与えた `delegate` を宣言して load する
THEN Error Diagnostic は出ない
AND 構築された定義でその Session は delegate を持つ

## B-002: inputs は任意

GIVEN Session handle への delegate 宣言
WHEN `inputs` を書かずに load する
THEN Error Diagnostic は出ない

## B-003: delegate の値の受理形

GIVEN Session handle への delegate 宣言
WHEN `child` に Node 値、`inputs` に `<パラメータ名> = <Source>` の table、`when` に Source または `r.all` / `r.any` が返す Predicate、`max_iterations` に1以上の整数を与えて load する
THEN Error Diagnostic は出ない

## B-004: inputs の供給元

GIVEN delegate を宣言する Session が `input` パラメータを持ち、`artifact` を宣言している
WHEN `inputs` の値に親 Session の Input、親 Session の handle そのもの、`work.<field>...`、`work.child.<field>...`、`r.request` を与えて load する
THEN Error Diagnostic は出ない
AND 各値は YAML で同じ供給元を書いた場合と同じ配線として解決される

## B-005: when の供給元

GIVEN delegate を宣言する Session
WHEN `when` に親 Session 自身の Artifact の field への参照と `work.child.<field>...` を単独または `r.all` / `r.any` で合成して与え、load する
THEN Error Diagnostic は出ない
AND 述語は YAML で同じ参照を書いた場合と同じ形に解決される

## B-006: child 段の走査

GIVEN delegate を宣言した Session
WHEN child が Session または Command のとき `work.child.<field>` を、child が Sequence または Fanout のとき統合 map に沿った `work.child.<Node名または添字>.<field>` を参照して load する
THEN Error Diagnostic は出ない

## B-007: delegate を宣言しない Session の child 参照

GIVEN delegate を宣言していない Session
WHEN その handle の `child` 段を辿る参照を書いて load する
THEN Error Diagnostic が出る
AND その Diagnostic は YAML で同じ参照を書いた場合と同じ `code` / `stage` / `message` である

## B-008: child の Artifact Contract による型検査

GIVEN delegate を宣言した Session
WHEN `inputs` または `when` が `work.child.<field>...` で child の Artifact Contract に存在しない field、または型が合わない field を参照して load する
THEN Error Diagnostic が出る

## B-009: require との併記

GIVEN `completion = { require = r.completion.approval }` を宣言した Session
WHEN その handle に delegate を宣言して load する
THEN Error Diagnostic は出ない
AND 構築された定義でその Session は `require` と delegate の両方を持つ

## B-010: delegate だけから参照される child

GIVEN 合成子の children に置かれておらず、delegate の `child` としてだけ参照される Node
WHEN その定義を load する
THEN 到達できない Node としての Error Diagnostic は出ない
AND その Node は構築された定義に含まれる

## B-011: YAML 定義との同一性

GIVEN 同じ delegate を Lua と YAML のそれぞれで書いた2つの workflow 定義
WHEN 両方を load する
THEN 構築された `WorkflowDefinition` は同一である

## B-012: YAML と同じ Diagnostic

GIVEN Lua の workflow 定義
WHEN `artifact` を宣言していない Session へ delegate を宣言する、必須 field を欠く、未知キーを与える、`max_iterations` に1未満を与える、親 Session 自身または root を `child` にする、他の合成子や別の delegate の child と同じ Node を `child` にする、のいずれかを書いて load する
THEN Error Diagnostic が出る
AND その Diagnostic は YAML で同じ誤りを書いた場合と同じ `code` / `stage` / `message` である

## B-013: 同じ handle への二重宣言

GIVEN すでに delegate を宣言した Session handle
WHEN 同じ handle へもう一度 delegate を宣言して load する
THEN `code` が `WFS002`、`stage` が `parse_shape` の Error Diagnostic が出る

## B-014: completion table の delegate

GIVEN Lua の workflow 定義
WHEN `completion` table に `delegate` キーを書いて load する
THEN `WFS002` の Error Diagnostic が出る

## B-015: 生成 stub

GIVEN Lua の補完用生成物を出力する操作
WHEN `.releash/releash.lua` を再生成する
THEN 生成された stub は Session handle の `delegate` メソッドの注釈を含む
AND 生成された stub は `child` 段の注釈を含む

## B-016: 用語集の記述

GIVEN `docs/glossary/WORKFLOW.md`
WHEN 「Lua」節と「Lua API」節を読む
THEN Lua 表面での delegate の受理形が記述されている
AND 「Lua は `delegate` を受理しない」「`delegate` は未対応」の記述は残っていない

## B-017: Artifact Contract に delegate field を持つ Session

GIVEN Artifact Contract の直下に `delegate` field を宣言した Session
WHEN その handle で `delegate{ ... }` を呼んで load する
THEN Error Diagnostic は出ない
AND 構築された定義でその Session は delegate を持つ
AND 同じ handle の `work.delegate` は `delegate` field への値参照にはならない

## B-018: inputs の順序と定義の同一性

GIVEN 複数の `inputs` を持つ同じ delegate を Lua と YAML のそれぞれで書いた2つの workflow 定義
WHEN YAML 側の `inputs` を Lua 側と異なる順序で書いて両方を load する
THEN 構築された `WorkflowDefinition` は同一である

## B-019: YAML の inputs の記述順

GIVEN 複数の `inputs` を持つ delegate を YAML で宣言した workflow 定義
WHEN それを load する
THEN child へ渡る入力の順序は `inputs` の記述順である
AND 保存と復元をまたいでも `inputs` の順序は記述順である
AND read model が返す `inputs` の順序は記述順である

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006, B-007 |
| R-006 | B-008 |
| R-007 | B-009 |
| R-008 | B-010 |
| R-009 | B-011, B-018 |
| R-010 | B-012 |
| R-011 | B-013 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-014 | B-016 |
| R-015 | B-017 |
| R-016 | B-019 |
