# Milestone 82 詳細設計（実装の正本）

本書は milestone 82 の実装詳細設計であり、**全 goal はこの設計に従って実装する**。Codex は本書と矛盾する設計をやり直さない（矛盾・実装不能を発見した場合は理由付きで報告し、勝手に読み替えない）。仕様の由来は `plan.md`（設計判断 D1〜D7 / P1〜P14）、`docs/workflow-yaml-syntax.md`、`docs/workflow-engine-evolution-plan.md`、`docs/architecture/GLOSSARY.md`。

**Diagnostic code の割当は §7 の表が唯一の正**（本文中の code 参照は表に従属する）。

---

## 1. モジュール配置マップ

層規約（`docs/architecture/`）に従う。**置換** = 同一責務を新実装で置き換え旧コード削除、**新規** = 新設、**削除** = goal 完了時に存在しない。

| 場所 | 処置 | 内容（実装 goal） |
|---|---|---|
| `adaptor/gateway/workflow/schema.rs` | 置換 | §2 の新 YAML schema 型（#1322, #1325, #1326, #1327, #1329） |
| `adaptor/gateway/workflow/span_map.rs` | 新規 | saphyr AST → YAML path→span map（#1323） |
| `adaptor/gateway/workflow/diagnostics.rs` | 置換 | §7 Diagnostic 型と parse/shape 段（#1323） |
| `domain/workflow/value_objects/definition.rs` | 置換 | schema の domain 鏡像（最小限）。`domain_mapping.rs` が変換（#1322〜） |
| `domain/workflow/services/contract_schema.rs` | 新規 | §4 Contract subset 検証エンジン（#1325） |
| `domain/workflow/services/routing.rs` | 新規 | §6 rules 静的検証 + 実行時評価。旧 `transition.rs` の regex 評価を置換（#1327） |
| `domain/workflow/services/reference.rs` | 新規 | §5 参照 parser（旧 variable_renderer 置換、#1326） |
| `domain/workflow/services/validation.rs` | 置換 | §7 の resolve / typecheck / control-flow 段に再編（#1323） |
| `domain/workflow/services/{parallel,approval_rules,contract}.rs` | 置換/削除 | fanout 展開規則 / approve 検証（reject 削除）/ 削除（#1329, #1324, #1325） |
| `infrastructure/process/command_runner.rs` | 新規 | §8.1 shell 実行 + cancellation（#1328） |
| `adaptor/gateway/workflow/event.rs`, `log.rs`, `event_projection.rs` | 置換 | §9 event 語彙・§10 projection（各 goal + #1331 で最終化） |
| `adaptor/gateway/workflow/{run,state}.rs` ほか step_* 系 | 置換 | §10 read model（#1331） |
| `adaptor/gateway/workflow/pending_command.rs`, `cli/workflow_io.rs` | 削除 | pending file 機構（#1332） |
| `adaptor/controller/api/` | 新規 | §11 local API（axum、#1332） |
| `cli/api_client.rs` | 新規 | discovery + HTTP client（#1332） |
| `adaptor/gateway/workflow/orphan_recovery.rs` | 置換 | §8.5 interrupted 化（#1335） |
| `src/components/panels/automation/` の StepEditor / WorkflowEditor フォーム編集 | 削除 | D7（#1322。削除対象はこの 2 系統のフィールド編集 UI とそのテスト） |
| `src/types/workflow.ts`, `workspace-tree.ts` | 置換 | §13 frontend 型（#1322〜#1331） |

## 2. YAML schema 型設計（Rust）

`adaptor/gateway/workflow/schema.rs` の完成形。**不正状態を型で表現不能にする**。GLOSSARY 禁止語を避けるため、YAML deserialize 先の root 型は `WorkflowDefinitionYaml`（adaptor DTO）、domain 鏡像は `WorkflowDefinition` とする。

