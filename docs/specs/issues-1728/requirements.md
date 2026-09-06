# Context

- Primary source は GitHub Issue #1728「[Session delegate と Node worktree 隔離] 参照の複数段解禁」（https://github.com/siro33950/releash/issues/1728 、label: enhancement、milestone: #85）である。
- Issue が「設計（正本）」として指定する `docs/specs/milestone-85/design.md` §5「参照」、および GitHub Milestone #85「01. Session delegate と Node worktree 隔離」の説明文も Primary source である。記述が食い違う場合は design.md に従う（milestone #85 説明文の指示）。
- 追加資料は `docs/glossary/WORKFLOW.md`（「Node の Interface と children の配線」「Fanout」「Command」「rules と辺」「Contract / schemas」「Lua」「Diagnostic」の各節）、`docs/glossary/DOMAIN.md`、`workflows/*.yml`（builtin 8本）、`workflows/examples/full-cycle-development.yml`、`AGENTS.md`、および現行の Rust 実装と既存テストである。
- 本 Issue が変えるのは、YAML と Lua の定義に書く参照文字列の段数だけである。参照が辿る先は `schemas` に宣言された Contract であり、Contract の宣言規則は変えない。入れ子の Object は現行の Contract subset で既に宣言できる。
- 参照の1段制限は次の5箇所にある（design.md §5 の一覧）。配線 `inputs` の供給元、辺の述語 `when.on` / `switch.on`、Command の `env`、テンプレート `{{ }}`、`fanout.items`。
- Contract は `type` / `properties` / `required` / `items` / `enum` だけを持つ JSON Schema subset であり、Object の `properties` と array の `items` を保持する（`docs/glossary/WORKFLOW.md`）。design.md §5 は、これにより多段でも load 時に静的に辿れるとし、1段に制限する根拠は正本のどこにも記録されていないとする。
- workflow 定義は YAML と Lua の2つの表面を持ち、どちらも同じ `WorkflowDefinition` を構築し、同じ定義上の誤りには同じ domain Diagnostic を使う（`docs/glossary/WORKFLOW.md`）。
- `docs/glossary/DOMAIN.md` は参照の段数を規定していないため、本 Issue の更新対象ではない。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。
- milestone #85 の全 ISSUE 共通の境界は次のとおりである。各 ISSUE は 1 PR で完結し、コード・Diagnostic・テストに加えて `docs/glossary/WORKFLOW.md` / `DOMAIN.md` の該当節、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）の更新を含む。各 ISSUE 完了時点で全定義が Diagnostic ゼロで load でき、テストが通る状態を保つ。
- 本 Issue は Wave 1 であり依存はない。後続の #1729（Sequence の Artifact を children の統合 map にする）、#1730（Fanout の Artifact を map にする）、#1731（述語を Predicate に共通化する）、#1732（completion を map にする）、#1733（Node worktree 隔離）、#1734（Session delegate）が本 Issue を前提とする。milestone #85 全体の前提は milestone #86（統一 Node モデル）の完了である。
- 現在状態の確認は、Issue、milestone、design.md、用語集、現行コード、既存テストの読解によって行った。build / test / lint などの検証コマンドは実行していない。
- `docs/specs/issues-1728` は未作成であり、本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash の workflow 定義を書く開発者、および milestone #85 の後続 ISSUE を実装する担当者である。

現在、参照は5箇所すべてで1段に制限されており、`<供給元>.<field>` より深い path を書くと load が Error Diagnostic で失敗する。そのため、入れ子の Object を持つ Artifact から内側の値を取り出せない。後続の #1729 / #1730 / #1734 が作る多段構造（Sequence の children 統合 map、Fanout の map、delegate の親 Artifact が持つ `child` キー）から値を引く経路が、この制限のままでは成立しない。

変更後は、5箇所すべてで多段参照を書ける。各段は load 時に静的に解決・型検査され、実行時にその値が渡る。解決できない段は Error Diagnostic になる。既存の1段参照の受理・拒否・値解決は変わらない。

# Current Behavior

現行の loader は、5箇所すべてに「2段以上を拒否する」検査を持つ。辺の述語は `when.on` と `switch.on` の2つの記法を含むため、下表は6行になる。

