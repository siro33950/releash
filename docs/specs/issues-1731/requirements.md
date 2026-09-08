# Context

- Primary source は GitHub Issue #1731「[Session delegate と Node worktree 隔離] 述語を Predicate に共通化し辺で and / or を受理する」（https://github.com/siro33950/releash/issues/1731 、state: OPEN、label: enhancement、milestone: #85、comment なし）である。
- Issue が「設計（正本）」として指定する `docs/specs/milestone-85/design.md` §6「述語の共通化」、述語の受理形と評価規則を示す同 §2.5「述語と評価」、および §7「現行からの変更一覧」の「辺の `when`」行、ならびに GitHub Milestone #85「01. Session delegate と Node worktree 隔離」の説明文も Primary source である。記述が食い違う場合は design.md に従う（milestone #85 説明文の指示）。
- 追加資料は `docs/glossary/WORKFLOW.md`（「rules と辺」「Lua」「Lua API」「Diagnostic」の各節）、`docs/glossary/DOMAIN.md`、`docs/specs/issues-1728/`、`docs/specs/issues-1729/`、`docs/specs/issues-1730/`、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）、`src-tauri/src/adaptor/gateway/workflow/builtin.rs`、`AGENTS.md`、`docs/architecture/`、および現行の Rust 実装と既存テストである。
- design.md §6 は次を定める。辺の `when` と delegate の `when` は同じ述語構造を持つ。述語は真偽値の合成だけを担い、参照の解決は各所が自分のスコープで行う。共通化するのは論理式の構造、評価（論理演算）、および「各参照が boolean を指すか」という型検査の枠である。辺の `when` にも and / or が使えるようになる。`switch` は string enum の多分岐であり述語ではないため対象外である。
- design.md §2.5 は述語の形を次のように示す。原子は Artifact の required boolean field への参照であり、`and` / `or` で合成できる。参照先が未確定なら false になり、これは現行の辺の評価と同じである。

    ```yaml
    when: done
    when:
      and:
        - done
        - child.judge.clean
    when:
      or:
        - child.judge.clean
        - child.scan.skipped
    ```

- 依存する #1728（参照の1段制限の撤廃）は commit ffbdb67f7 で main に取り込み済みである。同じ Wave 2 の #1729（Sequence の Artifact を統合 map にする）は commit 2467b1d0f、#1730（Fanout の Artifact を map にする）は commit 52f5b862d で取り込み済みであり、現行の `when.on` は多段 field path と、Sequence / Fanout の map を経由する参照を既に受理する。
- 後続の #1734（Session delegate）は `completion.delegate.when` で本 Issue の述語を使う。#1732（completion の map 化）は本 Issue と独立である。
- 辺の rules の構造（`then` の位置、`next` が when の else と無条件辺の二役を持つこと、`switch` が `cases` で網羅することとの非対称）の再検討は #1749（https://github.com/siro33950/releash/issues/1749 ）で行う。本 Issue は現行の骨格（`when: { on, then }` と sibling `next`）を維持し、`on` の値に述語を置く。
- workflow 定義は YAML と Lua の2つの表面を持ち、どちらも同じ `WorkflowDefinition` を構築し、同じ定義上の誤りには同じ domain Diagnostic を使う（`docs/glossary/WORKFLOW.md`）。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。
- milestone #85 の全 ISSUE 共通の境界は次のとおりである。各 ISSUE は 1 PR で完結し、コード・Diagnostic・テストに加えて `docs/glossary/WORKFLOW.md` / `DOMAIN.md` の該当節、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）の更新を含む。各 ISSUE 完了時点で全定義が Diagnostic ゼロで load でき、テストが通る状態を保つ。
- 現在状態の確認は、Issue、milestone、design.md、用語集、現行コード、既存テストの読解によって行った。build / test / lint などの検証コマンドは実行していない。
- `docs/specs/issues-1731` は本文書の作成時点まで未作成であり、現行の Rust ソースに述語を表す型は存在しない。本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash の workflow 定義を YAML または Lua で書く開発者、および後続 #1734 で `completion.delegate.when` を実装する担当者である。

現在、辺の `when` は自 Node の Artifact の boolean field を一つだけ指せる。複数の条件を組み合わせて分岐したい場合、定義を書く開発者は、判定結果を一つの boolean field に畳む Command または Session を余分に挟むしかない。また、述語の評価と型検査は辺の遷移規則の中に埋め込まれており、delegate の `when` が同じ述語を必要としても再利用できる形になっていない。

変更後は、辺の `when` で複数の boolean field 参照を `and` / `or` で合成し、それらをネストして書ける。単一の boolean field を指す従来の `when` は従来どおり動く。述語に含まれる各参照は load 時に boolean を指すことが検査され、指さなければ Error Diagnostic になる。実行時に参照先が未確定の参照は false として評価される。

# Current Behavior

現行の辺の `when` に関する挙動は次のとおりである。

