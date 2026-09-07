# Context

- Primary source は GitHub Issue #1730「[Session delegate と Node worktree 隔離] Fanout の Artifact を map にする」（https://github.com/siro33950/releash/issues/1730 、state: OPEN、label: enhancement、milestone: #85、comment なし）である。
- Issue が「設計（正本）」として指定する `docs/specs/milestone-85/design.md` §4.2「Fanout」も Primary source である。関連して同 §4「Artifact 構造」、§5「参照」、§7「現行からの変更一覧」を参照する。GitHub Milestone #85「01. Session delegate と Node worktree 隔離」の説明文と記述が食い違う場合は design.md に従う（milestone #85 説明文の指示）。
- 追加資料は `docs/glossary/WORKFLOW.md`（「Node の Interface と children の配線」「children の4形式」「Sequence」「Fanout」「rules と辺」「Contract / schemas」の各節）、`docs/glossary/DOMAIN.md`、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）、`src-tauri/src/adaptor/gateway/workflow/builtin.rs`、`docs/specs/issues-1728/`、`docs/specs/issues-1729/`、`AGENTS.md`、`docs/architecture/`、および現行の Rust 実装と既存テストである。
- design.md §4.2 は次を定める。Fanout は children の Artifact を map として返す。キーは `items` の有無で決まり、`items` なしは child 名、`items` ありは添字である。`items` があると同じ child が複数展開されて名前で一意に引けないため添字をキーにする。`items` と複数 children が同時にある場合も展開は `(item_index, child_index)` のフラットな並びであり、階層は作らない。配列ではなく map にする理由は2つあり、`on_failure: ignore` で除外された slot があっても他のキーがずれないこと、および `isolated` の Node にメタデータを足す場所が必要なことである。§7 は Fanout を「children の map（キーは child 名または添字）」とする。
- 依存する #1728（参照の1段制限の撤廃）は commit ffbdb67f7 で main に取り込み済みである。Fanout の map から値を取る経路は `<fanout名>.<キー>.<field>` の多段になるため、本 Issue はこの解決規則の上に成り立つ。
- 同 Wave の #1729（Sequence の Artifact を children の統合 map にする）は commit 2467b1d0f で main に取り込み済みであり、Sequence は既に child 名キーの統合 map を宣言なしに産出する。Node ごとの参照解決用 schema を返す `reference::node_reference_schema` もこのとき新設され、配線 `inputs` / `when.on` / `switch.on` / `fanout.items` の3経路がこれを共有する。
- 本 Issue は Node の宣言構文を変更しない。`fanout` block の `children` / `items`、children エントリの `on_failure` の書き方はいずれも現行のままであり、変更対象は engine が組み立てる Artifact の構造と、その Artifact を起点にする参照の解決である。
- workflow 定義は YAML と Lua の2つの表面を持ち、どちらも同じ `WorkflowDefinition` を構築し、同じ定義上の誤りには同じ domain Diagnostic を使う（`docs/glossary/WORKFLOW.md`）。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。
- milestone #85 の全 ISSUE 共通の境界は次のとおりである。各 ISSUE は 1 PR で完結し、コード・Diagnostic・テストに加えて `docs/glossary/WORKFLOW.md` / `DOMAIN.md` の該当節、`workflows/examples/full-cycle-development.yml`、`workflows/*.yml`（builtin 8本）の更新を含む。各 ISSUE 完了時点で全定義が Diagnostic ゼロで load でき、テストが通る状態を保つ。したがって旧構造で書かれた既存定義の書き換えは本 Issue の PR に含まれ、旧構造の互換維持は前提にない。
- 本 Issue は Wave 2 に属する。同 Wave の #1731（述語の Predicate 共通化）、#1732（completion の map 化）、および Wave 3 の #1733（Node worktree 隔離）、#1734（Session delegate）は別 ISSUE である。
- 現在状態の確認は、Issue、milestone、design.md、用語集、現行コード、既存テスト、現行の workflow 定義の読解によって行った。build / test / lint などの検証コマンドは実行していない。
- `docs/specs/issues-1730` は未作成であり、本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash の workflow 定義を書く開発者、および milestone #85 の後続 ISSUE（特に #1733 Node worktree 隔離）を実装する担当者である。

