# Context

- Primary source は GitHub Issue #1732「[Session delegate と Node worktree 隔離] completion を map にし require: approval を受理する」（https://github.com/siro33950/releash/issues/1732 、state: OPEN、label: enhancement、milestone: #85、comment なし）である。
- Issue が「設計（正本）」として指定する `docs/specs/milestone-85/design.md` §2.3「構文」も Primary source である。関連して同 §2.2「Session が所有する理由」、§7「現行からの変更一覧」の「`completion`」行を参照する。GitHub Milestone #85「01. Session delegate と Node worktree 隔離」の説明文と記述が食い違う場合は design.md に従う（milestone #85 説明文の指示）。
- 追加資料は `docs/glossary/WORKFLOW.md`（「Node の Interface と children の配線」「Session」「completion」「予約語と未解禁 field」「Lua」「Lua API」「Diagnostic」の各節）、`docs/glossary/DOMAIN.md`、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）、`src-tauri/src/adaptor/gateway/workflow/builtin.rs`、`docs/specs/issues-1729/`、`docs/specs/issues-1730/`、`docs/specs/issues-1731/`、`AGENTS.md`、`docs/architecture/`、および現行の Rust 実装と既存テストである。
- design.md §2.3 は次を定める。`completion` は map であり、`require` と `delegate` を並べる。`require` は任意で全 Node 種別で宣言でき、値域は `approval` である。要求しないなら `require` を書かない。現行の `completion: auto` / `completion: approval` という文字列形式は廃止し、`auto` という値も持たない（要求を書かないことが自動完了を意味する）。`delegate` は Session だけが宣言でき、`require` と両方あるときの意味は and である。

    ```yaml
    implement:
      session:
        provider: codex
        facets:
          instruction: implement
      artifact: implement_result
      completion:
        require: approval
    ```

- design.md §2.2 は、delegate を completion の一種であり `approval` と同型（本来の完了条件に条件を足して完了を保留する）と位置づける。Issue はこれを境界の理由とし、`completion` が map であることが delegate を置く場所の前提になるため、delegate 本体とは独立に既存の approval だけで先に構造を変えると定める。
- Issue の「含むもの」には、`NodeCompletion` 型を「要求の有無」に変えるという実装上の指定が含まれる。本文書はこれを要求として扱わず、Design で扱う。
- Issue は「正本サンプル2箇所（`spec_confirmation` / `implementation_confirmation`）と builtin 8本の書き換え」と記す。現行の `completion: approval` 宣言は、正本サンプルの2箇所と、builtin 8本のうち5本（`01_author-spec` 1箇所、`02_implement-existing-spec` 1箇所、`04_review-fix-policy-manual` 2箇所、`06_handle-pr-review` 1箇所、`06_handle-pr-review-manual` 2箇所）の計7箇所に存在する。`03_full-review` / `04_review-fix-policy` / `05_review-fix` には `completion` 宣言がなく、`completion: auto` の明示宣言はどの定義にもない。宣言のない Node は変更前後とも自動完了であり書き換える箇所がないため、「builtin 8本の書き換え」は、宣言を持つ5本の書き換えと、8本すべてが Diagnostic ゼロで load でき承認の挙動が変わらないこと（Issue の受け入れ基準）として扱う。
- Issue は「YAML と Lua の双方で同じ受理形にする」と要求し、テスト項目に「Lua での同値性」を挙げる。design.md §2.3 は YAML の表記だけを示し、map 形に対応する Lua の表記は確認した正本のどこにも定義されていない。現行の Lua DSL は `r.completion.approval` を Completion として定義している（`docs/glossary/WORKFLOW.md`「Lua API」）。
- 同 Wave 2 の #1729（Sequence の Artifact を統合 map にする）は commit 2467b1d0f、#1730（Fanout の Artifact を map にする）は commit 52f5b862d、#1731（述語の Predicate 共通化）は commit 94963548d で main に取り込み済みである。#1731 の spec は「`completion` の map 化と `require: approval` は #1732」を Non-goal とし、本 Issue と独立であると記す。
- 後続の #1734（Session delegate）は本 Issue に依存し、`completion` が map であることを前提に `completion.delegate` を追加する。`delegate` の構文と挙動は #1734 が扱う。
- workflow 定義は YAML と Lua の2つの表面を持ち、どちらも同じ `WorkflowDefinition` を構築し、同じ定義上の誤りには同じ `code`・`stage`・`message` の domain Diagnostic を使う。`span` は各表面の位置付けに従う（`docs/glossary/WORKFLOW.md`「Diagnostic」）。定義は未知 field、旧形式、互換 alias を受理せず、Error Diagnostic が一つでもある定義は実行できない（同「Workflow 定義」冒頭）。
- Lua の型注釈 stub（`<workflows_dir>/.releash/releash.lua`）は Releash が生成する。Lua 表面の受理形を変えると stub も同時に変わる。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。
- milestone #85 の全 ISSUE 共通の境界は次のとおりである。各 ISSUE は 1 PR で完結し、コード・Diagnostic・テストに加えて `docs/glossary/WORKFLOW.md` / `DOMAIN.md` の該当節、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）の更新を含む。各 ISSUE 完了時点で全定義が Diagnostic ゼロで load でき、テストが通る状態を保つ。したがって旧形式で書かれた既存定義の書き換えは本 Issue の PR に含まれ、旧形式の互換維持は前提にない。
- 現在状態の確認は、Issue、milestone、design.md、用語集、現行コード、既存テスト、現行の workflow 定義の読解によって行った。build / test / lint などの検証コマンドは実行していない。
- `docs/specs/issues-1732` は本文書の作成時点まで未作成であり、本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash の workflow 定義を YAML または Lua で書く開発者、および後続 #1734 で `completion.delegate` を実装する担当者である。