```rust
pub struct WorkflowDefinitionYaml {
    pub name: String,
    pub description: String,
    #[serde(default)] pub builtin: bool,
    #[serde(default)] pub schemas: BTreeMap<String, SchemaDef>,   // §4
    pub nodes: Vec<NodeDefinition>,
}

pub struct NodeDefinition {
    pub name: String,
    pub kind: NodeKind,                 // ちょうど1つ（型で保証）
    pub artifact: Option<String>,       // Contract 名
    pub input: Option<String>,          // Contract 名（fanout child のパラメータ型）
    pub inputs: Vec<InputRef>,          // §5。session/command のみ（fanout は不許可 = WFS004）
    pub rules: Vec<Rule>,               // 空 = 終端 node
}

pub enum NodeKind {
    Command(CommandSpec),
    Session(SessionSpec),
    Fanout(FanoutSpec),
}

pub struct CommandSpec { pub command: String }        // shell command scalar

pub struct SessionSpec {
    pub model: Option<String>,
    pub permission: PermissionMode,     // 既存 ask | edit | full を再利用（D5）
    pub gate: Gate,                     // #1322 では Option<Gate>（省略=Auto）、#1324 で必須化
    pub facets: FacetRefs,              // { policy?: String, knowledge?: String, instruction?: String }
}
pub enum Gate { Auto, Approval }

pub struct FanoutSpec {
    pub child: Vec<String>,             // YAML はスカラ/配列両受理（one-or-many）。node 名参照
    pub items: Option<ItemsSource>,
}
pub enum ItemsSource {
    Literal(Vec<serde_json::Value>),                  // リテラル配列
    ArtifactField { node: String, field: String },    // "list_threads.threads" 形式（§5 の subset）
}

pub enum InputRef { Request, Node(String) }           // "request" | "<node名>"

pub enum Rule {                                       // §6
    When { on: String, then: String, next: String },  // next 必須（bool の網羅）
    Switch { on: String, cases: BTreeMap<String, String>, next: Option<String> },
    LoopGuard { max_iterations: u32, on_exhausted: String },
    Next(String),
}
```

**Deserialize 方針**: serde derive では「kind block ちょうど1つ」を判別できないため、schema.rs 内部に private な `RawNode`（`command` / `session` / `fanout` を全て `Option` で持つ、`deny_unknown_fields`）を置き、`TryFrom<RawNode> for NodeDefinition` で kind 0個/2個以上を即エラー化する。`Rule` も要素 map のキー集合 **`{when,next}` / `{switch}` / `{switch,next}` / `{loop_guard}` / `{next}`** のいずれかで判別し（switch の `next` は Option。§6 R3）、それ以外のキー組合せは parse/shape Diagnostic。**旧 field（`type:` / `output_contract` / `input_contracts` / `pass_output_from` / `pass_previous_response` / `parallel_children` / `aggregate` / `collect` / `cycle_guard` / `resets_cycle_for` / `inline_prompt` / `variables:` / `match:`）は unknown field として拒否**され、WFS005（§7）で「旧構文」と明示する。

**移行期の暫定形**（該当 goal 完了で消える）:
- #1322 時点の `FanoutSpec` は `{ parallel_children: Vec<InterimChild>, aggregate: Option<ParallelAggregate> }` を内包する（**field 名のみ維持**）。`InterimChild` は旧 `ChildNodeDefinition` から **`type:` と flat facet を除去した** `{ name, model?, permission?, facets{...}, output_contract?, input_contracts?, pass_previous_response?, pass_output_from? }`（子は暗黙に session 扱い）。したがって #1322 の受け入れ基準「旧 `type:` が schema / built-in に残らない」は child 要素にも適用される。`InterimChild` と `aggregate` は #1329/#1330 で完成形に置換される。
- `rules` は #1327 まで旧 `Vec<TransitionRule>` のまま共通 field に残す。
- `output_contract` / `input_contracts` / `pass_*` は #1325/#1326 まで共通 field 位置に残す。

## 3. domain 鏡像

`domain/workflow/value_objects/definition.rs` は §2 と同形の型（serde 非依存）を持ち、`domain_mapping.rs` が 1:1 変換する。鏡像は**フィールドの増減・意味の差を持たない**（層規約上の分離のみが目的）。validation / routing / contract 検証は全て domain 型に対して行う。

## 4. Contract（`schemas:`）詳細設計（D2）

```rust
pub enum SchemaDef {
    Object {
        properties: BTreeMap<String, SchemaDef>,
        required: BTreeSet<String>,          // default 空
        additional_properties: bool,          // default true（JSON Schema と同じ。未宣言 field は無視して通す）
    },
    Array { items: String },                 // 要素型は名前付き Contract 参照のみ。inline 不可
    String { r#enum: Option<Vec<String>> },
    Boolean,
    Integer,
    Number,
}
```