現在、Fanout は children の Artifact を実行順の配列として返す。このため3つの問題がある。第一に、`on_failure: ignore` を宣言した child が失敗すると、その slot は配列から取り除かれ、後続の要素が前へ詰まる。下流は「何番目の要素がどの child／どの item の結果か」を位置から特定できなくなり、失敗の有無によって同じ添字が別の slot を指す。第二に、配列は child ごとに異なる Contract を表現できないため、Fanout を供給元にした field path 参照が load 時に Error Diagnostic になる。下流は Fanout の Artifact 全体を型なし input で受け取るしかなく、特定の child の値で分岐できない。第三に、slot ごとに engine 由来のメタデータを足す場所がない。

変更後、Fanout は children の Artifact を map として返す。キーは `items` の有無で決まり、`items` なしは children エントリ名、`items` ありは展開順の添字である。`on_failure: ignore` の失敗 slot はキーの欠番になり、残る slot のキーは変わらない。呼び出し側は `<fanout名>.<キー>.<field>` で必要な slot の値を引ける。

# Current Behavior

現行の Fanout の Artifact に関する挙動は次のとおりである。

| 対象 | 現行の挙動 |
| --- | --- |
| Fanout の Artifact | `artifact` を宣言せず、children の Artifact を配列として engine が組み立てる。`artifact` を宣言すると Error Diagnostic になる |
| 配列の並び | 展開した slot の宣言順。`items` なしは children の宣言順、`items` ありは `(item_index, child_index)` の item 優先のフラットな並び |
| `on_failure: ignore` の失敗 slot | 配列から取り除かれる。後続の要素が前へ詰まり、残る要素の添字が変わる |
| Artifact を産出しなかった slot | `null` 要素として残る。`artifact` を宣言しない child、および `on_failure` 宣言のない失敗がこれに当たる |
| slot が一つもない場合 | 空配列になる（`items` が空配列のとき） |
| Fanout を起点にした field path 参照 | 参照解決用 schema を持たないため Error Diagnostic になる。message は配線 `inputs` で ``source node '<node>' Artifact has no field path '<path>'``、`fanout.items` で ``node '<node>' has no Artifact field path '<path>'`` |
| Fanout を field path なしで受ける配線 | 成立する。Fanout は Artifact を産出する Node として扱われ、型なし input に配列がそのまま渡る |
| Sequence の統合 map 内の Fanout child | 参照解決用 schema に含まれない。`<sequence>.<fanout child>` およびその先の field path は Error Diagnostic になり、分岐の起点にもできない |
| Fanout の child 自身の参照 | 親へ集約されるため参照できない。message は ``node '<node>' is a fanout child and its Artifact is not referenceable`` |
| Fanout を自 Node とする辺の判別規則 | `when` / `switch` を書くと Error Diagnostic になる。code は `WFT006`、stage は `typecheck` である |
| 定義構文 | Fanout の Artifact 構造を宣言する field はない。YAML と Lua のどちらの表面にも該当 field がない |

最小の再現手順と結果は次のとおりである。実装は `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` の Fanout 集約（`serde_json::Value::Array` の組み立てと `is_ignored_failed_fanout_slot` による除外）にある。

1. `workflows/` に、children `a` と `b` を持つ `items` なしの Fanout `fan` を置き、`b` の children エントリに `on_failure: ignore` を宣言する。`a` と `b` はそれぞれ Artifact を宣言する Command とする。
2. 同じ Sequence の下流エントリに `results: fan` を配線する。
3. `b` だけが失敗すると、`fan` の Artifact は `[{<a の Artifact>}]` になる。`a` の結果は添字 0 に残り、`b` の slot は配列から消える。`b` が成功していれば `a` は同じ添字 0、`b` は添字 1 であり、失敗の有無で配列長が変わる。
4. 同じ定義で `results: fan.a` または `results: fan.a.<field>` と書くと、load 時に Error Diagnostic になり、その定義は load されない。特定の child の値を Fanout の外から名指しで取り出す書き方は存在しない。

現行の workflow 定義の実態は次のとおりである。

- `workflows/examples/full-cycle-development.yml` の Fanout は6つである。

    | Fanout | `items` | キーの決まり方（変更後） |
    | --- | --- | --- |
    | `implement_all`（L558） | `create_detailed_design.tasks` | 添字 |
    | `full_review_fanout`（L767） | なし | child 名（12 children） |
    | `verify_full_review_fanout`（L944） | なし | child 名（2 children） |
    | `fix_policy_fanout`（L987） | `scan_open_threads.threads` | 添字 |
    | `fix_policy_conflict_fanout`（L1032） | `check_fix_policy_consistency.tasks` | 添字 |
    | `fix_all`（L1080） | `create_fix_plan.tasks` | 添字 |

