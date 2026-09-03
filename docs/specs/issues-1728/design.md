# Design

## The actual design

### Architecture

#### 段の並びと解決規則は一つずつしか持たない

現行は5箇所がそれぞれ段数の規則を持っている。`reference::split_reference`（2要素まで）、`reference::parse_reference`（field 1つまで）、`InputParameterRef::new`、`ItemsSource` の deserialize、Lua host の `index` の3つの拒否、`contract_schema::routing_field_kind`（field 名1つ）。これらを次の2つへ集約し、5箇所は段数を判定しない。

- **`FieldPath`**（値オブジェクト、`src-tauri/src/domain/workflow/value_objects/`）: 段の並びと `.` 連結表記を所有する。段そのものの文字種は検査せず、段0個（field を持たない参照）も表現する。`<供給元>.<段>` 表面の構築・直列化でだけ、現行 `reference::is_reference_segment` 相当の文字種規則を検査する。routing の `when.on` / `switch.on` は Contract property 名をそのまま段として扱うため、この文字種規則を課さない。
- **静的解決**（`services/contract_schema.rs`）: 供給元の `SchemaDef` を入力に `FieldPath` を辿り、終端の `SchemaDef` と「終端が親 Object の `required` にあるか」を返す。失敗は「どの段で」「存在しない field を引いたか / Object でない値から引いたか」を返す。
- **実行時解決**（`services/reference.rs`）: 束縛済み値・Artifact の JSON を `FieldPath` で辿る。現行の1段走査（`field_value`）を置き換える。

5箇所が持つのは「終端に何を要求するか」だけになる（→ Interface）。

#### 中間段は Object でなければならない

R-007 は解決できない段として「Object でない値から field を引く段」を挙げ、array を Object でない値に含めている。したがって `SchemaDef::Array` から field を引く段は Error Diagnostic とし、array の要素 Contract（`SchemaDef::Array { items }` が指す名前付き Contract）を辿って次の段を解決することはしない。

#### `required` は終端の段にだけ要求する

R-002 / R-003 は、required を課す対象を末端 field と定め、経路上の中間段には要求しないと定めている。`when.on` / `switch.on` は、終端 field がその直上の Object の `required` に含まれることだけを要求する。

#### Contract を持たない供給元は段数によらず検査しない

R-009 は、Contract を持たない供給元（型なし input パラメータ）を起点とする参照を、段数によらず load 時の静的検査の対象外と定めている。供給元に Contract が無いと R-007 の「存在しない field を引く段」を判定できないため、静的解決は段を辿らない。

#### Command の Artifact は予約 field を合成した参照解決用 schema で解く

現行は `reference::artifact_field_schema` と `reference::node_field_available` が `COMMAND_RESERVED_FIELDS` を参照し、`routing::validate_routing_field` が `ok` だけを直接分岐している。いずれも「1段目が予約 field か」という形をしていて多段に載らない。これを、宣言された Artifact Contract の Object に `ok` / `exit_code` / `stdout` / `stderr` / `duration` を required property として合成した**参照解決用 schema** 1つへ集約し、全ての参照解決の起点にする。

- `artifact` 宣言の無い Command は予約 field だけの Object になる（B-017）。
- 予約 field の再宣言は現行どおり `WFT005` が拒否するため、合成時の衝突は起きない。
- この合成 schema は参照解決にだけ使い、実行時の stdout Contract 検証には使わない（実行時に engine が合成する Artifact 値の側は現行のまま）。

帰結として `switch.on: ok` は、専用 message（`switch.on cannot reference command reserved boolean field 'ok'`）ではなく終端の kind 不一致として拒否される。R-009 が維持対象を受理・拒否・値解決と定め message 文言を含めないため、この message の変化は許容される。

#### Lua 表面は参照解決の規則を持たない

Lua host から深さ制限（`nested artifact field references are not supported` / `nested input field references are not supported`）と、**定義に消費される参照**の schema 走査を外す。host は index 時に段を記録するだけにする。消費された参照は、build 後に走る共有の domain validation（`diagnose_lua_workflow_source` は `load_lua_workflow` 成功後に `diagnose_workflow_definition` を呼ぶ）が use site に応じた Diagnostic を返し、YAML と一致する（R-008 / B-014）。

どの配線・辺・`env`・`items` にも消費されない参照（例: `local invalid = child.missing`）は `WorkflowDefinition` に現れず共有 validation に届かない。現行はこれを index 行の `WFR003` で拒否しており、R-009 によりこの拒否は維持する必要があるため、host が build 完了時に未消費の source draft だけを自分の schema draft 上で解決し、同じ `WFR003` を返す。

