# Design

## The actual design

### Architecture

本設計は [`specs/unified-node-model/decisions.md`](../unified-node-model/decisions.md) の「定義と展開」、[`specs/unified-node-model/syntax.md`](../unified-node-model/syntax.md)、`docs/architecture/DOMAIN.md`、`docs/architecture/GATEWAY.md`、`docs/architecture/INFRASTRUCTURE.md`、[`docs/specs/milestone-82/design.md`](../../docs/specs/milestone-82/design.md) §7（Diagnostic code 表）、および現行実装である `src-tauri/src/domain/workflow/value_objects/definition.rs`、`src-tauri/src/domain/workflow/services/validation.rs`、`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs`、`src-tauri/src/adaptor/gateway/workflow/storage.rs` を根拠とする。

#### 責務 owner

定義言語の意味論（Node の種別・配線・完了・検証規則・名前空間）は workflow domain が所有し続ける。Lua は同じ `WorkflowDefinition` を組み立てる第二の入口であり、規則を持たない。Lua 経路が独自に判定してよいのは「Lua の値として書かれたものが定義の器に収まるか」までで、定義として正しいかは既存の検証層が判定する。

Lua state の生成、標準ライブラリの選択、chunk の評価、メモリと命令の上限、`require` のファイル解決、評価失敗と位置情報の raw な受け渡しは infrastructure が所有する。mlua の型は infrastructure の外へ出さない。

Lua の値と `WorkflowDefinition` の相互変換、Lua 位置と `DiagnosticSpan` の写像、`releash` モジュールと facet インデックスの実体、型スタブの生成は adaptor/gateway が所有する。

名前空間の規則（node 名の一意性、無名エントリの合成内部名 `<合成子名>#<index>`、予約語の禁止）は domain が所有する。現在この規則は `deserialize_node_catalog` / `CatalogNormalizer`（`definition.rs:1104-1140`、`definition.rs:969-1102`）の内部にしか入口がなく serde に結合しているため、serde 非依存の関数として切り出し、YAML 経路（`deserialize_node_catalog`）と Lua 経路の双方がそれを呼ぶ。規則の二重実装を作らない。

#### 主要な変更対象

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/Cargo.toml` | mlua（`lua54` + `vendored` + `serde`）を追加する |
| `src-tauri/src/infrastructure/lua/`（新規） | Lua state の生成と標準ライブラリの選択、chunk 評価、メモリ・命令上限、`require` のファイル解決、評価失敗と `(source, line)` の raw な返却 |
| `src-tauri/src/domain/workflow/value_objects/definition.rs` | 名前空間規則を serde 非依存の関数へ切り出し、`deserialize_node_catalog` をその呼び出しへ組み替える |
| `src-tauri/src/adaptor/gateway/workflow/lua/`（新規） | `releash` / facet インデックスモジュールの実装、ハンドルとその arena、ハンドルのグラフ → `WorkflowDefinition` 構築、node 名 → Lua 位置マップ、型スタブ生成 |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs` | Lua ソース用の入口を追加し、定義レベル診断へ Lua 位置を付ける。`load_all_workflows`（`:1357`）を `.lua` へ拡張する |
| `src-tauri/src/adaptor/gateway/workflow/storage.rs` | 定義ファイルの列挙・解決・load を「拡張子ごとの loader」へ一般化する（`:203` / `:246` / `:271` / `:379`）。保存経路は `.yml` のままにする |
| `src-tauri/src/adaptor/gateway/workflow/runtime_resolver.rs` | name 解決の走査対象に `.lua` を含める（`:57`） |
| `src-tauri/src/adaptor/gateway/workflow/definition_repository.rs` | `get` の解決を拡張し、`save` / `duplicate` は Lua 定義を対象外として拒否する |
| `src-tauri/src/adaptor/gateway/workflow/editor_gateway.rs` | `.lua` を外部エディタで開けるようにする（`:72-83`） |
| `src-tauri/src/usecase/workflow/dto.rs` / `query_service.rs` / `definition.rs` | summary / detail に定義形式を載せ、Lua 定義に対する保存・複製を拒否する |
| `src-tauri/src/adaptor/controller/command/workflow/definition.rs` | 型スタブ再生成の契機（facet 変更時）を既存 command から呼ぶ |
| `src/components/panels/automation/WorkflowList.tsx` / `WorkflowDetail.tsx` | 定義形式の表示、Lua のときの Monaco 非表示と診断一覧表示 |