| 箇所 | 現行の受理形 | 2段以上を書いたときの結果 |
| --- | --- | --- |
| 配線 `inputs` の供給元 | `<name>` または `<name>.<field>` | resolve 段の Error Diagnostic `WFR007`。message は ``source must be `<name>` or `<name>.<field>``` |
| 辺の述語 `when.on` | 自 Node Artifact Contract の required boolean field 名1つ | `a.b` はドットを含む1つの field 名として扱われ、typecheck 段の Error Diagnostic `WFT001`。message は `routing field 'a.b' is not declared on Contract '<contract>'` |
| 辺の述語 `switch.on` | 自 Node Artifact Contract の required string enum field 名1つ | 同様に typecheck 段の Error Diagnostic `WFT002` |
| Command の `env` の値 | `<パラメータ>` または `<パラメータ>.<field>` | parse/shape 段の Error Diagnostic `WFS002`。message は ``input parameter reference 'doc.a.b' must be `<parameter>` or `<parameter>.<field>``` |
| テンプレート `{{ }}` | `{{ パラメータ }}` または `{{ パラメータ.field }}` | resolve 段の Error Diagnostic `WFR003` |
| `fanout.items` | literal 配列、または `<node>.<field>` | parse/shape 段の Error Diagnostic `WFS002`。message は `fanout.items must be a literal array or a <node>.<field> Artifact reference` |

最小の再現手順と実際の出力は次のとおりである。

1. `workflows/` に、Object の `properties` の中にさらに Object を持つ Contract を宣言し、その内側の field を `inputs` の供給元として `<兄弟 Node>.<外側 field>.<内側 field>` で参照する定義を置く。
2. Releash がその定義を load し、Diagnostic を表示する経路（アプリの workflow 定義表示、または local API / CLI の diagnostics 経路）で結果を見る。
3. 上表の Error Diagnostic が返り、その定義は load されない。他の4箇所も、同じ定義に同じ形の参照を置くと上表の結果になる。

そのほか、現行で確認した挙動は次のとおりである。

- 実行時の値解決も先頭1段しか見ない。配線・`env`・テンプレートのいずれも、供給元を引いた値に対して field を1回だけ引く。
- 型なし（Contract を持たない）input パラメータの field 参照は、供給元の形が実行時に決まるため load 時に検査されず受理される。型あり（Contract 付き）パラメータと Node Artifact の field は、Contract に対して検査される。
- Lua 表面にも同じ制限がある。`node.field.sub` は `nested artifact field references are not supported`、`input.field.sub` は `nested input field references are not supported` で拒否される。Lua の `r.when{ on = ... }` / `r.switch{ on = ... }` も同じ Source 記法を使う。
- Lua で共有 validation を通る Diagnostic の span は、参照の行ではなく node の定義位置である。参照の行を指すのは Lua host が index 時に返す拒否だけで、そちらは1件で打ち切られる。
- facet 本文をアプリから保存するときのテンプレート検証でも、`{{ goal.a.b }}` は「未定義のテンプレート変数」として拒否される。
- Command の Artifact の予約 field（`ok` / `exit_code` / `stdout` / `stderr` / `duration`）は、Contract 宣言なしに1段で参照できる。
- `request` と `items` は予約供給元名であり field を持たない。`request.<field>` / `items.<field>` は Error Diagnostic になる。
- builtin 8本と `workflows/examples/full-cycle-development.yml` に多段参照は含まれていない。既存テストが、builtin の Error Diagnostic がゼロであること、および正本サンプルの Diagnostic がゼロであることを確認している。

# Scope / Non-goals

## Scope

- 配線 `inputs` の供給元、辺の述語 `when.on` / `switch.on`、Command の `env`、テンプレート `{{ }}`、`fanout.items` の5箇所について、参照の1段制限を撤廃する。
- 5箇所の「2段以上を拒否する」検査を、「各段が解決できるか」の検査に置き換える。load 時の静的な解決・型検査と、実行時の値解決の両方を対象にする。
- 上記に伴う Error Diagnostic。
- YAML と Lua の両方の定義表面。
- `docs/glossary/WORKFLOW.md` の該当節（配線、rules と辺、Command の `env` とテンプレート、Fanout の `items`、Lua）の更新。
- `workflows/examples/full-cycle-development.yml` と `workflows/*.yml`（builtin 8本）が Diagnostic ゼロで load できる状態の維持。
- 5箇所それぞれの多段参照、解決できない段の Diagnostic、型なしパラメータの多段参照、既存の1段参照の回帰を対象とするテスト。

