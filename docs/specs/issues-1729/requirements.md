# Context

- Primary source は GitHub Issue #1729「[Session delegate と Node worktree 隔離] Sequence の Artifact を children の統合 map にする」（https://github.com/siro33950/releash/issues/1729 、state: OPEN、label: enhancement、milestone: #85、comment なし）である。
- Issue が「設計（正本）」として指定する `docs/specs/milestone-85/design.md` §4「Artifact 構造」§4.1「Sequence」および §7「現行からの変更一覧」、ならびに GitHub Milestone #85「01. Session delegate と Node worktree 隔離」の説明文も Primary source である。記述が食い違う場合は design.md に従う（milestone #85 説明文の指示）。
- 追加資料は `docs/glossary/WORKFLOW.md`（「Node の Interface と children の配線」「children の4形式」「Sequence」「Fanout」「rules と辺」「Contract / schemas」「予約語と未解禁 field」「Lua」「Diagnostic」の各節）、`docs/glossary/DOMAIN.md`、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）、`src-tauri/src/adaptor/gateway/workflow/builtin.rs`、`docs/specs/issues-1728/`、`AGENTS.md`、`docs/architecture/`、および現行の Rust 実装と既存テストである。
- design.md §4.1 は次を定める。Sequence の `output` と `artifact` 宣言を廃止し、children の Artifact を child 名をキーとする map として返す。engine が組み立てるため Contract 宣言は不要になる（Fanout が Contract を宣言しないのと同じ仕組み）。通らなかった child は map に現れず、ループで複数回通った child は最後の結果が残る（現行の `sequence.artifacts` の挙動と同じ）。§7 は Sequence を「常に children の統合 map」とする。
- 依存する #1728（参照の1段制限の撤廃）は commit ffbdb67f7「feat(workflow): 参照の1段制限を撤廃し多段 field path を解決可能にする (#1728) (#1740)」で main に取り込み済みである。統合 map から値を取る経路は `<sequence名>.<子名>.<field>` の多段になるため、本 Issue はこの解決規則の上に成り立つ。
- workflow 定義は YAML と Lua の2つの表面を持ち、どちらも同じ `WorkflowDefinition` を構築し、同じ定義上の誤りには同じ domain Diagnostic を使う（`docs/glossary/WORKFLOW.md`）。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。
- milestone #85 の全 ISSUE 共通の境界は次のとおりである。各 ISSUE は 1 PR で完結し、コード・Diagnostic・テストに加えて `docs/glossary/WORKFLOW.md` / `DOMAIN.md` の該当節、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）の更新を含む。各 ISSUE 完了時点で全定義が Diagnostic ゼロで load でき、テストが通る状態を保つ。したがって旧記法で書かれた既存定義の書き換えは本 Issue の PR に含まれ、旧記法の互換維持は前提にない。
- 本 Issue は Wave 2 に属する。同 Wave の #1730（Fanout の Artifact を map にする）、#1731（述語の Predicate 共通化）、#1732（completion の map 化）、および Wave 3 の #1733（Node worktree 隔離）、#1734（Session delegate）は別 ISSUE である。
- 現在状態の確認は、Issue、milestone、design.md、用語集、現行コード、既存テスト、現行の workflow 定義の読解によって行った。build / test / lint などの検証コマンドは実行していない。
- `docs/specs/issues-1729` は未作成であり、本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash の workflow 定義を書く開発者、および milestone #85 の後続 ISSUE を実装する担当者である。

現在、Sequence は `output` で名指しした1つの child の Artifact をそのまま自分の Artifact として返す。そのため Sequence の外からは名指しした child 以外の結果を参照できず、複数の children の結果を同時に判断材料にできない。加えて、返す Artifact の Contract を Sequence 自身にも `artifact` として再宣言する必要があり、child の宣言と二重になる。`artifact` を宣言しない Sequence は Artifact を持たないため、配線の供給元にも辺の述語にも使えない。

変更後、Sequence は通った children の Artifact を child 名キーの map として常に返す。呼び出し側は `<sequence名>.<子名>.<field>` で必要な child の値を引く。map は engine が組み立てるため Contract 宣言は要らなくなり、`output` と Sequence の `artifact` は書けなくなる。

# Current Behavior

現行の Sequence の Artifact に関する挙動は次のとおりである。