- YAML 表記は JSON Schema 風（`type: object` / `properties:` / `required:` / `items: <Contract名>` / `enum:` / `additionalProperties:`）。この subset 外のキーワード（`oneOf` / `format` / `pattern` / `default` 等）は WFS002。
- `additionalProperties` の既定は **true**（JSON Schema の既定に合わせる）。閉じた検証が必要な Contract だけ `additionalProperties: false` を明示宣言する。
- **検証エンジン** `contract_schema::validate(value: &serde_json::Value, schema: &SchemaDef, schemas: &Map) -> Result<(), Vec<Violation>>`。Violation は JSON path + 理由。repair prompt はこの Violation 一覧から生成する。
- **routing 可能 field** の判定:
  - Object Contract の property `p` が routing 可能 ⇔ `p ∈ required` かつ型が `Boolean` または `String{enum: Some(_)}`。
  - **command node では予約 field `ok` が常に routing 可能な Boolean**（artifact 有無を問わない。保存時合成で常に存在するため、§6 R4 の catch-all 必須の根拠にはならない＝`ok` だけを参照する rules に R4 は適用しない）。
- **`artifact:` に指定できる Contract は Object のみ**（scalar/array を artifact にするのは WFT004。fanout の artifact は暗黙に「子 artifact の配列」であり宣言不要・宣言不可）。`input:` と `items:` の Contract は任意の SchemaDef。
- **`request` は暗黙の String scalar Contract**。schemas に `request` という名前を宣言するのは WFR004（予約名衝突）。
- **command の予約 field**（D3）: `ok: Boolean` / `exit_code: Integer` / `stdout: String` / `stderr: String` / `duration: Integer(ms)`。command node の `artifact:` Contract がこれらの property 名を宣言したら WFT005。検証は stdout JSON に対して Contract のみで行い、保存時に予約 field を合成する（§8.1）。

## 5. Artifact と参照規約

- **NodeExecution の識別**（§10 と共通）: engine は NodeExecution 開始時に **`node_execution_id`**（WorkflowExecution 内で一意な採番。例 `ne-000042`）を発行する。approve / Artifact 提出などの typed command は原則 node 名でアドレスし、**同名 NodeExecution が複数 active な場合（fanout child 並走）は `node_execution_id` の指定を必須**とする（§11/§12）。engine は session / command の実行環境に `RELEASH_WORKFLOW_EXECUTION_ID` / `RELEASH_NODE_EXECUTION_ID` を環境変数として注入し、session 内の agent が実行する CLI はこれを既定値に使う（fanout child 内からの `output submit` が自動的に正しい NodeExecution へ紐づく）。
- **保存**: WorkflowExecution ごとに `node名 → 検証済み Artifact（serde_json::Value）` の map を projection が保持（§10）。各 node の Artifact は最新の成功 attempt のもの。`request` は起動時に確定する読み取り専用エントリ。**fanout child の Artifact は親 fanout の配列にのみ格納され、node 名 map には載らない**（child 名の `inputs:` / `{{ }}` 参照は WFR003）。
- **参照 path**: `request` | `<node>` | `<node>.<field>` | `item` | `item.<field>` の 5 形のみ。parser は `reference.rs`（domain）に一本化し、`inputs:` / `{{ }}` / rules の `on:`（bare 名 = 自 node への省略形）が同じ parser を使う。**`items:` はこの文法のうち `<node>.<field>` 形とリテラル配列のみを受理する subset**（`<node>` 全体・`item` は items に書けない。fanout の配列 Artifact を別 fanout の items に直接渡す連鎖は不可で、間に reducer 等の通常 node を挟む）。
- **`inputs:` の意味**: kind ごとに次のとおり。
  - session: prompt 組み立て時に各 InputRef を次の形で追記する:
    ```
    ## input: <参照名>
    ```json
    <Artifact の JSON>
    ```
    ```
  - command: **依存の宣言のみ**（注入は行わない。データは `{{ }}` 補間で参照する）。未定義参照は通常どおり Diagnostic。
  - fanout: `inputs:` は不許可（WFS004）。
- **参照可能性**: command node は常に標準結果 Artifact を持つため常に参照可。**`artifact:` 無しの session node への `inputs:` / `{{ <node> }}` 参照は WFR003**（産出物が無い）。fanout node の参照は配列 JSON。
- **`{{ ... }}` template 補間**: command 文字列と facet 本文で使用可。String scalar はそのまま、それ以外は JSON serialize して埋め込む。shell quoting の危険は既知の制約とし（syntax doc の懸念）、ABI 改善はスコープ外・#1337 で文書に明記する。
- **scope 規則**: `item` は fanout child として実行される node 内でのみ有効（resolve 段で検査）。`request` / `item` を node 名にしたら WFR004。
- **旧参照の全廃**（D6）: `{{task}}` / `{{project_name}}` / `{{path_alias.*}}` / `{{vars.*}}` は renderer から存在ごと消す（未知の template 変数は resolve Diagnostic）。

