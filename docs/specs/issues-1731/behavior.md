## B-001: `and` で合成した述語の遷移

GIVEN 自 Node の Artifact Contract が required な boolean field `passed` と `clean` を持つ
WHEN その Node を child とする辺の `when` の `on` に `and` の map を置き、その要素に `passed` と `clean` を並べた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `passed` と `clean` の両方が true のとき、`then` が指す Node へ遷移する
AND `passed` と `clean` のいずれかが false のとき、sibling `next` が指す Node へ進む

## B-002: `or` で合成した述語の遷移

GIVEN 自 Node の Artifact Contract が required な boolean field `passed` と `skipped` を持つ
WHEN その Node を child とする辺の `when` の `on` に `or` の map を置き、その要素に `passed` と `skipped` を並べた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `passed` と `skipped` のいずれかが true のとき、`then` が指す Node へ遷移する
AND `passed` と `skipped` の両方が false のとき、sibling `next` が指す Node へ進む

## B-003: ネストした述語の遷移

GIVEN 自 Node の Artifact Contract が required な boolean field `passed`、`clean`、`skipped` を持つ
WHEN その Node を child とする辺の `when` の `on` に `and` の map を置き、その要素に `passed` と、`clean` と `skipped` を要素とする `or` の map を並べた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `passed` が true かつ `clean` と `skipped` のいずれかが true のとき、`then` が指す Node へ遷移する
AND それ以外のとき、sibling `next` が指す Node へ進む

## B-004: 単一の boolean field 参照の `when` の回帰

GIVEN 本変更前に受理された、`on` に boolean field 参照を一つ書く `when` を含む定義
WHEN その定義を load して実行する
THEN 受理と拒否は本変更前と同じである
AND 参照先が true のとき `then` が指す Node へ遷移し、false のとき sibling `next` が指す Node へ進む

## B-005: 述語の要素としての多段参照と合成子の map 経由の参照

GIVEN Sequence `s` の children エントリ `a` と `b` の Artifact が、それぞれ required な boolean field `passed` を持つ
WHEN `s` を自 Node とする辺の `when` の `on` に `and` の map を置き、その要素に `a.passed` と `b.passed` を並べた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、各参照は単一参照の `when.on` に同じ field path を書いたときと同じ値に解決される

## B-006: boolean を指さない参照を含む述語

GIVEN 辺の `when` の述語に含まれる参照のいずれかが、末端 field が boolean でない、末端 field が直上 Object の `required` に含まれない、または解決できない段を含む
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-007: 参照先が未確定の参照を含む述語の評価

GIVEN 辺の `when` の述語が、実行時に自 Node の Artifact に値が存在しない参照を要素に含む
WHEN その辺を評価する
THEN 値が存在しない参照は false として評価される
AND その参照を `and` で合成した述語は、他の要素の値によらず sibling `next` が指す Node へ進む
AND その参照を `or` で合成した述語は、他の要素のいずれかが true のとき `then` が指す Node へ遷移する

## B-008: Lua 表面での述語

GIVEN YAML で受理される `and` / `or` / ネストの述語と同じ意味の定義を、`r.all{ ... }` / `r.any{ ... }` で組み立てた述語を `r.when` の `on` に渡して Lua 表面で書く
WHEN その Lua 定義を load して実行する
THEN YAML と同じく Error Diagnostic なく load でき、同じ判断材料で同じ Node へ遷移する
AND boolean を指さない参照を含む場合は、YAML と同じ code、stage、message の Diagnostic が返り、その定義は load されない

## B-009: builtin 定義と正本サンプルの load

GIVEN `workflows/*.yml` の builtin 8本と `workflows/examples/full-cycle-development.yml`
WHEN 本変更後にそれぞれを load する
THEN Diagnostic はゼロである

## B-010: 正本文書の整合

GIVEN 本変更後の `docs/glossary/WORKFLOW.md`
WHEN 「rules と辺」節と「Lua API」節を読む
THEN 辺の `when` の受理形、型検査、評価規則、および Lua の述語の書き方の記述が変更後の挙動と一致する
AND `when.on` が boolean field を一つだけ指す前提の記述は残らない

## B-011: 要素が空の述語

GIVEN 辺の `when` の述語に、要素が空の `and` または `or` を含む定義
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-012: 用語集の述語

GIVEN 本変更後の `docs/glossary/DOMAIN.md`
WHEN 正規語の表を読む
THEN 述語（Predicate）の行があり、辺と completion の判断に使う真偽値の論理式であること、原子が Artifact の boolean field 参照であること、`and` / `or` で合成することが読める

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
| R-011 | B-011 |
| R-012 | B-012 |