現在、`completion` は `auto` または `approval` の文字列であり、Node の完了に付ける条件を一つの列挙値で表す。この形では、承認要求と delegate のように独立した条件を同じ Node に並べて書く場所がない。また、`auto` という値が「条件を付けない」ことを明示的な値として持つため、「要求を書かないことが自動完了である」という規則と、列挙値としての `auto` が二重に存在する。

変更後、`completion` は map になる。承認を要求する Node は `require: approval` を書き、要求しない Node は `completion` を書かない。`auto` という値はなくなり、文字列形式の `completion: approval` / `completion: auto` は Error Diagnostic になる。承認の実行時の挙動は現行と変わらず、全4種の Node で `require: approval` を宣言できる。YAML と Lua は同じ意味の completion を同じ受理結果で宣言できる。定義の read model と UI も、廃止した文字列形式ではなく変更後の受理形で `completion` を示す。

# Current Behavior

現行の `completion` に関する挙動は次のとおりである。

| 対象 | 現行の挙動 |
| --- | --- |
| `completion` の受理形 | 文字列 `auto` または `approval`。省略時は `auto`。全4種の Node で宣言できる |
| YAML に map を書く | `completion: { require: approval }` は、`completion` の値が文字列でないため parse/shape 段の Error Diagnostic `WFS002` になり、その定義は load されない。message は `workflow shape error:` に続けて deserialize の失敗内容を含む |
| YAML に `auto` / `approval` 以外の文字列を書く | `unknown variant` として parse/shape 段の Error Diagnostic `WFS002` になり、その定義は load されない |
| 実行時の意味 | Session は Submit と provider Stop の二信号が揃ったとき、Command は process 終了、Fanout は全 child の決着、Sequence は終端到達が本来の完了条件である。`auto` はその時点で完了する。`approval` は本来の完了条件を満たした後に WaitingApproval となり、人間の Approve で完了する |
| Lua 表面 | `completion = r.completion.approval` で approval を宣言する。省略時は `auto`。`r.completion.approval` 以外の値を渡すと parse/shape 段の Error Diagnostic `WFS002`（``field 'completion' must be Completion``）になる。Lua で `auto` を明示する表記はない |
| Lua の型注釈 stub | Releash が生成する `.releash/releash.lua` が `ReleashCompletion` 型と `r.completion.approval` を注釈する |
| 定義の read model と UI | 定義の DTO は `completion` を `"auto"` / `"approval"` の文字列で返す。workflow 定義の詳細表示は `approval` の Node に `completion: approval` の badge を付け、設定画面の Approval auto-approve の説明文は `completion: approval` の文字列形式を引用する |
| 保存済み実行 | 実行木の root の開始事実に定義の snapshot が保存され、`completion` が `approval` の Node だけ `completion: "approval"` を持つ（`auto` は省略）。読み出し時に解釈できない Node 定義は Node 単位で扱われる（#1744） |
| Diagnostic | `completion` は Node 共通 field として許可され、Node 名に使えない予約語である |