- 同ファイルで Fanout の Artifact を受けている配線は3つで、いずれも field path なしの型なし input である。`merge_implementations` の `results: implement_all`（L511）、`check_integration` の `results: implement_all`（L517）、`merge_fixes` の `results: fix_all`（L761）。L614 のコメントが受け取る値を「`implement_all` の子 Artifact 配列」と説明している。
- builtin 8本の Fanout は3つで、`workflows/02_implement-existing-spec.yml` の `implement_fanout`（L175、`items: create_detailed_design.tasks`）、`workflows/03_full-review.yml` の `full_review_fanout`（L76、`items` なし、12 children）と `verify_full_review_fanout`（L275、`items` なし、2 children）である。いずれも Artifact を受ける配線はなく、Fanout を供給元にした参照もない。
- `docs/glossary/WORKFLOW.md` は「Fanout」節の2行（children の Artifact は実行順の配列になる、`on_failure: ignore` の child は結果配列から除かれる）と「Contract / schemas」節（Fanout は child Artifact の配列を engine が組み立てるため `artifact` を宣言しない）で配列を前提にしている。加えて「Node の Interface と children の配線」節と「rules と辺」節が、Sequence の統合 map 内の Fanout child を参照先にも分岐の起点にもできないことを記述している。
- `docs/glossary/DOMAIN.md` は Fanout を Node kind と合成 Node の語として記述するだけで、Artifact 構造に関する記述を持たない。
- frontend は Fanout の定義構造（kind、children、items）を表示するだけで、Fanout の Artifact 構造に依存する処理を持たない。
- 既存テストが、builtin 8本を Error Diagnostic なく load できること、および正本サンプルが Diagnostic ゼロで load でき実行木を構築できることを確認している。実行木のテストは `implement_all` の集約結果を配列として期待し、`on_failure: ignore` の失敗 slot が集約から除かれることを配列長で確認している。
- 検証用 fixture `fanout-command-reducer.yml` と `fanout-session-reducer.yml` は、Fanout の Artifact を受けた集約 Node が判定結果を返す経路を持つ。Command 側は `echo '{{ reviews }}' | jq '{all_lgtm: all(.[]; .lgtm)}'` で畳む。

# Scope / Non-goals

## Scope

- Fanout の Artifact を map として engine が組み立てること。キーの決まり方（`items` なしは children エントリ名、`items` ありは展開順の添字）を含む。
- `items` と複数 children が同時にある場合の、階層を作らないフラットな展開とキーの割り当て。
- `on_failure: ignore` の失敗 slot をキーの欠番として扱い、他の slot のキーを変えないこと。
- Artifact を産出しなかった slot（`on_failure` 宣言のない失敗を含む）の値を `null` として残すこと。
- Fanout の Artifact を起点とする参照の解決。対象経路は配線 `inputs` の供給元、辺の述語 `when.on` / `switch.on`、`fanout.items` の供給元である。
- 合成子の Artifact を経由して Fanout の slot へ届く参照の解決。Sequence の統合 map を経由する `<sequence名>.<fanout名>.<キー>.<field>...` がこれに当たる。
- Fanout を自 Node とする辺の判別規則（`when` / `switch`）の受理。
- Fanout の Artifact 全体を field path なしで受ける既存の配線が引き続き成立すること。
- 参照元の書き換え。`workflows/examples/full-cycle-development.yml` と `workflows/*.yml`（builtin 8本）のうち、変更後の Artifact 構造および参照可能性と食い違う記述が対象である。
- `docs/glossary/WORKFLOW.md` の該当節の更新。「Fanout」節、「Contract / schemas」節、「Node の Interface と children の配線」節、「rules と辺」節が対象である。
- map の生成（キーの決まり方、フラットな展開、欠番、`null`）、Fanout を起点とする参照、集約 Node が map を受けて判定できること、正本サンプルと builtin 8本の load を対象とするテスト。

## Non-goals