`domain/workflow/services/validation.rs` の検証規則、`routing.rs` の実行時評価、事実ログ、実行木 read model は変更しない。

### Interface

#### Lua 定義言語

`releash` モジュールは次の関数と定数だけを公開する。引数は常に単一の table（`r.next` / `r.retry` / `r.input` / `r.schema` のプリミティブを除く）で、既知キー以外は呼び出し時点で拒否する。`?` は省略可能を表す。

| 呼び出し | 返す型 | 備考 |
| --- | --- | --- |
| `r.command{ name?, command, artifact?, input?, completion? }` | `Node` | |
| `r.session{ name?, provider, model?, permission?, facets?, artifact?, input?, completion? }` | `Node` | `model` / `permission` は provider CLI の語彙をそのまま渡す文字列 |
| `r.fanout{ name?, children, items?, artifact?, input?, completion? }` | `Node` | |
| `r.sequence{ name?, entry?, output?, children, artifact?, input?, completion? }` | `Node` | `entry` / `output` は自分の children に含まれる `Node` |
| `r.child{ node, inputs?, rules?, on_failure? }` | `Child` | children の要素はすべてこの形 |
| `r.next(node)` | `Rule` | |
| `r.when{ on, on_true, next }` | `Rule` | `on_true` は `on` が true のときの遷移先（YAML の `then`）、`next` は false のときの遷移先 |
| `r.switch{ on, cases, next? }` | `Rule` | `cases` は `<値> = Node` のマップ |
| `r.loop_guard{ max_iterations, on_exhausted }` | `Rule` | `on_exhausted` は遷移先の `Node` |
| `r.retry(n)` | `OnFailure` | |
| `r.ignore` | `OnFailure` | 定数 |
| `r.input(name, contract?)` | `Input` | `contract` は `Schema` |
| `r.request` | `Source` | 予約供給元 |
| `r.items` | `Source` | fanout の展開要素 |
| `r.completion.approval` | `Completion` | 既定（auto）は省略で表す |
| `r.provider.claude` / `r.provider.codex` | `Provider` | |
| `r.schema.object{ name?, properties, required? }` | `Schema` | `properties` は `<名前> = Schema` |
| `r.schema.array{ name?, items }` | `Schema` | `items` は `Schema` |
| `r.schema.string{ enum? }` / `r.schema.boolean()` / `r.schema.integer()` / `r.schema.number()` | `Schema` | |
| `r.workflow{ name, description, main }` | `Workflow` | chunk の戻り値がこれでなければ拒否 |

参照の型:

- `Node` は `Source` として使える（その node の Artifact 全体）。`node.<field>` は `Source`（Artifact の field）を返す。field は node が宣言した `artifact` の `Schema` に対して即座に検査される。
- `Input` は `Source` として使える。
- `inputs` は `<パラメータ名> = Source` のマップ。供給元に使えるのは、その合成子の children の `Node`、その合成子自身の `Input`、`r.request`、および fanout での `r.items` だけである。
- `when.on` / `switch.on` / `fanout.items` は `Source`。`items` はリテラル配列（Lua の配列 table）も受ける。

facet インデックスモジュールは `f.instruction.<key>` / `f.policy.<key>` / `f.knowledge.<key>` を公開し、それぞれ kind を持つ `Facet` を返す。`facets = { policy = <Facet>, knowledge = { <Facet>, ... }, instruction = <Facet> }` で、kind の合わない Facet は呼び出し時点で拒否する。`knowledge` は常に配列で書く（YAML の単数受けは Lua へ持ち込まない）。

部品は `sequence` を返す Lua 関数として書く。sequence を返すことで、実行木に子 sequence として現れ、折り畳みと `completion: approval` の単位になる（B-003）。engine から見れば部品も通常の node であり、部品由来かどうかを識別する情報は定義に残らないため、この規約は Lua 側の書き方として守る（葉を直接返す関数を engine が拒否することはできない）。同じ `Node` の値を複数の `r.child{}` へ渡すことは拒否する（B-005）。再利用は関数を再度呼ぶことで行う。

#### 生成物

workflows ディレクトリへ次を出力する。いずれも編集を前提としない。