## 6. rules 詳細設計

### 6.1 正規形（load 時に強制。順序非依存の根拠）

node の `rules` は次の組合せのみ valid（R 規則。code は §7 表）:

- **R1** 空 → 終端 node（WorkflowExecution 完了）。
- **R2** 判別 rule（When または Switch）は **最大 1 個**。When が 2 個以上・When+Switch 併用は WFC002（排他を機械証明できないため。syntax doc の「懸念」はこの規則で解消する）。
- **R3** catch-all: When は構文上 `next` 必須（bool の網羅）。Switch は (a) cases が enum 全値を被覆するなら `next` 禁止、(b) 非被覆なら `next` 必須。単独 `Next` 要素は判別 rule が無い場合のみ 1 個。
- **R4** P11 例外: **command + `artifact:` の node**は artifact validation が失敗しうるため、**Contract field**（予約 field `ok` 以外）を参照する場合は Switch が全値被覆でも catch-all（`next`）必須（WFC003）。`ok` のみ参照する場合は対象外（`ok` は常に存在）。session + `artifact:` は P13 により提出保証があるので R3 のまま。
- **R5** LoopGuard は最大 1 個。cycle を作る遷移を持つ node 群のうち、その cycle 上の少なくとも 1 node に到達可能な LoopGuard が無ければ WFC005。
- **R6** `when.on` は自 node Artifact の routing 可能 Boolean field、`switch.on` は routing 可能 enum field（§4。command の `ok` は常に可）。**fanout node の Artifact は配列で field を持たないため、fanout に When/Switch は WFT006（Next / LoopGuard のみ可）**。artifact 無し session も判別 rule 不可（WFT006）。
- **R7** **fanout child は leaf 専用**: fanout の `child` に参照される node は、entry（先頭 node）や他 node の rules の遷移先（then / cases / next / on_exhausted）になれない（WFC006）。child の `rules` は fanout 実行中無視される（P3。宣言自体は Diagnostic にしない）。child が fanout kind の node を参照する（fanout の入れ子）のも WFC006。

到達不能 node（entry = `nodes` 先頭から遷移で到達できない node。fanout child は child 参照で到達扱い）は WFC001。switch の cases に enum 外の値があれば WFT002。

### 6.2 実行時評価（`routing.rs`）

node 完了時、`route(node, artifact) -> Target`:

1. 判別 rule を評価。When: field が true → `then`、false → `next`。Switch: field 値の case → その target、case 外/field 不在 → `next`（P11: field 不在は no-match）。
2. 判別 rule 無し → `Next`。
3. 決定した target T について、**T の LoopGuard を検査**: T の完了済み実行回数 ≥ `max_iterations` なら T ではなく T の `on_exhausted` へ（実行回数カウントは既存 `step_execution_counts` 後継を流用）。
4. `rules` 空なら WorkflowExecution 完了。

正規形 R1〜R7 により、任意の Artifact 値で結果はちょうど 1 つ（property test の対象、#1327/#1323）。

## 7. Diagnostic 詳細設計

```rust
pub struct Diagnostic {
    pub code: String,        // 下表が唯一の正。テストで固定
    pub severity: Severity,  // Error | Warning
    pub stage: Stage,        // ParseShape | Resolve | Typecheck | ControlFlow
    pub span: Option<Span>,  // { start_line, start_col, end_line, end_col }
    pub message: String,
}
```

- **span 取得（P9）**: serde_saphyr の typed load と並行して saphyr AST を parse し、`YAML path（例: nodes[3].rules[1].when.on）→ Span` の map を構築（`span_map.rs`）。semantic 段（resolve 以降）は対象要素の path を組み立てて span を引く。取れない場合は最近傍 node の span に fallback。
- **段の責務**: ParseShape = YAML 構文 / unknown・旧 field / kind 個数 / kind 別許可 field（adaptor 層）。Resolve = 名前解決（node / Contract / Artifact path / 予約名 scope）。Typecheck = §4 の型検査・§6 R4/R6。ControlFlow = §6 R1〜R3・R5・R7・到達性。Resolve 以降は domain service（span なしの結果を gateway が span 付与）。
- **code 体系**（この表が唯一の正。同系列で追番可、既存 code の意味変更は不可）:

