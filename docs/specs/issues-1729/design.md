# Design

## The actual design

### Architecture

#### 統合 map は scope 状態から engine が組み立てる

Sequence スコープは既に children の Artifact を child 名キーの map（`SequenceScopeRuntime.artifacts`）として持ち、`scope_resolution_space` と辺の評価がそれを使っている。R-001 の統合 map は、この既存状態を Sequence インスタンス確定時（`complete_scope`）に JSON object へ写したものとする。新しい保持先は作らない。

R-002 と R-003 はこの写し方から従う。

- 通らなかった child は `artifacts` に入らないため、キーにならない。
- Artifact を産出しなかった child は `RuntimeArtifact.artifact` が `None` で入るため、`Some` のものだけを写す。
- 子の起動時に `artifacts` から自分のキーを除く現行の規則（`start_node_instance` / replay の両方）があるため、複数回通った child は最後の visit の結果だけが残る。

`output` 子の Artifact が無いまま終端へ到達したときの ValidationFailure 停止は削除する。キーが1つも立たない終端は空 map という正当な結果になり（B-002）、失敗にする根拠が無くなるためである。

#### 参照解決の起点 schema は Node ごとに一つの関数が返す

「その Node の Artifact に対する field path を、どの schema で解決するか」を現行は3箇所が各自持っている。`reference::artifact_field_schema`（`fanout.items`）、`validation::validate_node_source_field_path`（配線 `inputs`）、`routing::validate_routing_field`（`when.on` / `switch.on`）である。3箇所とも「Command なら予約 field を合成した schema、それ以外は `artifact` の Contract」という同じ分岐を書いている。

Sequence の合成 map をここへ足すと同じ分岐が3箇所で4通りになるため、`services/reference.rs` に `node_reference_schema(workflow, node)` を新設し、3箇所はこれを呼ぶだけにする。置き場所を `reference.rs` にするのは、Sequence の合成が children を辿るために `WorkflowDefinition` を必要とし、`contract_schema` を schema 代数のまま保つためである。`command_reference_schema` は `contract_schema` に残し、新関数がその中で使う。

関数が返す失敗は「Artifact Contract が Object でない」「参照解決できる Artifact を持たない」の2種別だけにし、Diagnostic の message は現行どおり各呼び出し側が組み立てる。message は呼び出し側ごとに文面が異なり（`source node '...'` / `routing field '...'`）、既存 fixture が code とともに固定しているため、所在を動かさない。

#### 合成 schema の property は Artifact を産出する children だけ

Sequence の参照解決用 schema は、children エントリ名を property、その child の参照解決用 schema を値とする Object とする。次の child は property にしない。

- Artifact を産出しない child（`artifact` 宣言の無い Session）。実行時にもキーが立たない。
- Fanout child。Fanout の Artifact は children の配列であり、`SchemaDef::Array { items }` の `items` が名前付き Contract 参照1つであるため、children ごとに Contract が異なる配列を表現できない。Fanout を直接の供給元にしたときに field path が引けない現行の扱いと揃う。

`required` は空にする。R-002 により、どの child も通るとは限らないためである。`when.on` / `switch.on` の required 判定は終端段の直上 Object に対して行われるので、`a.has_open_threads` は `a` の Contract の `required` で判定され、B-006 / B-007 は成立する。

children に Sequence がある場合は再帰する。合成子の包含 cycle は `WFC008` で拒否されるが、Diagnostic 経路は `validate_all` で全 error を集めるため cycle のある定義でも配線検証が走る。合成は訪問済み node 名の集合を持ち、再訪した child は property にしないことで停止する。

#### 判別規則の「Artifact を持たない child」判定を寄せる

`routing::validate_entry_rules` の `DiscriminatorWithoutArtifact` は `child.artifact.is_none() && !child.is_command()` で判定しており、`artifact` を宣言しない Sequence child を弾く。これを `!reference::node_has_artifact(child)` に変える。`node_has_artifact` は Sequence を常に真にする（R-004）。Fanout child への判別規則は `DiscriminatorOnFanout` が引き続き別に拒否する。

#### 廃止した2つの宣言は置き場所が違う