帰結として、消費される参照の解決 Diagnostic は Lua source の行ではなく node 単位の span になる。Lua で共有 validation を通る Diagnostic は現行から node 単位であり、R-008 は span を各表面の位置付けに委ねている。

#### 主要な変更対象

| path | 担う変更 |
| --- | --- |
| `src-tauri/src/domain/workflow/value_objects/` | 段の並びを表す `FieldPath` の新設 |
| `src-tauri/src/domain/workflow/services/contract_schema.rs` | schema 上の path 解決と、Command の参照解決用 schema の合成 |
| `src-tauri/src/domain/workflow/services/reference.rs` | 参照の静的検査と実行時の値解決を段の並びで行う |
| `src-tauri/src/domain/workflow/services/validation.rs` | 配線 `inputs` と `fanout.items` の段検査 |
| `src-tauri/src/domain/workflow/services/routing.rs` | `when.on` / `switch.on` の型検査・`cases` 網羅検査・実行時評価 |
| `src-tauri/src/domain/workflow/value_objects/definition.rs` | `InputParameterRef` と `ItemsSource::ArtifactField` の field 保持 |
| `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` | `fanout.items` の実行時解決 |
| `src-tauri/src/adaptor/gateway/workflow/workflow_host/prompt_rendering.rs` | テンプレート描画と、保存時のテンプレート検証（B-009） |
| `src-tauri/src/adaptor/gateway/workflow/lua/mod.rs` | 深さ制限の撤去と、未消費参照だけを対象にする build 完了時の解決 |
| `docs/glossary/WORKFLOW.md` | 配線 / rules と辺 / Command / Fanout / Lua の各節 |

`workflows/*.yml`（builtin 8本）と `workflows/examples/full-cycle-development.yml` は多段参照を必要としないため定義の変更は無く、R-010 / B-018 は回帰確認である。

### Interface

#### 定義表面の受理形

配線 `inputs`、Command の `env`、テンプレート `{{ }}`、`fanout.items` の受理形は `<供給元>` と `<供給元>.<段>...`（段は1つ以上）になり、区切り記号と各段の文字種は変えない。`request` / `items` は引き続き段を持たず、段を書けば Error Diagnostic になる（B-016）。

`when.on` / `switch.on` は `<段>...` を受理し、`.` を区切り記号とする一方、各段の文字種は制限しない。このため `legacy flag` のように空白を含む property 名は、引き続き1段参照として引ける。`.` を区切り記号とする帰結として、`.` を含む property 名は `when.on` / `switch.on` の分岐条件から引けない。参照文字列全体の前後空白と空の段は、段数によらず Error Diagnostic にする。

#### 各箇所が終端に要求するものと Diagnostic

| 箇所 | 終端に要求するもの | 解決できない段の Diagnostic |
| --- | --- | --- |
| 配線 `inputs` | 制約なし | `WFR007` / resolve（`InputWiringKind::UnknownSourceField`。記述そのものが不正な場合は同 code の `InvalidSourceFormat`） |
| `when.on` | 親 Object の `required` にある boolean | `WFT001` / typecheck |
| `switch.on` | 親 Object の `required` にある string enum | `WFT002` / typecheck。`cases` 網羅は終端 field の enum に対して行い、非網羅かつ catch-all 無しは現行どおり `WFC004` |
| `env` | 制約なし | `WFR003` / resolve |
| `{{ }}` | 制約なし | `WFR003` / resolve |
| `fanout.items` | `array` | `WFR003` / resolve |

1段参照でのこの対応は現行と同一である（R-009）。

#### 内部境界

- `Rule::When` / `Rule::Switch` の `on` と `ChildEntry.inputs` の `InputSourceRef` は**表記文字列のまま保持**し、`FieldPath` への解釈は validation と評価で行う。構築時に解釈すると、記述が不正な場合の Diagnostic が現行の resolve / typecheck 段から parse/shape 段へ移り、さらに YAML（serde 経由）と Lua（host 経由）で別の code に分かれる。R-009 が求める「1段参照の受理・拒否が従来どおり」と R-008 の表面一致を両立させるため、現行の保持形を変えない。
- `InputParameterRef`（`env`）と `ItemsSource::ArtifactField` は現行どおり構築時に解釈し、field 部分を `FieldPath` として持つ。これらは現行も構築時に解釈しており、Diagnostic の段は変わらない。
- 直列化表記は不変（`<root>.<段>...` の1本の文字列）。`RuleDto` / `ChildInputDto` / `ItemsSourceDto` および frontend の `Rule.on` / `ChildInput.source` / `FanoutItemsSource` は `string` のままで、frontend の変更は無い。