| 対象 | 現行の挙動 |
| --- | --- |
| Sequence の Artifact | `output` が名指しする children エントリの Artifact をそのまま返す。Contract と result もその child のものを引き継ぐ |
| `artifact` 宣言と `output` の関係 | `artifact` を宣言した Sequence に `output` がなければ parse/shape 段の Error Diagnostic `WFS008`。message は ``sequence node '<node>' declares an artifact and must name the child that provides it via `output` `` |
| `output` の指す先 | children エントリ名を指していなければ resolve 段の Error Diagnostic `WFR001` |
| `output` と `on_failure: ignore` | `output` が `on_failure: ignore` の child を名指しすると control-flow 段の Error Diagnostic `WFC009` |
| `artifact` 宣言のある Sequence の終端 | `output` 子の Artifact がないまま終端へ到達すると、実行時に ValidationFailure として停止する |
| `artifact` を宣言しない Sequence | Artifact を産出しない。配線の供給元にも辺の述語にも使えない |
| `output` の予約 | `output` は Node 名に使えない予約語である |
| Lua 表面 | `r.sequence{ name?, entry?, output?, children, artifact?, input?, completion? }` が同じ2つの field を受ける |
| Fanout の Artifact | `artifact` を宣言せず、children の Artifact を実行順の配列として engine が組み立てる |

実行中の Sequence scope は、既に children の Artifact を child 名をキーとする map（`sequence.artifacts`）として保持している。通らなかった child はこの map に入らず、同じ child が複数回通れば後の結果で上書きされる。ただしこの map は scope 内の兄弟参照の解決と辺の評価に使われるだけで、Sequence 自身の Artifact にはならない。

最小の再現手順と実際の出力は次のとおりである。

1. `workflows/` に、children `a` と `b` がそれぞれ Artifact を宣言する Sequence `s` を置き、`s` に `artifact:` と `output: b` を宣言する。
2. 下流 Node の配線に供給元 `s.a` を書く。
3. Releash がその定義を load し Diagnostic を表示する経路（アプリの workflow 定義表示、または local API / CLI の diagnostics 経路）で結果を見ると、`s` の Artifact Contract は `b` の Contract であり `a` という field を持たないため Error Diagnostic になり、その定義は load されない。`a` の結果を Sequence の外から取り出す書き方は存在しない。

現行の workflow 定義の実態は次のとおりである。

- `workflows/examples/full-cycle-development.yml` で `output` と `artifact` を宣言する Sequence は5つである。

    | Sequence | `output` | `artifact` |
    | --- | --- | --- |
    | `authoring_behavior`（L294） | `write_behavior` | `behavior-authoring-result` |
    | `authoring_design`（L331） | `write_design` | `design-authoring-result` |
    | `implement_and_verify`（L576） | `verify_task` | `implement-task-check-result` |
    | `review_scan`（L708） | `check_full_review_threads` | `thread-scan` |
    | `fix_and_verify`（L1102） | `verify_fix` | `fix-verification` |

- builtin 8本で `output` と `artifact` を宣言する Sequence は `workflows/05_review-fix.yml` の `fix_round`（L132、`output: close_round` / `artifact: fix-verification`）だけである。他の7本の Sequence はどちらも宣言しない。
- これらの Sequence の Artifact を外から参照している箇所は次のとおりである。
    - `workflows/examples/full-cycle-development.yml` L278: `authoring` の child `authoring_design` の配線 `behavior: authoring_behavior`。Sequence の Artifact 全体を受ける型なし input である。
    - `workflows/examples/full-cycle-development.yml` L688: `review` の child `review_scan` の辺 `when.on: has_open_threads`。`thread-scan` Contract の required boolean field で分岐する。
    - `workflows/05_review-fix.yml` L125: `main` の child `fix_report` の配線 `verify_fixes: fix_round`。Sequence の Artifact 全体を受ける型なし input である。
    - `implement_and_verify` と `fix_and_verify` は Fanout の child であり、その Artifact は Fanout の結果配列の要素になる。配列は `results: implement_all`（L515 / L521）と `results: fix_all`（L769）で下流 Session に渡る。
    - `authoring_design` の Artifact は、どこからも参照されていない。
- 既存テストが、builtin 8本の Error Diagnostic がゼロであること、および正本サンプルが Diagnostic ゼロで load でき実行木を構築できることを確認している。

# Scope / Non-goals

## Scope

