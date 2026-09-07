# Design

## The actual design

### Architecture

#### Fanout の Artifact は slot の展開座標をキーにした map にする

現行は `complete_scope` の Fanout 分岐（`entities/workflow_execution/mod.rs`）が `FanoutScopeRuntime.children` を走査して `serde_json::Value::Array` を組み立てている。R-001 から R-005 は、この同じ場所で `Value::Object` を組み立てる変更として満たす。新しい保持先も新しい状態も作らない。

キーは `children` の Vec 位置ではなく、slot の NodeExecution の親参照が持つ `FanoutSlot { item_index, child_index }` から導く。`children` の並びは live の初回展開では宣言順になるが、resume の replay では event 順に push されるため（`start_node_instance` の replay 分岐は、同じ座標の slot が見つからなければ末尾へ push する）、Vec 位置をキーの根拠にすると R-004 の「他の slot のキーは変わらない」が resume で崩れる。展開座標は retry の slot 差し替え後も新しい attempt の親参照が同じ値を持つため不変であり、`is_ignored_failed_fanout_slot` が既に同じ引き方をしている。

`on_failure: ignore` の失敗 slot の除外は `is_ignored_failed_fanout_slot` をそのまま使う（R-004）。除外しない slot は `artifact` が `None` でもキーを残し、値を `Null` にする（R-005）。slot が一つも展開されない場合は現行の `coordinates.is_empty()` から `complete_scope` へ入る経路をそのまま通り、空の `Value::Object` になる（B-004）。分岐を足さない。

#### キー規則の所有者は `FanoutSpec`

キーと展開座標の対応は、実行時の集約と load 時の参照解決の2箇所が使う。二重定義で規則が食い違うと、load を通った参照が実行時に当たらなくなる。対応は `FanoutSpec`（`value_objects/definition.rs`）が所有し、実行時は座標からキーを、load 時はキーから children エントリを引く。`items` の有無と `children` の両方を知っているのは `FanoutSpec` だけであり、`FanoutSlot` は座標しか知らない。

#### 参照解決は Node 境界を跨いで field path を歩く

#1729 が入れた `reference::node_reference_schema` は「その Node の Artifact を解決するための起点 schema」を `SchemaDef` で返し、呼び出し側が `contract_schema::resolve_field_path` を当てる。Sequence はこのとき children を辿って `SchemaDef::Object` を合成する。

`items` を宣言した Fanout のキーは展開順の添字であり、item 数は load 時に決まらない（`ItemsSource::ArtifactField` は実行時解決）。`SchemaDef::Object` は `properties` の列挙しか持たないため、添字キーを合成 schema として表現できない。`schemas` に書ける型を変えないことは Non-goal で決まっているので、宣言用の値オブジェクトである `SchemaDef` に添字 map の variant を足す選択も採らない。

したがって共有の入口を「起点 schema を返す」から「field path を解決する」へ変える。`reference` が Node 境界を跨いで段を1つずつ消費し、合成子の段は自分で解決して次の Node へ移る。Fanout の段は `FanoutSpec` のキー規則で children エントリへ落とし、その先を child Node に対して再帰する。leaf（Session / Command）へ着いた時点で残りの段を `contract_schema::resolve_field_path` に渡す。これにより合成 `SchemaDef::Object` の構築は不要になり、`node_reference_schema` は leaf の起点 schema を返す内部関数になる。

段の解決失敗が返す position は元の field path での絶対位置にし、message は現行どおり呼び出し側が組み立てる。呼び出し側は配線 `inputs`、辺の述語、`fanout.items`、および Lua 表面の未消費 source 検証の4箇所であり、段の解決失敗（position・segment）に関する message は変わらない。合成子の包含 cycle は訪問済み Node 名の集合で打ち切る（#1729 と同じ）。

合成子（Sequence / Fanout）の map で終端する `when.on` / `switch.on` は、段の解決失敗ではなく終端の型によって拒否する。例えば、Sequence `seq` の child `part`（Sequence）を `on: part` で指すと、変更前は合成 schema の `required` が空であるため `routing field 'part' must be required on its parent Object` となる。変更後は map 終端を表す `ResolvedNodeField` を返すため、`routing field 'part' must be boolean or string enum` へ拒否理由文が変わる。

#### 合成子の段は Sequence と Fanout で同じ規則にする

走査は合成子の段を「キーで children エントリへ落として次の Node へ移る」一つの規則で扱い、Sequence と Fanout で分岐しない。違うのはキーの引き方（Sequence はエントリ名、Fanout は上記のキー規則）だけである。この結果、Sequence の統合 map を経由した Fanout child（`<sequence名>.<fanout名>.<キー>.<field>...`）も、Fanout を直接起点にする参照と同じ経路で解決する（R-011）。合成子が何段入れ子になっても同じ規則で落ちる。