### Data Model

新規の永続 record は無い。`FieldPath` は定義の値オブジェクトであり、識別子は表記文字列そのものである。直列化表記が現行と同一のため、event store / fact log に既に書かれた定義との互換は保たれ、versioning は不要である。

### Database

該当なし。

### UI/UX

該当なし。frontend は参照を不透明な文字列として表示するだけで、段数を解釈しない。

### Algorithm

#### 静的解決の方式

供給元の schema を起点に段を順に辿り、各段で「Object か」「`properties` にその field があるか」を見る。Object でなければ「Object でない値から field を引く段」、`properties` に無ければ「存在しない field を引く段」として、失敗した段を伴う Error にする。`SchemaDef::Array` は Object ではないため中間段になれない。終端に到達したら、その `SchemaDef` と「直上の Object の `required` に含まれるか」を返す。

この方式を採る理由は、`SchemaDef` が Object の `properties` を保持しており各段を load 時に一意に決められるためである。実行時の値から段を推定する方式では、`when.on` の boolean 要求や `fanout.items` の array 要求を load 時に判定できず、R-007 の「解決できない段は load 時に Error Diagnostic」を満たせない。

供給元が Contract を持たない場合（型なし input パラメータ）は段を辿らず、静的検査の対象にしない。

#### 実行時解決の方式

束縛済みパラメータ値または Artifact の JSON を段の順に辿る。途中の値が Object でない、または key が無ければ未解決とする。未解決時の扱いは箇所ごとに現行と同じで、配線は束縛から除き、テンプレートは `{{ }}` を残し、`when` は false、`switch` は `next` へ落ち、`fanout.items` は実行時 error になる。

### Infra

該当なし。

### 必要な検証

- B-014 の YAML と Lua の一致は、消費される参照（配線 / 辺 / `env` / `items` / `{{ }}`）について同じ意味の定義を両表面で load し、code / stage / message を突き合わせて確認する。span は R-008 により一致対象ではない。未消費参照の `WFR003` は Lua 固有の挙動として別に固定する。
- 多段参照の受理と、解決できない段の拒否は、`workflows/` と定義 fixture を一括 load する既存の Diagnostic 検証経路に載せる。

## Alternatives Considered

- **Lua host が消費される参照も index 時に解決し続ける（現行構造の維持）**: 変更は最小になるが、解決できない段に対して Lua は `WFR003` を返し、YAML は配線で `WFR007`、辺で `WFT001` / `WFT002` を返すため、B-014 の「YAML と同じ Diagnostic」が成立しない。
- **Lua host の参照解決を完全に撤去し共有 validation へ一本化する**: 規則の所在は最も明快になるが、どこにも消費されない参照（`local invalid = child.missing`）が `WorkflowDefinition` に現れないため受理へ変わり、現行の拒否を覆して R-009 に反する。
- **消費される参照の Lua source 行を共有 Diagnostic へ引き継ぐ**: `LuaWorkflowDefinition` に参照単位の位置を持たせ、`DiagnosticItem` が参照を識別できるようにすれば span 精度を保てる。しかし R-008 は表面間の一致対象を code / stage / message と定め、span を各表面の位置付けに委ねている。Lua で共有 validation を通る Diagnostic は現行から node 単位であり、参照解決だけがその例外だったため採らない。
- **中間段が array のとき要素 Contract を辿る**: 入れ子配列から値を引けるようになるが、R-007 が解決できない段として array から field を引く段を挙げているため採らない。

## Cross-cutting concerns

- Diagnostic の入口は Tauri command / local API / CLI の3つあるが、いずれも同じ `diagnose_workflow_source` / `diagnose_lua_workflow_source` を通るため、多段対応のために入口ごとの露出作業は発生しない。
- Lua の評価環境は arena budget を持ち、`index` のたびに消費する。多段参照は段の数だけ source draft を作るため budget の消費が早くなる。上限は緩めない。

## Risks

- Lua は配線での型なし input パラメータの field 参照を `WFR003`（`input does not declare a contract`）で拒否するが、YAML は受理する。この1段の挙動は R-009 により維持するため、B-014 の検証は型ありの参照で行う必要がある。