- Sequence の `output` field の廃止と、宣言を Error Diagnostic として拒否すること。
- Sequence の `artifact` 宣言の廃止と、宣言を Error Diagnostic として拒否すること。
- Sequence の Artifact を、通った children の Artifact を child 名キーとする map として engine が組み立てること。
- 統合 map を起点とする多段参照（配線 `inputs` の供給元、辺の述語 `when.on` / `switch.on`、`fanout.items` の供給元）の解決。
- `output` の廃止に伴う既存検査の削除と置換（`artifact` 宣言時の `output` 要求、`output` の名指し先の解決、`on_failure: ignore` の child を `output` が名指しする `WFC009`）。
- `output` の Node 名予約からの除外。
- YAML と Lua の両方の定義表面。
- 参照元の書き換え。`workflows/examples/full-cycle-development.yml` の5つの Sequence とその参照元、および builtin 8本。
- `docs/glossary/WORKFLOW.md` の該当節の更新。「Sequence」節、「Node の Interface と children の配線」の Node 共通 field の表、「予約語と未解禁 field」、「rules と辺」、「Contract / schemas」、「Lua API」の表が対象である。
- 統合 map の生成（通らなかった child、複数回通った child）、`output` / `artifact` 宣言の Diagnostic、多段参照経由の辺の分岐と配線、正本サンプルの load を対象とするテスト。

## Non-goals

- Fanout の Artifact 構造の変更（#1730）。本 Issue の完了時点でも Fanout の Artifact は children の実行順配列のままである。
- `worktree` キーの付与と Node worktree 隔離（#1733）。
- 述語の `Predicate` への共通化と、辺での and / or の受理（#1731）。
- `completion` の map 化と `require: approval` の受理（#1732）。
- Session delegate（#1734）。
- 参照の段数の規則（#1728 で解禁済み）。本 Issue は参照先の構造だけを変え、参照記法と各段の解決規則は変えない。
- Contract の宣言規則。`schemas` に書ける型と `properties` / `required` / `items` / `enum` の意味は変えない。
- Sequence の `entry`、children の4形式、辺の規則、`on_failure` の規則。`output` に関する部分を除いて変えない。
- `docs/glossary/DOMAIN.md`。Sequence の Artifact 構造に関する記述を持たないため、本 Issue の更新対象にならない。
- 旧記法（`output` / Sequence の `artifact`）の互換維持、および既存定義の自動移行。

# Requirements

- R-001: Sequence の Artifact は、その Sequence 実行で通った children の Artifact を、children エントリ名をキーとする map である。各キーの値は、その child の Artifact そのものである。
- R-002: その Sequence 実行で通らなかった child、および Artifact を産出しなかった child は、map にキーとして現れない。
- R-003: 同じ child が一つの Sequence 実行の中で複数回通った場合、map にはその child の最後の結果が残る。
- R-004: Sequence は宣言なしに常にこの map を Artifact として産出する。Sequence の Artifact に Contract 名は結び付かない。
- R-005: 配線 `inputs` の供給元、辺の述語、および `fanout.items` の供給元から、`<sequence名>.<子名>.<field>...` の多段参照で Sequence の child の Artifact の値を引ける。解決できる参照は Error Diagnostic なく load でき、実行時にその値が渡る。
- R-006: Sequence の `output` 宣言は Error Diagnostic になり、その定義は load されない。YAML と Lua のどちらの定義表面でも拒否される。
- R-007: Sequence の `artifact` 宣言は Error Diagnostic になり、その定義は load されない。YAML と Lua のどちらの定義表面でも拒否される。
- R-008: `workflows/examples/full-cycle-development.yml` と `workflows/*.yml`（builtin 8本）は、本変更後も Diagnostic ゼロで load できる。
- R-009: 書き換え後の定義は、書き換え前と同じ判断材料で同じ経路を選ぶ。`review_scan` の `has_open_threads` による辺の選択、`authoring_behavior` と `fix_round` の Artifact を受ける下流の配線、`implement_and_verify` と `fix_and_verify` の結果を受ける下流の配線がこれに当たる。
- R-010: `docs/glossary/WORKFLOW.md` の記述が、変更後の構文および Artifact 構造と一致する。`output` と Sequence の `artifact` を書ける前提の記述は残らない。
- R-011: `output` は Node 名の予約語ではなくなる。`output` という名前の Node を宣言した定義は、その名前を理由に Error Diagnostic にならない。

# Assumptions / Open Questions

Assumption はない。Open Question はない。