最小の再現手順と結果は次のとおりである。

1. `workflows/` に、Session Node `review` の `completion` に map を書いた定義を置く。

    ```yaml
    nodes:
      main:
        sequence:
          children:
            - review
      review:
        session:
          provider: claude
          facets:
            instruction: review-diff
        completion:
          require: approval
    ```

2. Releash がその定義を load し Diagnostic を表示する経路（アプリの workflow 定義表示、または local API / CLI の `releash workflow diagnostics`）で結果を見る。
3. parse/shape 段の Error Diagnostic `WFS002` になり、その定義は load されない。同じ定義の `completion` を文字列 `approval` に置き換えると Diagnostic なく load され、`review` は Submit と provider Stop の後に WaitingApproval となる。

現行の workflow 定義の実態は次のとおりである。

- `workflows/*.yml`（builtin 8本）の `completion: approval` は7箇所である。`01_author-spec` 1、`02_implement-existing-spec` 1、`03_full-review` 0、`04_review-fix-policy` 0、`04_review-fix-policy-manual` 2、`05_review-fix` 0、`06_handle-pr-review` 1、`06_handle-pr-review-manual` 2。いずれも Session Node に宣言されている。
- `workflows/examples/full-cycle-development.yml` の `completion: approval` は2箇所（`spec_confirmation`、`implementation_confirmation`）であり、いずれも Session Node に宣言されている。
- `completion: auto` の明示宣言は builtin と正本サンプルのどこにもない。
- `docs/glossary/WORKFLOW.md` は、Node 共通 field の表で `completion` を「`auto` または `approval`。省略時は `auto`」と定め、「Session」節の例と説明で `completion: approval` / `completion: auto` を、「completion」節で `auto` / `approval` を列にした Node 種別ごとの表を、「Lua」節の例と「Lua API」の表で `r.completion.approval` を記載する。
- `docs/glossary/DOMAIN.md` は正規語の表で `completion` を「Node 自身が持つ完了の定義」と定め、述語（Predicate）を「辺と completion の判断に使う真偽値の論理式」と定める。
- 既存テストが、builtin 8本の Error Diagnostic がゼロであること、正本サンプルが Diagnostic ゼロで load でき実行木を構築できること、Lua の `r.completion.approval` が approval として解釈されることを確認している。

# Scope / Non-goals

## Scope

- `completion` を map として受理し、`require: approval` を全4種の Node で宣言できること。
- `require: approval` を宣言した Node の承認の挙動が、現行の `completion: approval` と同じであること。
- `completion` を省略した Node が自動完了すること。
- 文字列形式 `completion: approval` / `completion: auto` を Error Diagnostic として拒否すること。
- `require` の値域、`completion` map の未知キー、および要素を持たない `completion` map の拒否。
- YAML と Lua の両方の定義表面。Lua は `completion = { require = r.completion.approval }` の table 形で宣言する。
- 正本サンプル2箇所と、`completion` 宣言を持つ builtin 5本の書き換え。builtin 8本と正本サンプルが Diagnostic ゼロで load できる状態の維持。
- `docs/glossary/WORKFLOW.md` の Node 共通 field の表、「Session」節、「completion」節、「Lua」節、「Lua API」の表の更新。
- `docs/glossary/DOMAIN.md` の `completion` 定義の更新。
- 全4種の Node での `require: approval`、旧文字列形式の Diagnostic、`completion` 省略時の自動完了、Lua での同値性を対象とするテスト。
- 定義の read model の `completion` の値形と、workflow 定義詳細の Node の badge、設定画面の Approval auto-approve の説明文の文言。

## Non-goals

