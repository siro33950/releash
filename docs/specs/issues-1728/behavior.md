## B-001: 配線 `inputs` の多段参照

GIVEN 兄弟 Node の Artifact Contract が、Object の `properties` の中にさらに Object を持つ
WHEN その内側の field を、child の配線 `inputs` の供給元として `<兄弟 Node>.<外側 field>.<内側 field>` で参照する定義を load する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、child の input パラメータには末端 field の値が渡る

## B-002: 辺の述語 `when.on` の多段参照

GIVEN 自 Node の Artifact Contract が、入れ子の Object を経由して required な boolean field を持つ
WHEN その boolean field を辺の `when.on` に多段 field path で書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 末端 field の値が true のとき、`then` が指す Node へ遷移する
AND 末端 field の値が false のとき、`then` が指す Node へは遷移しない

## B-003: `when.on` の末端 field が boolean の条件を満たさない

GIVEN 辺の `when.on` に書いた多段 field path の末端 field が、required な boolean であるという条件を満たさない
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-004: 辺の述語 `switch.on` の多段参照

GIVEN 自 Node の Artifact Contract が、入れ子の Object を経由して required な string enum field を持つ
WHEN その field を辺の `switch.on` に多段 field path で書き、`cases` に末端 field の enum 値を並べた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 末端 field の値と一致する `case` の Node へ遷移する

## B-005: `switch.on` の末端 field が string enum の条件を満たさない

GIVEN 辺の `switch.on` に書いた多段 field path の末端 field が、required な string enum であるという条件を満たさない
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-006: 多段 `switch.on` の `cases` が末端 field の enum を網羅しない

GIVEN 辺の `switch.on` に書いた多段 field path の末端 field が required な string enum である
WHEN `cases` が末端 field の enum 値を網羅せず、同じ要素に catch-all の遷移先も無い定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-007: Command の `env` の多段参照

GIVEN Command が宣言した型あり input パラメータの Contract が、入れ子の Object field を持つ
WHEN その内側の field を `env` の値に多段 field path で書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 末端の値が string ならその文字列が、string 以外なら compact JSON テキストが、子 process の環境変数へ渡る

## B-008: テンプレート `{{ }}` の多段参照

GIVEN Node が宣言した型あり input パラメータの Contract が、入れ子の Object field を持つ
WHEN その内側の field を多段 field path で参照する `{{ }}` を含む定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND Command の本文と Session の facet 本文の双方で、その `{{ }}` は末端 field の値に展開される

## B-009: 多段参照を含む facet 本文の保存

GIVEN Session が宣言した型あり input パラメータの Contract が、入れ子の Object field を持つ
WHEN その内側の field を多段 field path で参照する `{{ }}` を含む facet 本文を保存する
THEN 未定義のテンプレート変数として拒否されず、保存できる

## B-010: `fanout.items` の多段参照

GIVEN Node の Artifact Contract が、入れ子の Object を経由して配列 field を持つ
WHEN その配列 field を `fanout.items` に多段 field path で書いた定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND その配列の要素ごとに children が展開される

## B-011: `fanout.items` の末端 field が配列でない

GIVEN `fanout.items` に書いた多段 field path の末端 field が配列でない
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-012: 参照先に存在しない field を引く段

GIVEN 多段 field path のいずれかの段が、参照先の Contract に存在しない field を引く
WHEN その参照を配線 `inputs`、`when.on`、`switch.on`、`env`、`{{ }}`、`fanout.items` のいずれかに書いた定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-013: Object でない値から field を引く段

GIVEN 多段 field path の途中の段が、array / string / boolean / integer / number の値から field を引く
WHEN その参照を配線 `inputs`、`when.on`、`switch.on`、`env`、`{{ }}`、`fanout.items` のいずれかに書いた定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-014: Lua 表面での多段参照

GIVEN YAML で受理される多段参照と、同じ意味の定義を Lua 表面で書く
WHEN その Lua 定義を load する
THEN YAML と同じく Error Diagnostic なく load でき、実行時に同じ値が解決される
AND 解決できない段を含む場合は、YAML と同じ code、stage、message の Diagnostic が返り、その定義は load されない

## B-015: 既存の1段参照の回帰

GIVEN 本変更前に受理された1段参照、および field を持たない参照を含む定義（参照文字列の前後に空白を含むもの、および Contract の property 名に `.` を含む field を `when.on` / `switch.on` の1段参照として引くものを除く）
WHEN その定義を load して実行する
THEN 受理と拒否は本変更前と同じである
AND 実行時に解決される値も本変更前と同じである
AND 参照文字列の前後に空白を含む参照は段数によらず受理されない

## B-016: 予約供給元 `request` / `items` の field 参照

GIVEN `request` または `items` を供給元とする参照
WHEN `request.<field>` または `items.<field>` を書いた定義を load する
THEN Error Diagnostic になり、その定義は load されない

## B-017: Command の Artifact の予約 field 参照

GIVEN Command が `ok` / `exit_code` / `stdout` / `stderr` / `duration` を Artifact Contract に宣言していない
WHEN これらの予約 field を1段で参照する定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND 実行時、その Command の実行結果の値が解決される

## B-018: builtin 定義と正本サンプルの load

GIVEN `workflows/*.yml` の builtin 8本と `workflows/examples/full-cycle-development.yml`
WHEN 本変更後にそれぞれを load する
THEN Diagnostic はゼロである

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003 |
| R-003 | B-004, B-005, B-006 |
| R-004 | B-007 |
| R-005 | B-008, B-009 |
| R-006 | B-010, B-011 |
| R-007 | B-003, B-005, B-011, B-012, B-013 |
| R-008 | B-014 |
| R-009 | B-015, B-016, B-017 |
| R-010 | B-018 |