#1729 がこの経路を除外していた理由は「Fanout の Artifact は children の配列であり、`SchemaDef::Array { items }` が children ごとに異なる Contract を表現できないため」（`docs/specs/issues-1729/design.md`）であり、Fanout が map になると成立しない。`docs/glossary/WORKFLOW.md` の「Node の Interface と children の配線」節と「rules と辺」節にある制限記述は、理由ごと削除する（R-010）。

#### Fanout を自 Node とする辺の判別規則を解禁する

B-010 / B-011 は、Fanout を自 Node とする children エントリの辺に `when.on` / `switch.on` を置き、slot のキーを起点に分岐することを要求する。現行は `routing::RoutingValidationError::DiscriminatorOnFanout`（WFT006）が Fanout への判別規則を無条件に拒否する。根拠は `docs/specs/milestone-82/design.md` の「fanout node の Artifact は配列で field を持たない」であり、map になると成立しない。この規則を削除し、受理可否は既存の型検査（`validate_routing_field`）だけに決めさせる。Artifact を宣言しない Session child への判別規則を拒否する `DiscriminatorWithoutArtifact` は WFT006 のまま残る。

実行時の評価は変えない。辺の評価は親 Sequence スコープに記録された child の Artifact に `reference::resolve_value_at_path` を当てるため、Artifact が map になった時点で `a.passed` も `0.passed` も引ける。実行時に `when.on` の指す値が存在しない場合は、述語を false として同じ要素の sibling `next` へ進む。`switch.on` の指す値が存在しない場合は、どの case にも当たらず、非網羅 switch は sibling `next` へ進む。網羅 switch は `next` の併記が静的検査で拒否されるため、進行エラーになる。Artifact を持つ Command の独自 field で分岐する場合に限り、網羅 switch にも command failure の catch-all として `next` が必要になる既存の例外は変えない。

#### 主要な変更対象