| Path | 内容 |
| --- | --- |
| `.releash/releash.lua` | `---@meta`。`releash` モジュールの `@class` / `@param` / `@return` 型定義。静的なテンプレート |
| `.releash/facets.lua` | `---@meta`。facet インデックスの型定義。各エントリの docstring に facet の説明と本文 md への `file://` リンクを含む |
| `.luarc.json` | `workspace.library` に `.releash` を、`runtime.version` に `Lua 5.4` を設定する。**既に存在する場合は生成しない** |

これらは補完のためだけに存在する。実行時に `require("releash")` / `require("facets")` が解決するのは Rust が提供するモジュールであり、生成物が欠けていても古くても load 結果と実行結果は変わらない（R-009、B-011）。

#### Tauri command / DTO

新しい Tauri command、WebSocket message、HTTP route は追加しない。既存 DTO を additive に拡張する。

- `WorkflowSummaryDto` / `WorkflowDto` に `sourceFormat`（`"yaml"` | `"lua"`）を追加する。
- `get_workflow_source` は Lua 定義に対して呼ばれない（frontend が呼ばない）。呼ばれた場合は Lua ソースをそのまま返す。
- `save_workflow_source` / `duplicate_workflow` は Lua 定義を対象にしたとき、定義形式を理由に拒否する。判定は Rust 側で行い、frontend はボタンの表示可否に使うだけとする。
- `open_workflow_in_editor` は `.lua` を解決できるようにする。builtin の拒否は変更しない。
- `diagnose_all` の戻り形は変更しない。Lua 由来の `DiagnosticItem` も既存の `code` / `severity` / `stage` / `span` / `message` に載る。

#### 内部境界

| Interface | Responsibility |
| --- | --- |
| `WorkflowDefinitionLoader`（拡張子ごとの loader） | 定義ファイルのパスを受け、`WorkflowDefinition` と診断を返す。YAML 版と Lua 版が実装する |
| `LuaEvaluator`（infrastructure） | chunk 名・ソース・workflows ディレクトリ・上限を受け、評価結果か、位置情報付きの失敗を返す |

### Data Model

Lua 側のハンドルはすべて mlua の UserData とし、実体は評価中だけ生きる arena（`Vec` と index）に置く。UserData が持つのは arena の index と種別だけで、定義そのものは持たない。これにより値の同一性（同じ `Node` を2回置いたか）が index の比較で判定でき、Lua 側から定義を書き換える経路も生まれない。

| ハンドル | arena の実体 |
| --- | --- |
| `Node` | kind ごとの spec、`name`（明示分のみ）、`artifact`、`input`、`completion`、宣言位置 |
| `Child` | 対象 `Node` の index、`inputs`、`rules`、`on_failure`、宣言位置 |
| `Source` | 供給元の種別（node / node.field / input / request / items）と対象 index |
| `Input` | パラメータ名、`Schema` の index |
| `Schema` | `SchemaDef` 相当、`name`（明示分のみ） |
| `Facet` | kind と key |
| `Workflow` | `name`、`description`、root の `Node` index |

評価が終わった時点で arena から `WorkflowDefinition` を構築し、arena と Lua state は破棄する。`WorkflowDefinition` に Lua 由来の情報は残さない（R-001、B-001）。診断のための `node 名 → (file, line)` マップは構築時に併せて作り、診断の生成が終わるまでだけ保持する。

### Database

該当なし。定義は実行開始時に `WorkflowDefinition` として engine へ渡り、事実ログ・read model・resume の扱いは YAML 由来と同一である。永続スキーマ、event 語彙、projection のいずれも変更しない。

### UI/UX

一覧（`WorkflowList.tsx`）は既存の builtin バッジと同じ位置に定義形式を表示し、診断件数バッジは現行のまま使う。

詳細（`WorkflowDetail.tsx`）は `sourceFormat` で分岐する。`yaml` は現行のまま（Monaco・保存・複製）。`lua` では Monaco を描画せず、次だけを表示する。

- 定義形式と、外部エディタで開くボタン。
- 診断の一覧。各行は `<ファイル名>:<行番号>`、code、メッセージを持つ。ソース本文は表示しない。

新しい編集手段・保存手段は追加しない。Monaco は Lua に対して一切使わない。

### Algorithm

#### load パイプライン

Lua 定義の load は次の順で行い、いずれかの段でエラーが出たらそこで止める。