`output` は `sequence` block 固有の field である。`SequenceSpec` / `RawSequenceSpec` / Lua の Sequence draft から削除すると、YAML は `deny_unknown_fields`、Lua は `reject_unknown` が拒否し、どちらも `WFS002` / `parse_shape` になる（B-008 / B-010）。廃止した field を model に残さない。

`artifact` は全 kind 共通の Node field であり、parse 段では kind を判別できない。両表面とも受理したうえで、共有 domain validation に `ValidationError::SequenceArtifactDeclaration` を新設して拒否する（B-009 / B-010）。`worktree` を受理して `WFU002` で拒否する現行と同じ形であり、YAML と Lua が同じ domain Diagnostic になる。

これに伴い、`ValidationError::SequenceArtifactRequiresOutput`、`ValidationError::SequenceOutputNotChild`、`routing::RoutingValidationError::SequenceOutputNotChild`、および `collect_on_failure_errors` の「`output` が `on_failure: ignore` の child を名指しする」節（`WFC009`）を削除する。`WFC009` の残り3つの依存（同一スコープの兄弟 `inputs`、その entry 自身の `when` / `switch`、兄弟 Fanout の `items`）はスコープに閉じた規則のまま変えない。統合 map から `ignore` child のキーが落ちることを親スコープ側で検査する規則は、要求が無いため追加しない。

#### `output` を予約 Node 名から外す

予約 Node 名は、children の形式④（kind または Node 共通 field から始まる無名インライン宣言）と Node 名が衝突しないための規則である。`output` は kind block の field でも Node 共通 field でもなくなるため、`RESERVED_NODE_NAMES` から外す（R-011）。`artifact` は Session / Command の Node 共通 field として残るため予約のままにする。

#### 主要な変更対象