| path | 担う変更 |
| --- | --- |
| `src-tauri/src/domain/workflow/value_objects/definition.rs` | `FanoutSpec` にキーと展開座標の対応を持たせる |
| `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` | Fanout インスタンスの成果を map にする |
| `src-tauri/src/domain/workflow/services/reference.rs` | Node 境界を跨ぐ field path 解決の新設、Fanout の段の解決、`node_reference_schema` の leaf 専用化 |
| `src-tauri/src/domain/workflow/services/routing.rs` | routing field の解決の差し替え、`DiscriminatorOnFanout` の削除 |
| `src-tauri/src/domain/workflow/services/validation.rs` | 配線 field path と `fanout.items` の解決の差し替え、削除した RoutingValidationError の写しの除去 |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` | `InvalidRuleKind::DiscriminatorOnFanout` の除去（code / stage / span / field の対応から） |
| `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs` | 未消費 source の field path 検証を新しい入口へ差し替える |
| `workflows/examples/full-cycle-development.yml` | Fanout の Artifact 構造を説明するコメント |
| `docs/glossary/WORKFLOW.md` | 「Fanout」「Contract / schemas」「Node の Interface と children の配線」「rules と辺」の各節 |

#### 既存定義の書き換え

`workflows/examples/full-cycle-development.yml` の3つの配線（`merge_implementations` と `check_integration` の `results: implement_all`、`merge_fixes` の `results: fix_all`）は、いずれも field path なしで Fanout 全体を型なし input へ渡している。供給元の書き方は変わらず、受け取る値だけが同じ slot 集合の map になるため、配線の書き換えは要らない（R-007 / R-009 / B-014 / B-016）。書き換えるのは、受け取る値を「子 Artifact 配列」と説明しているコメントだけである。

`workflows/*.yml`（builtin 8本）の3つの Fanout は、Artifact を受ける配線も Fanout を供給元にする参照も持たないため、変更しない。R-008 は現状のまま満たす。

### Interface

#### 定義表面

Fanout の宣言構文は変わらない。変わるのは engine が組み立てる Artifact の形と、Fanout を供給元にできる参照である。

| 供給元の形 | 意味 |
| --- | --- |
| `<fanout>` | map 全体（現行どおり field path なしで受けられる） |
| `<fanout>.<children エントリ名>.<field>...` | `items` を宣言しない Fanout の slot |
| `<fanout>.<添字>.<field>...` | `items` を宣言する Fanout の slot |
| `<合成子>.<fanout>.<キー>.<field>...` | 合成子の Artifact を経由した slot（Sequence の統合 map 経由を含む） |

辺の `when.on` / `switch.on` は自 Node の Artifact を起点にするため、Fanout を自 Node とするエントリでは `<キー>.<field>...` と書く。段の区切り記号、各段の文字種、段数の規則は #1728 のまま変えない。

添字キーは集約が作る正準な10進表記だけを解決する。`fan.007` のように同じ添字を指さない表記は解決せず、load を通らない。無名インラインエントリの合成内部名（`<fanout名>#<index>`）はキーになるが、`#` が field path の段に使えないため参照できない。Sequence の統合 map（#1729）と同じ扱いである。

Lua 表面も同じ domain の解決を通るため、YAML と同じ参照が同じ結果になる（B-013）。添字は Lua の識別子にできないので `fan["0"]` と書く。

#### 記述と Diagnostic の対応

| 記述 | code / stage | 拒否する場所 |
| --- | --- | --- |
| Fanout の `artifact` 宣言 | `WFT004` / `typecheck`（現行のまま） | 共有 domain validation |
| `<fanout>.<キー>.<field>` が配線で解決できない | `WFR007` / `resolve` | 共有 domain validation |
| Fanout を自 Node とする `when.on` の終端が required boolean でない | `WFT001` / `typecheck` | 共有 domain validation |
| Fanout を自 Node とする `switch.on` の終端が required string enum でない | `WFT002` / `typecheck` | 共有 domain validation |
| Fanout を自 Node とする判別規則そのもの | 受理する（現行は `WFT006`） | — |

段が解決できない場合の code / stage は #1728 の対応と同じであり、Fanout の map を起点にしても変わらない。

#### 内部境界

- `reference::resolve_node_field_path(workflow, node, field_path) -> Result<ResolvedNodeField, NodeFieldPathError>` が、Node の Artifact に対する field path 解決の唯一の入口になる。4つの呼び出し側はこれだけを呼ぶ。
- `ResolvedNodeField` は「leaf の Contract に着いた（schema と required を持つ）」か「合成子の map に着いた」かの2値。routing は後者を「boolean または string enum でない」（`routing field '<field>' must be boolean or string enum`）として拒否する。変更前に合成 schema の `required` 判定で拒否していた経路では、この message へ変わる。`fanout.items` は後者を「array でない」として、現行と同じ message で拒否する。
- `NodeFieldPathError` は「Artifact Contract が Object でない」「参照解決できる Artifact を持たない」「段の解決に失敗した（position・segment・NonObject / MissingProperty）」の3種別。前2つは現行の `NodeReferenceSchemaError` と同値で、呼び出し側の message は変わらない。
- `FanoutSpec` が、展開座標からキーへの写像と、キーから children エントリへの写像を持つ。
- `reference::node_has_artifact` は変えない。Fanout は引き続き Artifact を産出する Node として扱う。

### Data Model

新規の永続 record は無い。合成子の `ArtifactProduced` は fact log が導出として記録から外している（`adaptor/gateway/workflow/fact_log.rs` の composite 分岐）ため、event store に書かれる行の種類も形も変わらない。形が変わるのは、実行中の `node_executions` に載る Fanout インスタンスの `artifact` と、`ArtifactProduced` event の payload である（配列から object へ）。

保存済みの実行は replay で `complete_scope` を通り直すため、再構成される Fanout の Artifact は新しい形になる。配列のまま残る保存済みの値は無い。

Fanout インスタンスの確定時に載せる他の値は現行のままとする（`contract` は `None`、`result_summary` は `complete`、`token_usage` は children の合算）。

### Database

該当なし。

### UI/UX

該当なし。frontend は Fanout の定義構造を表示するだけで、Artifact は JSON をそのまま扱う。

### Algorithm

#### 添字キーと展開座標の対応

`items` を宣言する Fanout の添字は `item_index * children.len() + child_index` である。これは `expand_fanout_scope` が items を外側・children を内側にして展開する順序そのものであり、B-003 の並びと一致する。逆向きの解決は `child_index = 添字 % children.len()` になる。`on_failure: ignore` の欠番で再採番しない（R-004）ため、この対応は失敗の有無に依らず成立する。

`items` を宣言しない Fanout のキーは `children[child_index]` のエントリ名である。同一 children 内の重複エントリ名は `DuplicateChildReference` が拒否するため、キーは一意になる。

添字キーの静的解決が決めるのは「どの children エントリの Contract か」だけである。item 数は load 時に決まらないため、添字の上限は検査しない。

#### field path の走査

起点 Node から段を1つずつ消費する。

- 現在の Node が合成子なら、先頭の段をキーとして children エントリへ落とし、対応する Node を次の現在 Node にする。Sequence はエントリ名、Fanout は上記のキー規則で引く。落とせない段（未知のキー、Fanout child を指す Sequence の段、Artifact を産出しない child）は、その位置の `MissingProperty` として失敗する。
- 現在の Node が leaf なら、残りの段を `contract_schema::resolve_field_path` に渡す。position は元の field path での絶対位置へ戻す。
- 段が尽きた位置が合成子なら、map 終端として返す。
- 訪問済み Node 名の集合で再訪を打ち切る。深さは `MAX_NODES_PER_WORKFLOW` と包含 cycle の拒否が縛る。

required を与えるのは leaf の Contract の `required` だけであり、合成子の段は required にしない。通らなかった child と `ignore` の欠番があるため、どのキーも必ず立つとは言えないからである。`when.on` / `switch.on` の required 判定は終端段の直上 Object に対して行われるので、`a.passed` は `a` の Contract の `required` で判定され、B-010 / B-011 は成立する。

### Infra

該当なし。

### 必要な検証

- B-001 から B-006 は、Fanout スコープを全 slot 決着まで進めて確定した Artifact の形を見る実行木の単体テストで確認する。`items` なし、`items` あり、`items` と複数 children、`on_failure: ignore` の失敗、`artifact` を宣言しない child、宣言なしの失敗、slot 0個をそれぞれ含める。キーが Vec 位置に依らないことは、slot の push 順が展開順と異なる replay を作って確認する。
- B-007 は現行の `WFT004` 経路が変わらないため、既存の検証で足りる。
- B-008 / B-009 / B-012 / B-014 の受理は、`fixtures/valid` に Fanout を起点とする多段参照（配線 `inputs`・`fanout.items`・field path なし）を持つ定義を足して Diagnostic ゼロを固定する。実行時に渡る値は束縛解決の単体テストで確認する。
- B-010 / B-011 は、受理を `fixtures/valid` の定義で、遷移先を routing の単体テストで確認する。既存の `fixtures/invalid/WFT006_fanout-discriminator.yml` は解禁により invalid でなくなるため取り除き、WFT006 は Artifact を宣言しない Session child の fixture で覆う。
- B-018 / B-019 は、Sequence の children に Fanout を置き、その Sequence の外から `<sequence>.<fanout>.<キー>.<field>` を配線と `when.on` に書いた定義を `fixtures/valid` へ足して受理を固定し、遷移先を routing の単体テストで確認する。
- B-013 は、YAML と同じ参照を Lua で書いた定義を load し、Diagnostic ゼロが一致することを確認する。
- B-015 / B-016 の load 側は、builtin と正本サンプルの Diagnostic ゼロを確認する既存経路がそのまま担う。
- B-017 は文書の差分で確認する。

## Alternatives Considered

- **`SchemaDef` に添字 map の variant を足す**: 合成 `SchemaDef::Object` と `resolve_field_path` の再利用をそのまま保てる。しかし `schemas` に書けない variant を宣言用の値オブジェクトへ入れることになり、直列化にも意味のない形が増える。宣言規則を変えない Non-goal とも近接するため採らない。
- **走査を `items` あり Fanout の起点だけに限り、他は合成 `SchemaDef::Object` を維持する**: 差分は小さいが、`items` あり Fanout が別の合成子の children に現れると合成側で表現できず、黙って参照不能になる。同じ目的の機構が2つ残る。
- **添字キーの解決を children エントリが1つの Fanout に限る**: B-009 は満たすが、`items` と複数 children を同時に宣言した Fanout（R-003 が展開順を定めている）だけ参照できなくなる。剰余で一意に決まるため、制限を足す根拠がない。
- **集約のキーを `children` の Vec 位置から作る**: 座標を引く手間は減るが、resume の replay では push 順が展開順と一致しないため、R-004 のキー安定が保てない。

## Cross-cutting concerns

- 参照解決は load 時に Node 境界を跨いで再帰する。深さは包含 cycle の拒否（`WFC008`）、訪問済み集合、`MAX_NODES_PER_WORKFLOW` で有界である。合成 schema を組み立てなくなるため、1段だけの参照のために合成子の部分木全体を clone することも無くなる。
- Fanout の Artifact は children の Artifact をそのまま含む。secret の masking は leaf の Artifact 生成時に済んでいるため、map 化で新しい露出経路は生まれない。
- Diagnostic の入口は Tauri command / local API / CLI の3つとも同じ diagnose 経路を通る。Lua 表面の未消費 source 検証だけが別の関数から解決しているため、そこも新しい入口へ差し替える（B-013）。

## Risks

- 添字キーの参照は、load 時には children エントリの Contract までしか検査できない。`items` が実行時に空、または slot が存在しない添字を書いた参照は、load を通ったうえで実行時に未解決になる。配線は束縛から除かれる。辺では `when` の述語は false になり、同じ要素の sibling `next` へ進む。`switch` はどの case にも当たらず、非網羅なら sibling `next` へ進み、網羅なら `next` を書けないため進行エラーになる。欠番を判断材料にする定義は、Fanout 全体を field path なしで受ける Command / Session が routing 用の値を Artifact にする。
- Fanout の結果を判断材料として読む facet 本文が配列を前提に書かれている場合、その齟齬は Diagnostic にもテストにも現れない。facet 本文は本 ISSUE の対象外であり、見つかった時点で別途扱う。