| code | 内容 |
|---|---|
| WFS001 | YAML 構文エラー |
| WFS002 | unknown field / subset 外キーワード |
| WFS003 | kind block が 0 個または 2 個以上 |
| WFS004 | kind に許可されない field（fanout への inputs 含む） |
| WFS005 | 旧構文（type:, output_contract, parallel_children, aggregate, match:, cycle_guard, pass_output_from, variables 等） |
| WFS006 | node 名重複 / 名前形式違反 |
| WFR001 | 未定義 node 参照（rules target / fanout child / inputs） |
| WFR002 | 未定義 Contract 参照（artifact / input / items） |
| WFR003 | 未定義または参照不能な Artifact path（artifact 無し session への参照、fanout child 名の参照を含む） |
| WFR004 | 予約名の誤用（request / item を node 名に、schemas に request 宣言） |
| WFR005 | `item` の scope 外使用 |
| WFT001 | when.on が routing 可能 Boolean でない |
| WFT002 | switch.on が routing 可能 enum でない / cases に enum 外の値 |
| WFT003 | fanout items と child input の不整合（要素型不一致・items 無しで child が input 宣言・items 有りで child が input 未宣言） |
| WFT004 | artifact: に Object 以外の Contract / fanout への artifact 宣言 |
| WFT005 | 予約 field 衝突（command artifact Contract が ok 等を宣言） |
| WFT006 | 判別 rule 不能な node への When/Switch（fanout / artifact 無し session） |
| WFC001 | 到達不能 node |
| WFC002 | 排他違反（判別 rule 2 個以上、Next 重複） |
| WFC003 | 網羅違反（catch-all 欠落、被覆済み switch への next、R4 の next 欠落） |
| WFC004 | switch enum の被覆漏れ（next 無しの場合） |
| WFC005 | 到達可能な loop_guard の無い cycle |
| WFC006 | fanout child の leaf 違反（child への通常遷移 / entry / fanout の入れ子） |

## 8. Runtime 実行経路

engine は kind 単位の 3 経路に整理する（monolith への追記禁止、goal-common 実装原則 7）。

### 8.1 command（#1328）

`command_runner.rs`（infrastructure）: `/bin/sh -c <command>`、cwd = worktree、**process group を作って spawn**、stdout/stderr を既存 output 制限で capture、`duration` は ms。→ gateway が結果を Artifact 化:

1. `{{ }}` 補間（§5）→ 実行 → 予約 field 生成。
2. `artifact:` 有り: stdout を JSON parse → Contract 検証（§4）→ 成功: `予約 field ∪ Contract fields` を Artifact として保存、`ok = exit_code==0`。失敗（parse 不能 or Violation）: Artifact は予約 field のみ、`ok = false`。**node は完了扱い**（NodeCompleted）で §6.2 の routing（catch-all へ）に進む。
3. `artifact:` 無し: Artifact = 予約 field のみ、`ok = exit_code==0`。
4. process 起動自体の失敗のみ NodeFailed（infrastructure_crash）。
5. **cancellation**: abort / stop（§8.5）/ アプリ終了時、engine は実行中 command の **process group を kill** する（既存 child process の staged shutdown 機構を再利用）。kill された NodeExecution は完了させず、abort なら ExecutionAborted、stop / アプリ終了なら ExecutionInterrupted に帰着する。timeout は構文スコープ外のまま（hang した command が stall observation の対象外である点は既知の制約として #1337 で文書化）。

### 8.2 session（#1322 / #1324 / #1325）

既存 session 実行を維持し、完了判定だけ再構成する: turn 完了 → `artifact:` 有りなら検証済み提出（§4、CLI submit / repair 機構）を待つ（P13）→ `gate: auto` は即完了、`gate: approval` は Approve command まで waiting_approval（追加指示は同一 session へ、既存 approval chat 経路）。reject / rerun 経路は存在しない（#1324）。

### 8.3 fanout（#1329 / #1330）

1. `items` 解決（Literal はそのまま / ArtifactField は実行時の Artifact から。要素数 0 → 子 0 個で即完了、Artifact = `[]`）。
2. 展開: `child × items`（items 無しは child のみ）。各展開は**通常の NodeExecution**として起動（`node_execution_id` 採番、`fanout_parent { parent_node, parent_attempt, item_index?, child_index }` 参照付き、§9）。child の `rules` は評価しない（P3）。`item` を child の input / 参照に束縛。
3. 全子完了 → fanout の Artifact = 子 Artifact の配列。**配列要素**: session child は検証済み Artifact（`artifact:` 無しの session child は `null`）、command child は標準結果 ∪ Contract fields。順序は items 順 × child 宣言順（行優先で平坦化）。→ 親の ArtifactProduced（contract 無し、§9）として記録し、fanout node の rules（Next / LoopGuard のみ）で遷移。
4. 集約機構なし（#1330 で aggregate / collect 削除）。畳み込みは後続の reducer node（通常 node）が `inputs: [<fanout名>]` で配列を受けて行う。