- Sequence の Artifact 構造（#1729 で完了済み）。本 Issue は Sequence の統合 map の生成規則を変えない。
- `worktree` キーの付与と Node worktree 隔離（#1733）。map にする理由の一つは #1733 のメタデータの置き場所だが、キーの追加は本 Issue に含まない。
- 述語の `Predicate` への共通化と、辺での and / or の受理（#1731）。
- `completion` の map 化と `require: approval` の受理（#1732）。
- Session delegate と親 Artifact の `child` キー（#1734）。
- 参照の記法（#1728 で多段を解禁済み）。本 Issue は参照先の構造を変えるだけで、段の区切り記号、各段の文字種、段数の上限は変えない。
- Fanout の宣言構文。`children`、`items`、`on_failure` の書き方と、`items` を受ける child パラメータの束縛・型検査の規則は変えない。
- Fanout の child を、その child の Node 名で参照できるようにすること。現行の禁止（``node '<node>' is a fanout child and its Artifact is not referenceable``）を維持する。slot へは Fanout の Artifact のキーを経由して届く。
- `on_failure` の扱いそのもの。`ignore` が失敗を除外して続行すること、宣言のない失敗が中断することは変えない。
- Contract の宣言規則。`schemas` に書ける型と `properties` / `required` / `items` / `enum` の意味は変えない。Fanout が `artifact` を宣言できないことも変えない。
- `docs/glossary/DOMAIN.md`。Fanout の Artifact 構造に関する記述を持たないため、本 Issue の更新対象にならない。
- frontend。Fanout の Artifact 構造に依存する処理を持たないため、変更対象にならない。
- 旧構造（配列）の互換維持、および既存定義の自動移行。

# Requirements

- R-001: Fanout の Artifact は、展開した slot ごとの成果をキーで引ける map（JSON object）である。Fanout は `artifact` を宣言せず、engine が常にこの map を組み立てる。残る slot が一つもない場合は空の object になる。
- R-002: `items` を宣言しない Fanout のキーは、children エントリ名である。各キーの値は、その child の Artifact そのものである。
- R-003: `items` を宣言する Fanout のキーは、展開順の添字である。`items` と複数 children が同時にある場合も、展開は item を外側・child を内側とするフラットな並びであり、階層を作らない。
- R-004: `on_failure: ignore` を宣言した child の失敗 slot は、キーの欠番になる。他の slot のキーは、その失敗の有無によって変わらない。
- R-005: `on_failure: ignore` の失敗 slot 以外は、Artifact を産出しなかった場合もキーとして残り、その値は `null` になる。`on_failure` を宣言しない失敗と、`artifact` を宣言しない child がこれに当たる。
- R-006: 配線 `inputs` の供給元、辺の述語 `when.on` / `switch.on`、および `fanout.items` の供給元から、Fanout の slot の Artifact の値を引ける。Node 名を書く配線 `inputs` と `fanout.items` は `<fanout名>.<キー>.<field>...` の形になり、`items` なしの Fanout は `<fanout名>.<child名>.<field>`、`items` ありの Fanout は `<fanout名>.0.<field>` である。自 Node の Artifact を起点とする辺の述語は `<キー>.<field>...` の形になる。解決できる参照は Error Diagnostic なく load でき、実行時にその値が渡る。
- R-007: Fanout の Artifact を field path なしで受ける配線は引き続き成立し、受け側は変更前と同じ slot 集合の結果を map として受け取る。
- R-008: `workflows/examples/full-cycle-development.yml` と `workflows/*.yml`（builtin 8本）は、本変更後も Diagnostic ゼロで load できる。
- R-009: 書き換え後の定義は、書き換え前と同じ判断材料で同じ経路を選ぶ。`merge_implementations` と `check_integration` が受ける `implement_all` の結果、および `merge_fixes` が受ける `fix_all` の結果がこれに当たる。
- R-010: `docs/glossary/WORKFLOW.md` の記述が、変更後の Artifact 構造および参照可能性と一致する。Fanout の Artifact を配列とする前提の記述、および Fanout の Artifact が配列であることを根拠に参照可能性を制限する記述は残らない。
- R-011: Fanout が合成子の child である場合、その合成子の Artifact を経由した参照からも同じ slot を引ける。R-006 の3経路すべてで、Sequence の統合 map を経由する `<sequence名>.<fanout名>.<キー>.<field>...` が解決できる。
- R-012: Fanout を自 Node とする辺に判別規則（`when` / `switch`）を書ける。受理の可否は、述語が指す終端 field の型と `required` だけで決まる。

# Assumptions / Open Questions

Assumption はない。

Open Question はない。