## Non-goals

- Artifact の構造変更。Sequence の children 統合 map 化（#1729）と Fanout の map 化（#1730）は本 Issue に含まない。
- Contract の宣言規則の変更。`schemas` に書ける型、`properties` / `required` / `items` / `enum` の意味は変えない。
- 述語の and / or 合成と `Predicate` への共通化（#1731）。
- `completion` の map 化と `require: approval` の受理（#1732）。
- Node worktree 隔離（#1733）と Session delegate（#1734）。
- 参照の起点（供給元スコープ）の規則の変更。どの Node・パラメータ・予約供給元を起点にできるかは変えない。Fanout child の Artifact を参照可能にすることも含まない。
- 段数以外の参照記法の変更。区切り記号、各段の識別子に使える文字種は変えない。
- 比較・計算・配列集約の式言語の導入。
- 参照先の Artifact の構造そのものの変更。本 Issue は参照の解決だけを変える。

# Requirements

- R-001: 配線 `inputs` の供給元に、`<供給元>.<field>.<field>` 以上の多段 field path を書ける。各段が解決できる場合、その定義は Error Diagnostic なく load でき、実行時に child の input パラメータへ末端の値が渡る。
- R-002: 辺の述語 `when.on` に多段 field path を書ける。末端 field は現行と同じ boolean の条件、すなわち直上の Object の `required` に含まれ、かつ boolean であることを満たす必要がある。経路上の中間段に `required` は要求しない。満たす場合、実行時にその値で辺が選ばれる。
- R-003: 辺の述語 `switch.on` に多段 field path を書ける。末端 field は現行と同じ string enum の条件、すなわち直上の Object の `required` に含まれ、かつ非空の `enum` を持つ string であることを満たす必要がある。経路上の中間段に `required` は要求しない。`cases` の網羅性検査は末端 field の enum 値に対して行われる。実行時はその値で分岐する。
- R-004: Command の `env` の値に多段 field path を書ける。解決できる場合、末端の値が現行と同じ規則（string はそのまま、string 以外は compact JSON テキスト）で子 process の環境変数へ渡る。
- R-005: テンプレート `{{ }}` に多段 field path を書ける。解決できる場合、Command の本文と Session の facet 本文の双方で末端の値に展開される。
- R-006: `fanout.items` の Artifact 参照に多段 field path を書ける。末端 field が配列である場合に受理し、実行時にその要素で children を展開する。
- R-007: 5箇所すべてで、解決できない段を含む参照は load 時に Error Diagnostic になり、その定義は load されない。解決できない段とは次のいずれかである。
    - 参照先の Object の `properties` に存在しない field を引く段。
    - Object でない値から field を引く段。array、string、boolean、integer、number はいずれも Object ではない。
    - 末端の型がその箇所の要求（`when.on` の boolean、`switch.on` の string enum、`fanout.items` の配列）に合わない段。
- R-008: 多段参照は YAML と Lua の両方の定義表面で書ける。同じ定義上の誤りには、どちらの表面でも同じ code、同じ stage、同じ message の Diagnostic が返る。Diagnostic の span は各表面の位置付けに従い、表面間で一致しなくてよい。
- R-009: 既存の1段参照および field を持たない参照は、参照文字列の前後に空白を含むもの、および Contract の property 名に `.` を含む field を `when.on` / `switch.on` の1段参照として引くものを除き、受理・拒否・実行時の値解決のいずれも従来どおりである。維持対象はこの3つであり、Error Diagnostic の message 文言は含まない。参照文字列の前後に空白を含む参照は、段数によらず受理しない。`.` は `when.on` / `switch.on` でも段の区切りとして解釈されるため、`.` を含む property 名は分岐条件から引けない。Contract を持たない供給元（型なし input パラメータ）を起点とする参照は、段数によらず load 時の静的検査の対象にならない。`main` から到達しない Node draft を起点とする未消費参照は、段数によらず load 時の静的検査の対象にならない。予約供給元 `request` と `items` は引き続き field を持たず、`request.<field>` / `items.<field>` は Error Diagnostic になる。Command の Artifact の予約 field は引き続き Contract 宣言なしに参照できる。
- R-010: `workflows/*.yml`（builtin 8本）と `workflows/examples/full-cycle-development.yml` は、本変更後も Diagnostic ゼロで load できる。

# Assumptions / Open Questions

Assumption はない。Open Question はない。