### 8.4 失敗・repair

既存 `WorkflowStepFailureKind` / RetryPolicy / stall observation を維持（P7）。structured output repair は §4 Violation ベースの prompt 生成に置換。子の一部失敗は fanout policy を持たず #1335 の resume で扱う。

### 8.5 stop / resume（#1335）

- status に `Interrupted` を追加（P2）。crash / stale / **stop（明示停止）** / orphan 検出で Interrupted になる（自動 abort しない）。
- **Stop typed command を新設**: `StopExecution { execution_id }`。Running / WaitingApproval の WorkflowExecution を ExecutionInterrupted{reason: stop} で中断する（実行中 session の turn 中断・command の kill を含む。§8.1）。API `POST /v1/workflow/executions/{id}/stop`、CLI `releash workflow stop <execution-id>`、UI の stop アクションが入口。
- `ResumeExecution { execution_id }`: event log を replay して「確定済み NodeExecution（NodeCompleted 済み）」を復元 → 未確定 node を次の attempt として再実行（session 再アタッチはしない）。fanout 途中なら完了済み child Artifact を再利用し未確定 child のみ再実行（P5）。
- **許可状態集合**: resume は Interrupted のみ。stop は Running / WaitingApproval のみ。abort は Running / WaitingApproval / Interrupted（終端状態への abort / stop / resume は拒否）。いずれも WorkflowExecution typed command として target validation（存在 / 状態 / worktree 整合）。

## 9. Event log 語彙（最終形）

NDJSON append-only（`log.rs`）。**最終形の variant を下表で固定**する。各 goal は自分の担当 event を最終形（主語 rename を除く variant 構造）で実装し、主語 rename（run_id→execution_id 等の残り）は #1331 で一括完了する（P4: 在庫互換不要）。

**fanout 親も通常の NodeExecution として NodeStarted / NodeCompleted を出す**（Fanout 専用 variant は持たない。子は `fanout_parent` で親に紐づき、fanout の境界は親 Node* event が担う）。

| variant | fields（全てに execution_id, timestamp） |
|---|---|
| ExecutionStarted | workflow_name, worktree_path, definition（snapshot）, request: String |
| NodeStarted | node_execution_id, node_name, kind, attempt, fanout_parent?: { parent_node, parent_attempt, item_index?, child_index } |
| SessionAttached | node_execution_id, session_id |
| ArtifactProduced | node_execution_id, node_name, contract: Option<String>, value（command stdout / session submit / CLI・API submit / fanout 配列の共通 event。artifact 無し command の標準結果と fanout 配列は contract = null。旧 OutputSubmitted 置換） |
| NodeCompleted | node_execution_id, node_name, attempt, result_summary?, token_usage? |
| NodeFailed | node_execution_id, node_name, attempt, reason, failure_kind, retry_count? |
| ApprovalRequested / ApprovalResolved | node_execution_id, node_name / + comment?（approve のみ。reject record は存在しない） |
| ContractViolated | node_execution_id, node_name, violations, repair_attempt |
| StallObserved / StallCleared | 既存踏襲 |
| ExecutionCompleted / ExecutionFailed / ExecutionAborted | total_token_usage / reason, failure_kind / aborted_node? |
| ExecutionInterrupted / ExecutionResumed | reason（crash\|stale\|stop\|orphan）/ resume_from_node（#1335） |

削除される variant: StepSessionStarted（→SessionAttached）、OutputCollected、Parallel* 全て（→NodeStarted.fanout_parent + 親の Node*）、ContractRepairRequested（→ContractViolated）、CliMutationRequested / CliMutationRejected（pending file と共に削除、#1332）。

## 10. Read model / projection（#1331）

保存: メタ `workflow_executions/{execution_id}.json`、event log `workflow_execution_logs/{execution_id}.ndjson`。state は非永続（event replay の on-demand projection、既存方式維持・full-retention 経路を増やさない）。

