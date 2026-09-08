## B-001: 全4種の Node での `require: approval` の受理

GIVEN Session、Command、Fanout、Sequence のそれぞれの Node に `completion` map を置き、その要素に `require: approval` を書いた定義
WHEN その定義を load する
THEN `completion` に起因する Error Diagnostic はなく、その定義は load できる

## B-002: `require: approval` を宣言した Node の承認

GIVEN `completion` map に `require: approval` を書いた Node を含む定義を実行している
WHEN その Node が本来の完了条件（Session は Submit と provider Stop の二信号、Command は process 終了、Fanout は全 child の決着、Sequence は終端到達）を満たす
THEN その Node は完了せず WaitingApproval になる
AND 人間が Approve すると、その Node は完了する
AND この挙動は、変更前に `completion: approval` を宣言した同じ種別の Node と同じである

## B-003: `completion` を省略した Node の自動完了

GIVEN `completion` を宣言しない Node を含む定義を実行している
WHEN その Node が本来の完了条件を満たす
THEN その Node は WaitingApproval を経ずに完了する
AND この挙動は、変更前に `completion` を省略した同じ種別の Node と同じである

## B-004: 文字列形式の `completion` の拒否

GIVEN `completion: approval` または `completion: auto` の文字列形式を書いた Node を含む定義
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-005: `require` の値域と `completion` map の未知キー・空 map

GIVEN `completion` map の `require` に `approval` 以外の値（`auto` を含む）を書いた定義、`completion` map に `require` 以外のキーを書いた定義、または要素を持たない `completion` map を書いた定義
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-006: Lua 表面での同値性

GIVEN Lua 表面で `completion = { require = r.completion.approval }` を書いた Node を含む定義
WHEN その Lua 定義を load して実行する
THEN YAML の `require: approval` と同じく `completion` に起因する Error Diagnostic なく load でき、その Node は本来の完了条件の後に WaitingApproval となり、人間の Approve で完了する
AND table で包まない `completion = r.completion.approval` を書いた定義は、YAML の文字列形式と同じく Error Diagnostic になり、その定義は load されない
AND YAML で Error Diagnostic になる `completion` の誤りと同じ誤りを Lua 表面で書いた場合は、YAML と同じ `code`、`stage`、`message` の Diagnostic が返り、その定義は load されない

## B-007: builtin 定義と正本サンプルの load と承認

GIVEN 本変更後の `workflows/*.yml` の builtin 8本と `workflows/examples/full-cycle-development.yml`
WHEN それぞれを load する
THEN Diagnostic はゼロである
AND 変更前に `completion: approval` を宣言していた Node は `require: approval` の map 形で宣言されており、本来の完了条件の後に WaitingApproval となる

## B-008: WORKFLOW.md の整合

GIVEN 本変更後の `docs/glossary/WORKFLOW.md`
WHEN Node 共通 field の表、「Session」節、「completion」節、「Lua」節、「Lua API」の表を読む
THEN `completion` の受理形、省略時の意味、Node 種別ごとの承認の挙動、および Lua の書き方の記述が変更後の挙動と一致する
AND `auto` / `approval` の文字列形式を前提とする記述は残らない

## B-009: DOMAIN.md の整合

GIVEN 本変更後の `docs/glossary/DOMAIN.md`
WHEN 正規語の表の `completion` の行を読む
THEN `completion` が Node の完了に対する要求の集合であること、要求として `require: approval` を持つこと、要求を書かないことが自動完了を意味することが読める

## B-010: read model と UI の整合

GIVEN `require: approval` を宣言した Node と `completion` を宣言しない Node を含む定義
WHEN その定義を read model で読み、workflow 定義詳細と設定画面を表示する
THEN read model の `completion` は `require: approval` の宣言の有無を変更後の受理形と一致する形で示し、`auto` という値を持たない
AND workflow 定義詳細は `require: approval` を宣言した Node にだけ承認要求の badge を付け、その文言は変更後の受理形（`require: approval`）と一致する
AND 設定画面の Approval auto-approve の説明文は `completion: approval` の文字列形式を引用せず、`require: approval` を宣言した Node が対象であることを示す

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008 |
| R-009 | B-009 |
| R-010 | B-010 |