1. **評価** — 標準ライブラリを絞った Lua state を作り、`releash` と facet インデックスを Rust のモジュールとして登録し、対象ファイルを chunk 名付きで評価する。chunk の戻り値が `Workflow` でなければ拒否する。
2. **構築** — arena から `WorkflowDefinition` を組み立てる。node 名の決定と正規化は domain の名前空間規則を使う。
3. **定義レベル検証** — 既存の `diagnose_workflow_definition(&workflow, None)` を呼び、返った診断に `node 名 → Lua 位置`マップで span を付ける。
4. **facet 解決と template 参照検証** — 既存の `resolve_and_validate_workflow_facets` を YAML 経路と同じく呼ぶ。

1 と 2 は Lua 経路固有、3 と 4 は YAML 経路と共有する。これにより「同じ誤りには同じ診断」が構造的に成立する（R-007、B-008）。

#### 名前の決定と正規化

- `r.workflow{ main = X }` に渡された `Node` はカタログ名 `main` を持つ。X に `name` が明示されていた場合は拒否する（規約名と別名の二重管理を作らない）。
- `name` を明示した `Node` はその名前を持つ。予約語（`RESERVED_NODE_NAMES`）は拒否し、重複も拒否する。
- `name` を持たない `Node` は、それを子に持つ合成子の名前と children 内の index から `<合成子名>#<index>` を得る。YAML の無名エントリと同一規則を同一関数から得る。
- `Schema` も同じ規則で、明示名か自動生成名を持ち、`schemas` カタログへ登録される。

構築は `main` から children をたどる走査で行い、到達した順にカタログへ並べる。到達しない `Node` はカタログに載せない（Lua では変数に束ねただけの node は定義に現れない）。

#### 診断コードの割当

§7 の code 表は「同系列で追番可、既存 code の意味変更は不可」であり、stage は接頭辞で決まる。Lua 固有の失敗にも新しい接頭辞は作らず、既存系列へ追番する。同じ意味を持つ既存 code は再利用する。

| 事象 | code | 扱い |
| --- | --- | --- |
| Lua の構文エラー | `WFS009` | 新規（ParseShape） |
| 評価の失敗、上限超過による打ち切り、chunk が `Workflow` を返さない | `WFS010` | 新規（ParseShape） |
| `require` の解決失敗、workflows ディレクトリ外の参照、循環 require | `WFS011` | 新規（ParseShape） |
| ビルダー引数の未知キー・型不一致・必須キー欠落 | `WFS002` | 既存（unknown field / unknown variant）を再利用 |
| workflow 名がファイル名と一致しない、node 名の重複・予約語 | `WFS006` | 既存（名前重複 / 名前形式違反）を再利用 |
| 同一 `Node` を複数の children へ置いた | `WFC007` | 既存（children の重複参照）を再利用 |
| スコープ外の供給元を配線した | `WFR007` | 既存（未解決の input 供給元）を再利用 |
| 存在しない facet、artifact 未宣言 node の field 参照、`main` 欠落 | `WFR900` / `WFR003` / `WFR006` | 既存をそのまま |

`WFS009` / `WFS010` / `WFS011` の span は Lua の `(file, line)`、`WFS002` の span はビルダー呼び出し位置、既存 code の span は node の宣言位置とする。

#### 評価の閉じ込めと有界化

- 標準ライブラリは `table` / `string` / `math` のみを読み込む。`io` / `os` / `package` / `debug` は読み込まない。加えて base の `load` / `loadstring` / `dofile` / `loadfile` は評価前に `nil` を代入して落とす。これにより評価結果が時刻・環境変数・外部ファイルに依存しなくなる（R-010、B-012）。
- `require` は Rust 実装をグローバルへ置く。モジュール名をドット区切りとして workflows ディレクトリ基準で `.lua` へ解決し、正規化した実パスがディレクトリ配下でなければ拒否する。同一評価内でモジュールを一度だけ評価してキャッシュし、循環 require は検出して拒否する（R-012、B-014）。
- メモリ上限を 64 MiB、命令上限を 5,000 万とする。いずれも定義の評価を一度きり行う用途に対して十分に大きく、終了しない定義をアプリの寿命内で確実に打ち切る。超過は `WFS010` の診断として観測でき、他の workflow の load・実行は影響を受けない（R-011、B-013）。
- 評価はすべて既存の `spawn_blocking` の内側で完結させ、Lua state を await をまたいで保持しない。

