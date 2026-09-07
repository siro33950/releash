## B-001: `items` を宣言しない Fanout の Artifact

GIVEN `items` を宣言しない Fanout の children エントリ `a` と `b` がそれぞれ Artifact を産出する
WHEN その Fanout を実行し、`a` と `b` の slot が Artifact を産出して決着する
THEN その Fanout の Artifact は、`a` と `b` を children エントリ名としてキーに持つ map である
AND 各キーの値は、その slot の Artifact そのものである

## B-002: `items` を宣言する Fanout の Artifact

GIVEN `items` を宣言する Fanout の children エントリが Artifact を産出する
WHEN その Fanout を実行し、`items` の各要素について展開した slot が Artifact を産出して決着する
THEN その Fanout の Artifact は、展開順の添字をキーに持つ map である
AND 各キーの値は、その添字が指す slot の Artifact そのものである

## B-003: `items` と複数 children エントリを同時に宣言した Fanout の展開

GIVEN Fanout が `items` と複数の children エントリを同時に宣言する
WHEN その Fanout を実行し、各 item について全 children エントリを展開して決着する
THEN 展開した slot は、item を外側・children エントリを内側とする順で添字を与えられる
AND 全ての slot のキーは Fanout の Artifact 直下に並び、item ごとや children エントリごとの階層は作られない

## B-004: slot が残らない Fanout の Artifact

GIVEN Fanout の実行で slot が一つも展開されない、または展開した slot が全て `on_failure: ignore` の失敗になる
WHEN その Fanout が決着する
THEN その Fanout の Artifact は空の object である

## B-005: `on_failure: ignore` を宣言した child の失敗 slot

GIVEN Fanout の children エントリの一つに `on_failure: ignore` が宣言されている
WHEN その Fanout を実行し、そのエントリから展開した slot が失敗し、他の slot は決着する
THEN 失敗した slot のキーは Fanout の Artifact に現れない
AND 他の slot のキーと値は、その失敗が起きなかった場合と同じである

## B-006: Artifact を産出しなかった slot

GIVEN Fanout の children エントリの一つが `artifact` を宣言せず、別のエントリが `on_failure` を宣言しないまま失敗する
WHEN その Fanout が決着する
THEN `artifact` を宣言しないエントリから展開した slot はキーとして残り、その値は `null` である
AND `on_failure` を宣言しないまま失敗した slot はキーとして残り、その値は `null` である

## B-007: Fanout の `artifact` 宣言

GIVEN Fanout に `artifact` を書く
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-008: 配線 `inputs` からの children エントリ名キーの参照

GIVEN `items` を宣言しない Fanout `fan` の children エントリ `a` の Artifact が field `passed` を持つ
WHEN 下流 Node の配線 `inputs` の供給元に `fan.a.passed` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その input パラメータには `a` の slot の Artifact の `passed` の値が渡る

## B-009: 配線 `inputs` からの添字キーの参照

GIVEN `items` を宣言する Fanout `fan` の children エントリの Artifact が field `passed` を持つ
WHEN 下流 Node の配線 `inputs` の供給元に `fan.0.passed` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その input パラメータには展開順で先頭の slot の Artifact の `passed` の値が渡る

## B-010: 辺の述語 `when.on` からの slot の参照

GIVEN `items` を宣言しない Fanout `fan` の children エントリ `a` の Artifact が、required な boolean field `passed` を持つ
WHEN `fan` を自 Node とする children エントリの辺に `when.on` として `a.passed` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `a` の slot の `passed` が true のとき、`then` が指す Node へ遷移する
AND `a` の slot の `passed` が false のとき、`then` が指す Node へは遷移しない

## B-011: 辺の述語 `switch.on` からの slot の参照

GIVEN `items` を宣言しない Fanout `fan` の children エントリ `a` の Artifact が、required な非空 string enum field を持つ
WHEN `fan` を自 Node とする children エントリの辺に `switch.on` としてその field を `a.<field>` で書き、`cases` に enum 値を並べた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `a` の slot のその field の値と一致する `case` の Node へ遷移する

## B-012: `fanout.items` からの slot の参照

GIVEN `items` を宣言しない Fanout `fan` の children エントリ `a` の Artifact が配列 field `tasks` を持つ
WHEN 別の Fanout の `items` の供給元に `fan.a.tasks` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その Fanout は `a` の slot の Artifact の `tasks` の各要素で展開される

## B-013: Lua 表面での slot の参照

GIVEN Lua 表面で、Fanout の slot を起点とする供給元を配線 `inputs`、辺の述語、`fanout.items` に書く
WHEN その定義を load する
THEN YAML 表面に同じ参照を書いた場合と同じく、Error Diagnostic なく load できる

## B-014: field path なしで Fanout を受ける配線

GIVEN 下流 Node の配線 `inputs` が Fanout を field path なしの供給元にする
WHEN その定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その input パラメータには、変更前と同じ slot 集合の結果が map として渡る

## B-015: builtin 定義と正本サンプルの load

GIVEN `workflows/*.yml` の builtin 8本と `workflows/examples/full-cycle-development.yml`
WHEN 本変更後にそれぞれを load する
THEN Diagnostic はゼロである

## B-016: 書き換え後の Fanout の結果を受ける下流の維持

GIVEN 書き換え後の `workflows/examples/full-cycle-development.yml` で、`merge_implementations` と `check_integration` が `implement_all` の結果を、`merge_fixes` が `fix_all` の結果を受ける
WHEN その workflow を実行する
THEN 各下流 Node には、書き換え前と同じ slot 集合の結果が判断材料として渡る
AND `check_integration` を自 Node とする辺は、書き換え前と同じ判断材料で同じ遷移先を選ぶ

## B-017: 正本ドキュメントでの Fanout の Artifact の説明

GIVEN workflow 定義の書き手が `docs/glossary/WORKFLOW.md` で Fanout を確認する
WHEN Fanout の Artifact の作られ方と、Fanout を供給元にできる参照を読む
THEN Fanout の Artifact は、slot をキーで引ける map として説明されている
AND キーが `items` の有無で決まることが説明されている
AND Fanout の Artifact を配列とする前提の記述は残っていない
AND Fanout の Artifact が配列であることを根拠に参照可能性を制限する記述は残っていない

## B-018: Sequence の統合 map を経由した slot の参照

GIVEN Sequence `seq` の children エントリに `items` を宣言しない Fanout `fan` があり、`fan` の children エントリ `a` の Artifact が field `passed` を持つ
WHEN `seq` の外の Node の配線 `inputs` の供給元に `seq.fan.a.passed` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その input パラメータには `a` の slot の Artifact の `passed` の値が渡る

## B-019: Sequence の統合 map を経由した slot からの分岐

GIVEN Sequence `seq` の children エントリに `items` を宣言しない Fanout `fan` があり、`fan` の children エントリ `a` の Artifact が required な boolean field `passed` を持つ
WHEN `seq` を自 Node とする children エントリの辺に `when.on` として `fan.a.passed` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `a` の slot の `passed` が true のとき、`then` が指す Node へ遷移する
AND `a` の slot の `passed` が false のとき、`then` が指す Node へは遷移しない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-004, B-007 |
| R-002 | B-001 |
| R-003 | B-002, B-003 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-008, B-009, B-010, B-011, B-012, B-013 |
| R-007 | B-014 |
| R-008 | B-015 |
| R-009 | B-016 |
| R-010 | B-017 |
| R-011 | B-018, B-019 |
| R-012 | B-010, B-011 |