| 対象 | 現行の挙動 |
| --- | --- |
| 辺の `when` の受理形 | `when: { on: <field path>, then: <Node> }` と、同じ要素の sibling `next` の3つの値。`on` は文字列だけを受理する |
| `on` に文字列以外を書く | YAML の `on` に map（例 `on: { and: [passed, clean] }`）を書くと、parse/shape 段の Error Diagnostic `WFS002` になり、その定義は load されない。and / or を表す記法は存在しない |
| `on` の指す先 | 自 Node の Artifact を起点にした field path。末端は直上 Object の `required` に含まれる boolean field。Sequence を自 Node とする辺では `<child>.<field>...`、Fanout を自 Node とする辺では `<キー>.<field>...` を起点にできる（#1728 / #1729 / #1730 で確定済み） |
| load 時の型検査 | 末端が required boolean でない場合、typecheck 段の Error Diagnostic `WFT001`。message は ``when.on field '<field>' must be a required boolean``。存在しない段は ``routing field '<field>' has undeclared segment N ('<segment>')``、Object でない値から引く段は ``routing field '<field>' cannot resolve segment N ('<segment>') from a non-object value``、required でない末端は ``routing field '<field>' must be required on its parent Object`` |
| Artifact を持たない Node への `when` | typecheck 段の Error Diagnostic `WFT006` |
| 実行時の評価 | 自 Node の Artifact を field path に沿って引き、boolean なら値、値が存在しないか boolean でなければ false として扱う。true なら `then`、false なら sibling `next` へ進む |
| Lua 表面 | `r.when{ on, on_true, next }`。`on` は Source（`node.field...`）でなければならず、Source 以外を渡すと `WFS002`（``field 'on' must be Source``）。Source が自 child の Artifact field でなければ `WFR003`（``rule discriminator must reference the current child artifact field``） |
| 述語の表現 | 述語を表す型は存在しない。評価は辺の遷移先決定の中に埋め込まれ、型検査は辺の検証の中で `when.on` 専用に行われる |
| UI | workflow 定義の詳細表示が、辺を ``when <on> then <then> else <next>`` の一行で表示する。`on` は文字列として受け取る |
| 保存済み実行 | 実行木の root の開始事実に定義の snapshot が保存され、辺は `when` の `on` を文字列として持つ。読み出し時に解釈できない Node 定義は Node 単位で扱われる（#1744） |

最小の再現手順と結果は次のとおりである。

1. `workflows/` に、required boolean field `passed` と `clean` を持つ Contract を Artifact に宣言する Session `judge` を置き、`judge` を child とする Sequence の辺に次を書く。

    ```yaml
    rules:
      - when:
          on:
            and:
              - passed
              - clean
          then: done
        next: done
    ```

2. Releash がその定義を load し Diagnostic を表示する経路（アプリの workflow 定義表示、または local API / CLI の `releash workflow diagnostics`）で結果を見る。
3. parse/shape 段の Error Diagnostic `WFS002` になり、その定義は load されない。`passed` と `clean` の両方が true のときだけ `done` へ進む辺を、Command または Session を追加せずに書く方法は存在しない。

現行の workflow 定義の実態は次のとおりである。

- `workflows/*.yml`（builtin 8本）の `when` は 14 箇所である。01_author-spec 2、02_implement-existing-spec 2、03_full-review 0、04_review-fix-policy 1、04_review-fix-policy-manual 1、05_review-fix 2、06_handle-pr-review 3、06_handle-pr-review-manual 3。いずれも1段の boolean field を一つ指す。
- `workflows/examples/full-cycle-development.yml` の `when` は 4 箇所である。`behavior_complete`（L300）、`design_complete`（L336）、`complete`（L520）、`check_full_review_threads.has_open_threads`（L682）。いずれも boolean field を一つ指す。`switch` は同ファイルに 1 箇所ある。
- `docs/glossary/WORKFLOW.md`「rules と辺」節は `when.on` を「自 Node Artifact の Object を各段に沿って辿り、末端の直上 Object で required になっている boolean field」と定め、and / or の記述を持たない。「Lua API」節は `r.when{ on, on_true, next }` を記載する。
- `docs/glossary/DOMAIN.md` は述語、`when`、`Predicate` に関する記述を持たない。
- 既存テストが、builtin 8本の Error Diagnostic がゼロであること、および正本サンプルが Diagnostic ゼロで load でき実行木を構築できることを確認している。辺の評価のテストは、単一参照の true / false、多段参照、空白入り property 名、参照文字列の前後空白の拒否、Sequence / Fanout の map 経由の分岐を覆っている。

# Scope / Non-goals

## Scope

- 辺の `when` で、boolean field 参照を `and` / `or` で合成した述語と、そのネストを受理すること。
- 述語の実行時評価。論理演算と、参照先が未確定の参照を false として扱うこと。
- 述語に含まれる各参照が boolean を指すかの load 時型検査と、その Error Diagnostic。
- 単一の boolean field を指す従来の `when` の受理と遷移の維持。
- YAML と Lua の両方の定義表面。
- `docs/glossary/WORKFLOW.md` の「rules と辺」節と「Lua API」節の更新。
- `docs/glossary/DOMAIN.md` の正規語への述語の追加。
- `workflows/examples/full-cycle-development.yml` と `workflows/*.yml`（builtin 8本）が Diagnostic ゼロで load できる状態の維持。
- `and` / `or` / ネストの評価、参照先が未確定のときの false、boolean でない field を指した場合の Diagnostic、既存の単一 field `when` の回帰を対象とするテスト。