#### 生成物の更新

型スタブと facet インデックススタブは、アプリ起動時と、facet の作成・削除・リネームの成功時に再生成する。生成は冪等とし、既存ファイルと内容が一致する場合は書き込まない。生成に失敗しても load・実行・診断は継続する（生成物は補完のためだけに存在するため）。

`.luarc.json` は存在しない場合だけ生成する。既存ファイルがあれば内容を検査も変更もしない。

### Infra

mlua を `lua54` + `vendored` + `serde` で追加する。`vendored` は Lua 5.4 のソースを同梱してビルドするため C コンパイラを要求するが、Tauri のビルドで既に満たされている。`send` feature は使わない（Lua state は `spawn_blocking` の内側で閉じるため `Send` を必要としない）。

新しいプロセス・サービス・データベース・デプロイ構成は追加しない。

## Alternatives Considered

- **Lua の値を serde 経由で YAML 相当の中間表現へ落とし、既存の `Deserialize` に流す案**: 実装は薄いが、node の値参照（同一性）を中間表現で表現できず、エラー位置もビルダー呼び出し行にならない。確定した「値参照のみ」と「呼び出し時点での検証」を満たせないため採らない。
- **文字列名による参照**: 実装は最も薄いが、LuaLS のリネーム・定義ジャンプ・未定義検出が効かず、ISSUE が掲げる目的（認知支援）を満たさないため採らない。
- **`WFL` などの新しい code 接頭辞**: §7 の「同系列で追番可」と「stage は接頭辞で決まる」規約に反し、同じ誤りに YAML と Lua で別 code が付く。既存系列への追番と既存 code の再利用を採る。
- **facet インデックスを実ファイルとして生成し `require` で読む案**: 生成物が実行時の正本の一部になり、生成物の欠落・陳腐化が load を壊す。R-009 に反するため採らない。
- **`when` の true 側遷移先に `["then"]` を使う案**: YAML の遷移先キーと一致するが、Lua の予約語のため角括弧記法が必須になり、他のキーと書き方が揃わない。`on_true` を採る。
- **アプリ内 Monaco での Lua 編集**: 補完・定義ジャンプが効かない場所で Lua を書かせることになり、Lua を採用した目的と衝突する。Releash では Monaco を使わない方針でもあるため採らない。
- **children の要素に素の `Node` も許す案**: 直列の記述は短くなるが、子の書き方が2通りになる。「同じ意味に複数の書き方を作らない」方針により `r.child{}` の一形式に統一する。
- **部品を table として返し複製 API を用意する案**: `require` のキャッシュにより同じ値が複数箇所に現れるため、暗黙または明示の複製という Lua にない概念が要る。関数として返す形は Lua の通常の書き方で同じ目的を満たすため採らない。

## Cross-cutting concerns

- **起動時コスト**: 起動時にスタブ生成と、既存の診断走査に `.lua` の評価が加わる。評価は定義ごとに一度きりで、上限により最悪時間も有界である。生成は内容一致時に書き込まない。
- **命名の衝突**: 同じ workflow 名を複数ファイルが宣言した状態は、既存の `resolve_workflow_by_name` が `WFS006` で拒否する経路をそのまま使う（`.yml` と `.lua` の組み合わせを含む）。
- **builtin**: builtin は YAML のまま同梱し、Lua 経路を通らない。builtin 名との重複拒否も現行のまま。

## Risks

- **自動生成名の可読性**: 名前を明示しない node は `<合成子名>#<index>` として実行木 UI と診断に現れる。YAML の無名エントリと同じ挙動だが、Lua では無名で書ける場面が増えるため、読みにくい名前が出やすくなる。型スタブの docstring で `name` の明示を促す以上の強制は行わない。
- **定義ファイルの権限**: Lua 定義はアプリと同じプロセスで評価される。標準ライブラリの制限と `require` の閉じ込めで到達範囲は絞るが、Lua で任意の計算ができること自体は前提である（`command` node が任意のシェルを実行できるのと同じ地平にある）。
- **ビルド要件**: `vendored` により C コンパイラがビルド要件に加わる。CI と開発環境では既に満たされているが、ビルド時間とバイナリサイズがわずかに増える。