```rust
pub struct WorkflowExecution {
    pub id: String,                       // 旧 run_id
    pub workflow_name: String,
    pub status: ExecutionStatus,          // Running | WaitingApproval | Interrupted | Completed | Failed | Aborted
    pub current_node: Option<String>,
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,    // DesktopUi | Cli | Agent | Api（GLOSSARY 禁止語 TriggerSource は使わない）
    pub started_at / updated_at / completed_at?: f64,
    pub error_reason: Option<String>,
    pub total_token_usage: TokenUsage,
}
pub struct NodeExecution {
    pub id: String,                       // node_execution_id（§5。第一級の識別子）
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,               // command | session | fanout
    pub attempt: u32,                     // 同一 node の反復回（fanout child の並走個体は fanout_parent で識別）
    pub status: NodeStatus,               // Running | WaitingApproval | Succeeded | Failed | Aborted
    pub session_id: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub token_usage: Option<TokenUsage>,
    pub failure: Option<{ reason, kind }>,
    pub fanout_parent: Option<FanoutParentRef>,   // { parent_node, parent_attempt, item_index?, child_index }
    pub started_at / completed_at?: f64,
}
```

Fanout は「`fanout_parent` で子を束ねた derived view」（親 NodeExecution + 子 NodeExecution 列 + 配列 Artifact）。Artifact map（`request` 含む、fanout child を除く §5）も projection が提供。**公開 DTO / Tauri command / frontend 型はこの 2 型 + Fanout view + Diagnostic のみ**を語彙とし、旧 `WorkflowStateSnapshot` / `StepHistoryEntry` / `StepOutput` / `ParallelStepState` は削除。Tauri command rename は `goal-10-issue-1331.md` 実装内容 4 の一覧どおり。

## 11. Local API（#1332、D1）

axum を `adaptor/controller/api/` に新設。127.0.0.1 の ephemeral port、起動時に `{data_dir}/local-api.json`（`{ port, token, pid }`、0600）を書き、終了時に削除。全 endpoint は `Authorization: Bearer <token>` 必須。handler は Tauri command と**同じ usecase** を呼ぶ薄い controller（logic 追加禁止）。エラーは `{ code, message }`。

| method / path | 対応 |
|---|---|
| GET `/v1/workflows` | 定義一覧（query service） |
| GET `/v1/workflow/executions?worktree=&status=` | 一覧 |
| POST `/v1/workflow/executions` `{workflow_name, worktree_path, request, permission_mode?, created_from?}` | StartExecution（P12: name 解決、request Artifact 化。created_from は cli\|agent\|api の自己申告、既定 api） |
| GET `/v1/workflow/executions/{id}` | WorkflowExecution + NodeExecution 一覧 |
| GET `/v1/workflow/executions/{id}/log` | event log |
| POST `/v1/workflow/executions/{id}/approve` `{node, node_execution_id?, comment?}` | ApproveNode（同名 waiting が複数なら node_execution_id 必須。無指定で曖昧なら候補一覧付きエラー） |
| POST `/v1/workflow/executions/{id}/abort` / `/stop` / `/resume` | Abort / Stop / Resume（§8.5。stop / resume は #1335） |
| POST `/v1/workflow/executions/{id}/artifacts` `{node, node_execution_id?, contract, value}` | Artifact 提出（ArtifactProduced。並走時は node_execution_id 必須） |
| POST `/v1/workflow/executions/{id}/artifacts:validate` | 検証のみ（副作用なし） |
| GET `/v1/workflow/executions/{id}/artifacts/{node}` | Artifact 取得（fanout 親名は配列を返す） |

## 12. CLI（#1332）

```
releash workflow list
releash workflow executions [--worktree <path>] [--status <s>]
releash workflow start <workflow-name> "<request>" [--worktree <path>] [--permission ask|edit|full]
releash workflow status <execution-id>
releash workflow logs <execution-id>
releash workflow approve <execution-id> --node <node> [--node-execution <id>] [--comment <c>]
releash workflow abort <execution-id>
releash workflow stop <execution-id>              # #1335
releash workflow resume <execution-id>            # #1335
releash workflow output submit <execution-id> --node <node> [--node-execution <id>] --type <contract> (--json <j> | --file <f>)
releash workflow output validate <execution-id> --node <node> --type <contract> --file <f>
releash workflow output get <execution-id> --node <node>
```

- `--node-execution` は同名 NodeExecution が複数 active な場合（fanout child 並走）に必須。**session 内から実行される CLI は env `RELEASH_NODE_EXECUTION_ID`（§5）を既定値に使う**ため、workflow の instruction は通常 `--node-execution` を書かなくてよい。
- discovery ファイル（`local-api.json`）があれば API 経由（無ければ: mutation は「アプリ起動が必要」エラー、read-only のみ file-direct fallback）。`runs` / `run_id` / `--step` / `reject` は存在しない。CLI help の agent 注入文・repair prompt・instructions facet の CLI 例文も本表に揃える。

