## B-001: 通った children の統合 map

GIVEN Sequence の children エントリ `a` と `b` がそれぞれ Artifact を産出する
WHEN その Sequence を実行し、`a` と `b` の両方を通って終端に達する
THEN その Sequence の Artifact は、`a` と `b` を children エントリ名としてキーに持つ map である
AND 各キーの値は、その children エントリの Artifact そのものである

## B-002: map に現れない children エントリ

GIVEN Sequence の children エントリに、その実行では辺の分岐によって通らないものと、通るが Artifact を産出しないものがある
WHEN その Sequence を実行して終端に達する
THEN 通らなかった children エントリは map のキーに現れない
AND Artifact を産出しなかった children エントリは map のキーに現れない
AND どの children エントリもキーにならない場合、その Sequence の Artifact は空の map である

## B-003: 複数回通った children エントリの結果

GIVEN Sequence の children エントリ `a` が後方辺のループ上にあり、Artifact を産出する
WHEN 一つの Sequence 実行の中で `a` を複数回通って終端に達する
THEN map の `a` の値は、その実行で最後に `a` が産出した Artifact である

## B-004: 宣言のない Sequence が産出する Artifact

GIVEN Sequence が Artifact に関する宣言を持たない
WHEN その Sequence を配線 `inputs` の供給元に指定した定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、配線先の input パラメータには通った children の統合 map が渡る

## B-005: 配線 `inputs` からの統合 map の多段参照

GIVEN Sequence `s` の children エントリ `a` の Artifact が field `passed` を持つ
WHEN 下流 Node の配線 `inputs` の供給元に `s.a.passed` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その input パラメータには `a` の Artifact の `passed` の値が渡る

## B-006: 辺の述語 `when.on` からの統合 map の参照

GIVEN Sequence `s` の children エントリ `a` の Artifact が、required な boolean field `has_open_threads` を持つ
WHEN `s` を自 Node とする children エントリの辺に `when.on` として `a.has_open_threads` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `a` の `has_open_threads` が true のとき、`then` が指す Node へ遷移する
AND `a` の `has_open_threads` が false のとき、`then` が指す Node へは遷移しない

## B-007: 辺の述語 `switch.on` からの統合 map の参照

GIVEN Sequence `s` の children エントリ `a` の Artifact が、required な非空 string enum field を持つ
WHEN `s` を自 Node とする children エントリの辺に `switch.on` としてその field を `a.<field>` で書き、`cases` に enum 値を並べた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `a` のその field の値と一致する `case` の Node へ遷移する

## B-008: Sequence の `output` 宣言

GIVEN YAML の Sequence に `output` を書く
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-009: Sequence の `artifact` 宣言

GIVEN YAML の Sequence に `artifact` を書く
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-010: Lua 表面での `output` / `artifact` 宣言

GIVEN Lua 表面の Sequence に `output` または `artifact` を渡す
WHEN その定義を load する
THEN YAML と同じく Error Diagnostic になり、その定義は load されない

## B-011: builtin 定義と正本サンプルの load

GIVEN `workflows/*.yml` の builtin 8本と `workflows/examples/full-cycle-development.yml`
WHEN 本変更後にそれぞれを load する
THEN Diagnostic はゼロである

## B-012: `review_scan` の分岐の維持

GIVEN 書き換え後の `workflows/examples/full-cycle-development.yml` の `review` の child `review_scan`
WHEN `check_full_review_threads` が open thread の有無を示す判断材料を産出して `review_scan` が終端に達する
THEN その値が true のときも false のときも、書き換え前と同じ遷移先が選ばれる

## B-013: Sequence の Artifact を受ける下流配線の維持

GIVEN 書き換え後の `authoring_design` の配線 `behavior: authoring_behavior`、および `workflows/05_review-fix.yml` の `fix_report` の配線 `verify_fixes: fix_round`
WHEN それぞれの workflow を実行する
THEN 下流 Node には、書き換え前に渡っていた children エントリの Artifact と同じ判断材料が渡る

## B-014: Fanout の child である Sequence の結果を受ける下流の維持

GIVEN 書き換え後の `implement_and_verify` と `fix_and_verify` が Fanout の child であり、その結果が `results: implement_all` と `results: fix_all` で下流 Session に渡る
WHEN それぞれの workflow を実行する
THEN 下流 Session には、書き換え前と同じ判断材料が渡る

## B-015: 正本ドキュメントでの Sequence の Artifact の説明

GIVEN workflow 定義の書き手が `docs/glossary/WORKFLOW.md` で Sequence を確認する
WHEN Sequence の Artifact の作られ方と、Sequence に書ける field を読む
THEN Sequence の Artifact は、通った children の Artifact を children エントリ名でキーにした統合 map として説明されている
AND `output` と Sequence の `artifact` を書ける前提の記述は残っていない

## B-016: `output` という名前の Node

GIVEN `output` という名前の Node を宣言した定義がある
WHEN その定義を load する
THEN 予約 Node 名を理由とする Error Diagnostic は出ない

## B-017: `fanout.items` からの統合 map の多段参照

GIVEN Sequence `s` の children エントリ `a` の Artifact が配列 field `tasks` を持つ
WHEN Fanout の `items` の供給元に `s.a.tasks` を書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、Fanout は `a` の Artifact の `tasks` の各要素で展開される

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005, B-006, B-007, B-017 |
| R-006 | B-008, B-010 |
| R-007 | B-009, B-010 |
| R-008 | B-011 |
| R-009 | B-012, B-013, B-014 |
| R-010 | B-015 |
| R-011 | B-016 |