- `completion.delegate`（#1734）。本 Issue では `delegate` は `completion` map の未知キーとして扱う。
- Node worktree 隔離（#1733）。
- 辺の `when` と述語。#1731 で確定した規則を変えない。
- 承認の実行時経路。Approve / Reject の操作、Approval auto-approve 設定、WaitingApproval の状態遷移、承認事実の記録は変えない。
- 各 Node 種別の本来の完了条件（Session の二信号、Command の process 終了、Fanout の全 child 決着、Sequence の終端到達）。
- `require` の値の追加。値域は `approval` だけである。
- `docs/specs/milestone-85/design.md` の変更。design.md は既に変更後の構文を記載している。
- リポジトリ外にあるユーザー定義の workflow の書き換え。
- 変更前に保存された実行の定義 snapshot の読み出し互換。`completion: "approval"` の文字列を持つ旧 snapshot の Node は、#1744 が定めた保存済み実行の規則に従い、Node 単位で解釈できない定義として扱われる。旧形式の解釈経路と自動移行は追加しない。

# Requirements

- R-001: `completion` は map として宣言でき、`require: approval` を持つ map を Session / Command / Fanout / Sequence の全4種の Node で受理する。受理された定義は `completion` に起因する Error Diagnostic なく load できる。
- R-002: `require: approval` を宣言した Node は、本来の完了条件を満たした後に WaitingApproval となり、人間の Approve で完了する。この挙動は、変更前に `completion: approval` を宣言した Node の挙動と同じである。
- R-003: `completion` を宣言しない Node は、本来の完了条件を満たした時点で完了する。この挙動は、変更前に `completion` を省略した Node の挙動と同じである。
- R-004: 文字列形式の `completion: approval` と `completion: auto` は load 時に Error Diagnostic になり、その定義は load されない。
- R-005: `require` の値は `approval` だけを受理する。`approval` 以外の値（`auto` を含む）を持つ `require`、`require` 以外のキーを持つ `completion` map、および要素を持たない `completion` map は load 時に Error Diagnostic になり、その定義は load されない。`completion` を書くなら要求を一つ以上持つ。
- R-006: Lua 表面では、YAML の `completion` map と同形の table `completion = { require = r.completion.approval }` で `require: approval` と同じ意味の completion を宣言でき、YAML と同じ受理結果と同じ完了の挙動になる。`r.completion.approval` は `require` の値としてだけ受理され、table で包まない `completion = r.completion.approval` は YAML の文字列形式と同じく load 時に Error Diagnostic になり、その定義は load されない。同じ定義上の誤りには YAML と同じ `code`、`stage`、`message` の Diagnostic が返る。Diagnostic の `span` は各表面の位置付けに従い、表面間で一致しなくてよい。
- R-007: `workflows/examples/full-cycle-development.yml` の `spec_confirmation` / `implementation_confirmation` と、`completion` 宣言を持つ builtin 5本の `completion: approval` は `require: approval` の map 形に書き換えられ、builtin 8本と正本サンプルは本変更後も Diagnostic ゼロで load できる。書き換えた Node の承認の挙動は変更前と同じである。
- R-008: `docs/glossary/WORKFLOW.md` の Node 共通 field の表、「Session」節、「completion」節、「Lua」節、「Lua API」の表の記述が、変更後の `completion` の受理形、省略時の意味、Node 種別ごとの承認の挙動、および Lua の書き方と一致する。`auto` / `approval` の文字列形式を前提とする記述は残らない。
- R-009: `docs/glossary/DOMAIN.md` の正規語 `completion` の定義が、`completion` が Node の完了に対する要求の集合であり、要求として `require: approval` を持ち、要求を書かないことが自動完了を意味することと一致する。
- R-010: 定義の read model（Tauri と local API が読む定義の DTO）と UI（workflow 定義詳細の Node の badge、設定画面の Approval auto-approve の説明文）は、`auto` / `approval` の文字列形式を前提とせず、`require: approval` の宣言の有無を変更後の受理形と一致する形で示す。廃止した `auto` は read model の値として残らない。

# Assumptions / Open Questions

Assumption はない。

Open Question はない。