## 13. Frontend（D7）

- **Automation panel**: WorkflowList（一覧・複製・削除）+ Monaco YAML editor + Diagnostic 表示（保存時に backend へ invoke → `Diagnostic[]` を受けインライン marker + 一覧表示）。フォーム編集（StepEditor / WorkflowEditor のフィールド UI）は削除。facet（policy/knowledge/instruction）の管理 UI は現行機能を維持する。
- **WorkflowView**: WorkflowExecution summary / NodeExecution timeline（kind・status・attempt・fanout グループ表示）/ Artifact viewer（JSON）/ approve ボタン（gate: approval の waiting NodeExecution 単位。node_execution_id でアドレス）/ abort / stop / resume。reject UI なし。
- **型**: `types/workflow.ts` は §10 の鏡像（WorkflowExecutionSummary / NodeExecutionView / DiagnosticView / SchemaDef 表示用）のみ。validation・分岐判断ロジックを frontend に置かない。

## 14. 削除一覧（旧 → 処置）

| 旧 | 処置（goal） |
|---|---|
| `NodeType` / `type:`（ChildNodeDefinition の type: 含む） | §2 NodeKind / InterimChild（#1322） |
| node 直下 policy/knowledge/instruction（child の flat facet 含む） | session.facets（#1322） |
| `inline_prompt` | 削除（#1322） |
| `output_contract` / `input_contracts` / contract facet / contract-validation メタブロック / spec-directory ハードコード | §4 schemas（#1325） |
| `pass_output_from` / `pass_previous_response` / `variables:` / `{{task}}` / `{{vars.*}}` / `{{project_name}}` / `{{path_alias.*}}` | §5 inputs / 参照規約（#1326） |
| `TransitionRule(match/next)` / regex routing / node 直下 `cycle_guard` / `resets_cycle_for` | §6 rules（#1327） |
| `DiagnosticItem`（code/span なし） | §7 Diagnostic（#1323） |
| `type: bash` 残骸 / UnexpectedNodeType(Bash) | §8.1（#1328） |
| reject / rerun / can_reject / ApprovalDecisionInput::Reject / CLI Reject | §8.2（#1324） |
| `parallel_children` / `ChildNodeDefinition` / `InterimChild` / Parallel* event | §8.3 / §9（#1329） |
| `aggregate` / `all_match` / `any_match` / `collect` / `ReduceStrategy` / OutputCollected | §8.3（#1330） |
| `WorkflowStateSnapshot` / `StepHistoryEntry` / `StepOutput` / `ParallelStepState` / `run_id` / `WorkflowRun` / `TriggerSource` / step 語彙 | §10（#1331） |
| pending file 機構 / CliMutation* event / `--step` / `runs` | §11-12（#1332） |
| abort-only orphan recovery | §8.5（#1335） |
| docs の未確定・懸念節 / model-boundary doc / **full-pipeline.yml の不整合（fix_result 未宣言・permission: read・routing field の required 未宣言 = lgtm / all_lgtm / has_open / verdict）** / syntax doc への required 要件と rules 要素形（catch-all は when/switch の sibling `next`）の明記 | 正本化（#1337） |

## 15. goal ↔ design 対応

| goal | 実装する設計節 |
|---|---|
| 01 (#1322) | §2（NodeKind / RawNode / InterimChild、rules・fanout は暫定形）, §3, §13 の editor 置換 |
| 02 (#1325) | §4, §5 の Artifact 保存, §8.2 の提出・repair, §9 ArtifactProduced |
| 03 (#1326) | §5, §9 ExecutionStarted.request |
| 04 (#1327) | §6, §2 Rule |
| 05 (#1323) | §7, span_map, fixture suite |
| 06 (#1328) | §8.1（cancellation 含む）, §2 CommandSpec 実行 |
| 07 (#1324) | §8.2 gate 必須化・reject 削除 |
| 08 (#1329) | §8.3, §2 FanoutSpec 完成形, §5/§9/§10 の node_execution_id・fanout_parent, §6 R7 |
| 09 (#1330) | §8.3 集約削除, §14 該当行 |
| 10 (#1331) | §9 最終化, §10, §13 型 |
| 11 (#1332) | §11, §12 |
| 12 (#1335) | §8.5（stop / resume）, §9 Interrupted/Resumed |
| 13 (#1337) | §14 の総ざらい + docs 正本化（full-pipeline の required 追加含む） + 実行検証 |