| path | 担う変更 |
| --- | --- |
| `src-tauri/src/domain/workflow/value_objects/definition.rs` | `SequenceSpec` / `RawSequenceSpec` からの `output` 削除、予約 Node 名からの `output` 除外 |
| `src-tauri/src/domain/workflow/services/reference.rs` | Node の参照解決用 schema を返す関数の新設と Sequence の合成、`node_has_artifact` の Sequence 分岐 |
| `src-tauri/src/domain/workflow/services/validation.rs` | Sequence の `artifact` 宣言の拒否、`output` 関連検査の削除、配線 field path の起点の差し替え |
| `src-tauri/src/domain/workflow/services/routing.rs` | `output` の名指し検査の削除、routing field の起点の差し替え、判別規則の Artifact 有無判定 |
| `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` | Sequence インスタンスの成果を統合 map にする、`output` 子欠落の失敗経路の削除 |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` | 新 ValidationError の code / stage / span / field 対応と旧 variant の除去 |
| `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs` | `r.sequence` の受理 field と Sequence draft からの `output` 除去 |
| `src-tauri/src/adaptor/gateway/workflow/lua/stubs.rs` | LuaLS stub の `output` 除去 |
| `src-tauri/src/adaptor/gateway/workflow/mapper.rs`、`src-tauri/src/usecase/workflow/dto.rs` | `output` の写しの削除 |
| `src/types/workflow.ts`、`src/components/panels/automation/WorkflowDetail.tsx` | `output` の型と表示行の削除 |
| `workflows/examples/full-cycle-development.yml`、`workflows/05_review-fix.yml` | 宣言の削除と参照の書き換え |
| `docs/glossary/WORKFLOW.md` | Sequence / Node の Interface と children の配線 / rules と辺 / 「Contract / schemas」 / 予約語と未解禁 field / Lua API の各節 |

#### 既存定義の書き換え

R-009 が維持を求める判断材料は、いずれも「これまで `output` が名指ししていた children エントリの Artifact」である。したがって参照の書き換えは、供給元に child 名の1段を挿し込む形にし、Node の構成そのものは変えない。

| 定義 | 削除する宣言 | 参照の書き換え |
| --- | --- | --- |
| `authoring_behavior` | `output: write_behavior` / `artifact: behavior-authoring-result` | `authoring` の `authoring_design` エントリの `behavior: authoring_behavior` を `authoring_behavior.write_behavior` にする |
| `authoring_design` | `output: write_design` / `artifact: design-authoring-result` | 参照元なし |
| `review_scan` | `output: check_full_review_threads` / `artifact: thread-scan` | `review` の `review_scan` エントリの `when.on: has_open_threads` を `check_full_review_threads.has_open_threads` にする |
| `implement_and_verify` | `output: verify_task` / `artifact: implement-task-check-result` | 配線は変えない（下記） |
| `fix_and_verify` | `output: verify_fix` / `artifact: fix-verification` | 配線は変えない（下記） |
| `fix_round`（`workflows/05_review-fix.yml`） | `output: close_round` / `artifact: fix-verification` | `main` の `fix_report` エントリの `verify_fixes: fix_round` を `fix_round.close_round` にする |

`implement_and_verify` と `fix_and_verify` は Fanout の child であり、その Artifact は Fanout が配列へ集約する。配列の要素に child 名の1段を挿し込む書き方は無いため、配線は変えられない。両者の子のうち `implement_task` と `fix_task` は `artifact` を宣言しないため統合 map のキーにならず、要素は `{ "verify_task": ... }` / `{ "verify_fix": ... }` の単一キー map になる。下流の `results: implement_all` / `results: fix_all` は型なし input で Session の判断材料になるだけで辺の条件にならないため、B-014 の維持対象である判断材料は1段深い位置で保たれる。

### Interface

#### 定義表面

Sequence の kind block が受ける field は `entry` と `children` だけになる。`output` と Sequence の `artifact` は Error Diagnostic になり、その定義は load されない。

Sequence を供給元にする参照は `<sequence>`（統合 map 全体）と `<sequence>.<child>.<field>...` になる。段の区切り記号、各段の文字種、段数の規則は #1728 のまま変えない。Sequence child に置く `when.on` / `switch.on` は、field path の1段目が children エントリ名になる。

#### 記述と Diagnostic の対応

| 記述 | code / stage | 拒否する場所 |
| --- | --- | --- |
| Sequence の `output` | `WFS002` / `parse_shape` | YAML は未知 field、Lua は未知 field |
| Sequence の `artifact` | `WFS008` / `parse_shape` | 共有 domain validation |
| `<sequence>.<child>.<field>` が配線で解決できない | `WFR007` / `resolve` | 共有 domain validation |
| `when.on` の終端が required boolean でない | `WFT001` / `typecheck` | 共有 domain validation |
| `switch.on` の終端が required string enum でない | `WFT002` / `typecheck` | 共有 domain validation |

段が解決できない場合の code / stage は #1728 の対応と同じであり、統合 map を起点にしても変わらない。

#### 内部境界

- `reference::node_reference_schema(workflow, node) -> Result<SchemaDef, NodeReferenceSchemaError>` が、参照解決の起点 schema を返す唯一の入口になる。`NodeReferenceSchemaError` は「Contract が Object でない」「参照解決できる Artifact を持たない」の2値で、message は持たない。
- `reference::node_has_artifact` は Sequence を常に真にする。「供給元になれるか」の判定はこの関数、「field path を解決できるか」の判定は `node_reference_schema` が持つ。
- `SequenceSpecDto` と frontend の `SequenceSpec` から `output` を落とす。両者とも `entry` と `children` だけの形になる。

### Data Model

新規の永続 record は無い。統合 map は Sequence インスタンス確定時に scope 状態から導出する値であり、合成子の `ArtifactProduced` は fact log が既に「導出であり事実ではない」として記録から除いているため、event store に新しく書かれるものは無い。形が変わるのは `node_executions` に載る合成子インスタンスの `artifact` と `ArtifactProduced` event の payload だけである。

Sequence インスタンスの確定時に載せる他の値は、`contract` を `None`（R-004 により Contract 名が結び付かない）、`result_summary` を `None`、`token_usage` を `None` とする。いずれも `artifact` を宣言しない現行の Sequence と同じであり、children の計上をそのまま使う。

### Database

該当なし。

### UI/UX

`WorkflowDetail` の Sequence 表示から `Output` 行を除く。Artifact の表示は JSON をそのまま扱うため、他の変更は無い。

### Algorithm

#### 統合 map の走査順

`SequenceScopeRuntime.artifacts` は `HashMap` であり反復順が決まらない。map の組み立ては children の宣言順に走査し、`artifacts` の反復順や child の実行順に依存させない。同じ実行から同じ JSON が得られることを、直列化の実装に依らずに決めるためである。

#### 合成 schema の再帰

children を宣言順に辿り、各 child の参照解決用 schema を得て property にする。child が Sequence なら同じ手順を再帰し、訪問済み node 名の集合に入っている child は property にしないで打ち切る。深さは `MAX_NODES_PER_WORKFLOW` が縛るため、cycle 以外に非有界の芽は無い。

### Infra

該当なし。

### 必要な検証

- B-008 / B-009 / B-010 の Diagnostic は、`fixtures/invalid` の追加（`output` 宣言と Sequence の `artifact` 宣言）で code / stage / span と実 loader の拒否まで既存の fixture suite に載せ、Lua は同じ意味の定義を load して code と stage が YAML と一致することを確認する。
- B-005 / B-006 / B-007 / B-017 の受理は `fixtures/valid` に統合 map への多段参照（配線・`when.on`・`switch.on`・`fanout.items`）を持つ定義を追加して Diagnostic ゼロを固定し、実行時の値と遷移先は routing と実行木の単体テストで確認する。
- B-001 / B-002 / B-003 は、Sequence スコープを進めて終端へ到達させ、確定した Artifact の形を見る実行木の単体テストで確認する。通らない child、Artifact を産出しない child、後方辺で複数回通る child をそれぞれ含める。
- B-011 と B-013 / B-014 の load 側は、canonical example と builtin の Diagnostic ゼロを確認する既存経路がそのまま担う。B-012 の遷移は `review_scan` に相当する構造の routing テストで確認する。

## Alternatives Considered

- **Sequence の `artifact` も parse 段で拒否する**: YAML の Node 本体を kind ごとの struct に割れば `deny_unknown_fields` で拒否できる。しかし共通 field の重複キー検出と children のインライン正規化を kind ごとに持つことになり、得られるのは message の精度だけであるため採らない。
- **`output` も domain validation で拒否する（raw に残す）**: `artifact` と拒否の形が揃うが、二度と正当にならない field を model と DTO に残すことになるため採らない。
- **合成 schema の property を `required` にする**: 通らなかった children がキーにならない R-002 と矛盾する。`when.on` / `switch.on` の required 判定は終端段の直上 Object に対して行われるため、必要でもない。
- **合成 schema を `contract_schema` に置く**: schema 代数に `WorkflowDefinition` 依存が入る。合成は children を辿る操作であり、参照解決の知識を持つ `reference` の側に属する。
- **3箇所に Sequence 分岐を足して集約しない**: 変更は局所になるが、同じ分岐が3箇所で4通りに増える。#1730 で Fanout の Artifact も map になると同じ3箇所を再度触ることになるため採らない。
- **Fanout child を `SchemaDef::Array { items }` として property にする**: `items` は名前付き Contract 参照1つであり、children ごとに Contract が異なる Fanout を表現できない。

## Cross-cutting concerns

- 入れ子の Sequence では、統合 map が親の統合 map の値として入れ子に複製される。深さは合成子の包含 cycle 拒否と `MAX_NODES_PER_WORKFLOW` で有界であり、合成子の Artifact は fact log に記録されず replay で導出されるため、永続側の保持は増えない。増えるのは実行中の `node_executions` と `ArtifactProduced` event の payload である。
- 統合 map は children の Artifact をそのまま含む。secret の masking は leaf の Artifact 生成時に済んでいるため、写しによって新しい露出経路は生まれない。
- Diagnostic の入口は Tauri command / local API / CLI の3つあるが、いずれも同じ diagnose 経路を通るため、入口ごとの露出作業は発生しない。

## Risks

- 保存済みの実行が旧記法（`output` を含む）の定義を抱えている場合、field 削除後にその定義を読めなくなる。Non-goals が旧記法の互換維持と自動移行を対象外としているため移行手段は設けず、破壊を受け入れる。
- Fanout child の Sequence の結果が1段深い map になることは、下流が型なし input で受けるため load 時に検出されない。判断材料を読む facet 本文が旧い形を前提に書かれている場合、その齟齬は Diagnostic にもテストにも現れない。facet 本文は本 ISSUE の対象外であり、齟齬が見つかった時点で別途扱う。