## Non-goals

- `completion` 側での述語の使用。`completion` の map 化と `require: approval` は #1732、`completion.delegate.when` は #1734 が扱う。
- `switch`。string enum の多分岐であり述語ではないため、受理形も評価も変えない。
- Node worktree 隔離（#1733）と Session delegate（#1734）。
- `and` / `or` 以外の演算子。否定、比較、計算、配列集約の式言語は導入しない。
- 参照の起点、段数、区切り記号、各段の解決規則。#1728 / #1729 / #1730 で確定した規則を変えない。述語の各参照は、現行の `when.on` と同じ起点と解決規則に従う。
- Contract の宣言規則。`schemas` に書ける型と `properties` / `required` / `items` / `enum` の意味は変えない。
- 辺のその他の規則。`then` / `next` の意味、判別規則・`loop_guard`・単独 `next` の個数制約、辺の target の制約、到達性と cycle の検査は変えない。
- 既存の builtin 8本と正本サンプルの辺の判断内容の変更。現行の 18 箇所の `when` は単一参照のまま従来と同じ経路を選ぶ。
- 正本サンプル `workflows/examples/full-cycle-development.yml` への `and` / `or` の例の追加。and / or の書き方は `docs/glossary/WORKFLOW.md` の「rules と辺」節が示す。
- workflow 定義の詳細表示（UI）での `and` / `or` を含む辺の表現。
- 変更前に保存された実行の定義 snapshot の読み出し互換。
- 辺の rules の構造の見直し（`then` の位置、`next` の二役、`switch` との対称性）。#1749 が扱う。

# Requirements

- R-001: 辺の `when` の `on` の値に、`and` を唯一のキーとする map を置き、その配列要素に boolean field 参照を並べて述語を書ける。実行時に、要素の全てが true のとき `then` が指す Node へ遷移し、それ以外のとき sibling `next` が指す Node へ進む。
- R-002: 辺の `when` の `on` の値に、`or` を唯一のキーとする map を置き、その配列要素に boolean field 参照を並べて述語を書ける。実行時に、要素のいずれかが true のとき `then` が指す Node へ遷移し、いずれも true でないとき sibling `next` が指す Node へ進む。
- R-003: `and` / `or` の配列要素には、boolean field 参照の文字列だけでなく、`and` / `or` を唯一のキーとする map を置いてネストできる。実行時の評価結果は、その論理式を通常の論理演算として評価した結果と一致する。
- R-004: 単一の boolean field 参照を `on` の値として書く従来の `when` は、受理形、load 時の検査、実行時の遷移のいずれも変更前と同じである。
- R-005: 述語に含まれる各参照は、現行の単一参照の `when.on` と同じ起点と解決規則に従う。自 Node の Artifact を起点にし、多段 field path、Sequence の統合 map と Fanout の map を経由する参照を、単一参照のときと同じ書き方で書ける。
- R-006: 述語に含まれる参照のいずれかが boolean を指さない場合、その定義は load 時に Error Diagnostic になり、load されない。boolean を指さないとは、末端 field が boolean でない、末端 field が直上 Object の `required` に含まれない、または field path のいずれかの段が解決できないことである。
- R-007: 実行時に参照先の値が存在しない、または boolean でない参照は、false として評価される。この規則は単一参照のときも `and` / `or` の要素のときも同じである。
- R-008: 述語は Lua 表面でも書ける。`r.all{ ... }` と `r.any{ ... }` がそれぞれ `and` / `or` の述語を返し、その要素には Source または述語を渡せる。`r.when` の `on` には Source または述語を渡せる。同じ意味の定義は YAML と同じ受理結果と同じ遷移になり、同じ定義上の誤りには同じ code、同じ stage、同じ message の Diagnostic が返る。Diagnostic の span は各表面の位置付けに従い、表面間で一致しなくてよい。
- R-009: `workflows/examples/full-cycle-development.yml` と `workflows/*.yml`（builtin 8本）は、本変更後も Diagnostic ゼロで load できる。
- R-010: `docs/glossary/WORKFLOW.md` の「rules と辺」節と「Lua API」節の記述が、変更後の辺の `when` の受理形、型検査、評価規則、および Lua の述語の書き方と一致する。`when.on` が boolean field を一つだけ指す前提の記述は残らない。
- R-011: `and` / `or` の配列要素は1つ以上である。要素が空の `and` / `or` を含む定義は load 時に Error Diagnostic になり、load されない。
- R-012: `docs/glossary/DOMAIN.md` の正規語に述語（Predicate）が定義され、辺と completion の判断に使う真偽値の論理式であること、原子が Artifact の boolean field 参照であること、`and` / `or` で合成することが読める。

# Assumptions / Open Questions

Assumption はない。

Open Question はない。
