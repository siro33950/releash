# Context

- 要求の正本: [Issue #1591](https://github.com/siro33950/releash/issues/1591)「[統一 Node モデル] Lua による workflow 定義」（OPEN・milestone 86 の wave 4・comment なし）。
- 設計の正本: [`specs/unified-node-model/decisions.md`](../unified-node-model/decisions.md) の §定義と展開、および [`specs/unified-node-model/syntax.md`](../unified-node-model/syntax.md)（Lua が組み立てる定義は、この構文が表すものと同一の `WorkflowDefinition` である）。
- 補助資料:
  - [milestone 86](https://github.com/siro33950/releash/milestone/86)「統一 Node モデル」— wave 順序（本 Issue は wave 4。#1465 / #1466 / #1467 とは依存なし、#1468 より前）。
  - [#1463](https://github.com/siro33950/releash/issues/1463)（CLOSED・commit `09c27cbbc`）— sequence の再帰実行。部品を sequence として入れ子にできることが Lua 側の合成の土台であり、本 Issue の前提。
  - [#1464](https://github.com/siro33950/releash/issues/1464)（CLOSED・不採用）— 定義を跨ぐ参照（`ref`）は持たない。部品化と再利用は定義言語の外で行う。
  - [#1468](https://github.com/siro33950/releash/issues/1468)（OPEN・wave 8）— 文法の正本化。`docs/workflow-yaml-syntax.md` への Lua の記述追加は本 Issue ではなく #1468 が担う。
  - [`docs/specs/milestone-82/design.md`](../../docs/specs/milestone-82/design.md) §7 — Diagnostic code 表。「この表が唯一の正。同系列で追番可、既存 code の意味変更は不可」であり、stage は code 接頭辞（WFS→ParseShape / WFR→Resolve / WFT→Typecheck / WFC→ControlFlow）で決まる。
- 確定済みの背景と制約（後続の Behavior・Design が従う）:
  - 型は厳密に定め、同じ意味に複数の書き方を作らない。冗長でも書き方が一つに定まることを優先する。
  - `.lua` は load 時に一度だけ評価され、既存の `WorkflowDefinition` を返す。実行時に Lua は走らない。engine が受け取るものは YAML 由来と完全に同一である。
  - 合成は `require` と sequence の入れ子で行う。engine には常に単一の `WorkflowDefinition` を渡し、合成子境界を engine に持ち込まない（loop_guard 検知・resume・diagnostics は単一定義内に閉じたままにする）。
  - 目的は表現力の拡大ではなく、定義を書く・読むときの認知支援（型・補完・定義ジャンプ・リネーム）である。型検査の正本は既存の検証層のままで、Lua 側の型注釈は補完・定義ジャンプのためにある。
  - node・facet・Artifact field の参照は値参照で書く。文字列名で参照する経路は設けない。
  - 部品は sequence を返す関数として書く。同一の値を複数の children へ置く記述は拒否する。
  - 部品の Interface（`input` パラメータ）と配線（`inputs`）は明示する。合成子のスコープ外の値を内側から参照する記述は拒否する（engine の配線規則は「供給元は合成子のスコープに閉じる」）。
  - children エントリと rules はコンストラクタ関数で書く。
  - facet は生成された定数テーブル経由で参照する。
  - 実行時に使われるモジュール（`releash` と facet インデックス）は Rust が提供する。workflows ディレクトリへ出力する生成物は LuaLS 用の型スタブだけであり、実行時の正本にしない。
  - Lua 定義は Releash のアプリ内でソース表示・編集の対象にしない。編集は外部エディタに委ねる（Releash では Monaco Editor を使わない）。
  - builtin workflow は YAML のまま維持する。
  - 品質ゲートは本リポジトリの既定（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、`pnpm lint` / `pnpm test` / `pnpm build`、`pnpm test:integration`）。

# Outcome

- 対象者: Releash で workflow を定義・保守する開発者。
- 現在の問題: 定義言語が YAML 単一で、定義を跨いだ部品化・再利用の手段が無い（`ref` は #1464 で不採用が確定している）。定義を書くときの支援も無く、node 名・facet キー・Contract 名・Artifact field はすべて生の文字列であるため、綴りの誤りや存在しない参照は書いている最中には分からず、保存・load 時の Diagnostic まで持ち越される。同じ手順を複数の workflow で使う手段が無いため、定義は肥大した単一ファイルとして重複する。
- 変更後に実現する状態: workflow を `.lua` でも定義できる。部品は `require` で合成され、engine が受け取るのは YAML 由来と完全に同一の単一 `WorkflowDefinition` である。workflows ディレクトリを VSCode + LuaLS で開けば、ビルダー引数・node 参照・facet キーに対して補完・型検査・定義ジャンプ・リネームが効き、Releash 固有の誤りは load 時に Lua のファイル名・行番号付きで分かる。

# Current Behavior

commit `6e5c47022`（branch `feat/issues/1591`）で、以下をコード調査により確認した。調査範囲は workflow 定義の load・検証・保存・編集経路（`src-tauri/src/adaptor/gateway/workflow/`）、定義の値オブジェクトと検証規則（`src-tauri/src/domain/workflow/`）、定義を扱う frontend（`src/components/panels/automation/`）、および依存関係（`src-tauri/Cargo.toml`）である。

## 定義の入口は `.yml` だけである

workflow 定義を読む経路は5つあり、いずれも拡張子 `yml` を直接判定する。

- 実行開始時の name 解決: `resolve_workflow_by_name` が workflows ディレクトリを走査し、`yml` のみを対象に全件 load してから `WorkflowDefinition.name` で一意解決する（`runtime_resolver.rs:57`）。複数ファイルが同じ name を宣言した状態は WFS006 で拒否される（`runtime_resolver.rs:33-38`）。
- 一覧: `storage::list_workflows` → `list_yml_summaries` が `yml` のみを列挙する（`storage.rs:271`、`storage.rs:246`）。summary の name はファイル stem で上書きされる（`storage.rs:255-259`）。
- 単体取得: `WorkflowDefinitionFileRepository::get` が `resolve_workflow_path`（`<name>.yml` の存在確認。`storage.rs:379`）→ `storage::load_workflow`（`storage.rs:203`）を呼び、見つからなければ builtin にフォールバックする（`definition_repository.rs:128-141`）。
- 全体診断: `diagnostics::load_all_workflows` が `yml` のみを走査する（`diagnostics.rs:1357`、`diagnostics.rs:1366`）。
- 外部エディタ起動: `resolve_workflow_editor_path` が `resolve_workflow_path` に委譲するため、`<name>.yml` しか開けない（`editor_gateway.rs:72-83`）。

保存経路も `<name>.yml` への書き込みに固定されている（`storage.rs:118-137`、`storage.rs:172-194`）。

## 定義レベルの検証は YAML に依存していない

`diagnose_workflow_source` は3段で構成される（`diagnostics.rs:150-207`）。

1. `YamlSpanMap::parse` で YAML の位置情報を作る（`span_map.rs`）。
2. raw JSON 値に対する形の診断（`parse_shape_diagnostics`）。
3. `serde_saphyr` で `WorkflowDefinition` へ deserialize し、`diagnose_workflow_definition(&workflow, Some(&span_map))` を呼ぶ。

このうち 3 の `diagnose_workflow_definition` は `span_map: Option<&YamlSpanMap>` を取り（`diagnostics.rs:210-212`）、domain の検証規則（`domain/workflow/services/validation.rs`）を呼ぶ。すなわち定義レベルの検証（WFR / WFT / WFC 系）は YAML 表現に依存しておらず、位置情報は付加的である。facet 参照の解決と template 参照検証は `resolve_and_validate_workflow_facets` が別途行う（`storage.rs:336-352`）。

## children 4形式の正規化は serde の Deserialize 実装に埋まっている

インライン宣言・無名エントリをカタログへ登録し、無名を `<合成子名>#<index>` へ命名する正規化は `CatalogNormalizer` が担うが、その入口は `deserialize_node_catalog`（`definition.rs:1104-1140`）であり、`WorkflowDefinition` の `nodes` フィールドの `deserialize_with` としてのみ到達できる（`definition.rs:53-57`）。名前重複の検出（`definition.rs:1121-1126`、`definition.rs:1213-1220`）、kind ブロックがちょうど1つであることの検査（`definition.rs:1062-1068`）も同じ経路にある。

## 定義を跨いだ部品化・再利用の手段が無い

定義内での部品化は #1463 の sequence 再帰実行で可能になっている（`SequenceSpec` の children に sequence を置ける。`definition.rs:298-307`）。一方、別の `WorkflowDefinition` を参照する構文は存在せず、`ref` は #1464 で不採用が確定している。したがって、複数の workflow で同じ手順を使う手段は「定義をコピーする」以外に無い。

## node・facet・Artifact field の参照はすべて生の文字列である

- children エントリの参照名は `String`（`definition.rs:436-446`）、inputs の供給元は `InputSourceRef(String)`（`definition.rs:513-523`）で、`<node>.<field>` を `.` で分解して解釈する（`definition.rs:525-532`）。
- rules の遷移先・`when.on` / `switch.on` / `switch.cases` の値もすべて `String`（`definition.rs:1274-1291`）。
- facet 参照は `FacetRefs { policy: Option<String>, knowledge: Vec<String>, instruction: Option<String> }`（`definition.rs:216-236`）。未定義 facet は WFR900 で報告される。
- これらの誤りは編集中には検出されず、保存・load・診断の実行時にはじめて Diagnostic になる。

## 定義の編集はアプリ内の Monaco で行う

`WorkflowDetail.tsx` が Monaco Editor に YAML ソースを載せ（`WorkflowDetail.tsx:137-157`、`:246-262`）、Diagnostic を marker として重ね（`:290-312`）、`save_workflow_source` で保存する。Monaco を使っているのは非テストコードではこの1ファイルだけである。

## Lua ランタイムは存在しない

`src-tauri/Cargo.toml` に Lua 関連の依存は無い。YAML の deserialize には `serde-saphyr` を使っている。

# Scope / Non-goals

## Scope

- workflow 定義の入口として `.lua` を追加し、load 時の一度きりの評価で `WorkflowDefinition` を得ること。
- `releash` モジュール（`session` / `command` / `fanout` / `sequence` / `workflow`、children・rules のコンストラクタ、schema ビルダー、input パラメータ）を Rust 関数として公開し、その呼び出し時点で引数を検証すること。
- `require` と sequence の入れ子による合成、および部品を関数として再利用する規則。
- Lua 由来の失敗を、Lua のファイル名・行番号付きで既存の Diagnostic 体系に載せること。
- LuaLS 用の型定義スタブと facet インデックスの生成、および VSCode で補完・定義ジャンプ・リネームが効く状態にすること。
- Lua workflow のアプリ内での扱い（一覧への表示、診断の提示、外部エディタでの起動）。
- Lua 定義の評価の決定性・有界性・探索範囲の制限。

## Non-goals

- YAML 入口の廃止。YAML と Lua は併存する。
- builtin workflow の Lua 化。
- 文法ドキュメントの正本化（`docs/workflow-yaml-syntax.md` ほかへの Lua 記述の追加）。#1468 が所有する。
- Monaco Editor の除去、および YAML 編集 UI の廃止。別 Issue が所有する。
- `worktree: shared | isolated` の受理・解禁、および delegate（milestone #85）。
- 実行時（load 後）の Lua 評価、および実行中の定義の再評価規則の変更。
- 既存の Diagnostic code の意味変更、および YAML 経路の検証規則の変更。
- 実行木 UI の変更。

# Requirements

- R-001: `.lua` で定義した workflow は、同等の YAML 定義と完全に同一に load・実行・観測・永続化・再開される。実行木・事実ログ・read model に「Lua 由来である」という区別が現れない。
- R-002: `.lua` の評価は load 時の一度だけで、workflow の実行中に Lua は走らない。
- R-003: 部品は `require` で合成でき、engine が受け取るのは常に単一の `WorkflowDefinition` である。定義を跨ぐ参照は存在しない。
- R-004: 部品は関数として再利用でき、同じ部品を同一定義内で複数回使ったとき、それぞれが独立した node 群になる。同一の値を複数の children へ置く記述は拒否される。
- R-005: node・facet・Artifact field の参照は値で書き、存在しない参照・型の合わない参照は、その記述を書いた Lua のファイル名と行番号付きで報告される。
- R-006: 部品の Interface（`input`）と配線（`inputs`）は定義に明示され、合成子のスコープ外の値を内側から参照する記述は拒否される。
- R-007: Lua 定義に対する検証結果は、同じ誤りを YAML で書いた場合と同じ Diagnostic（code・stage・message）になる。Lua 固有の失敗に与える code は既存の code 表の体系に従う。
- R-008: workflows ディレクトリを VSCode + LuaLS で開くと、`releash` モジュールの補完と引数の型検査、facet キーの補完・定義ジャンプ・リネームが効く。facet の定義ジャンプの到達先から、その facet 本文の md へ到達できる。
- R-009: 生成物（型スタブ・facet インデックス）は補完のためだけに存在し、生成物が欠けていても古くても、workflow の load 結果と実行結果は変わらない。
- R-010: 同じ `.lua` ファイル群からは常に同じ `WorkflowDefinition` が得られる。評価結果が時刻・環境変数・外部 I/O・実行順に依存しない。
- R-011: `.lua` の評価は有界であり、終了しない定義や過大なメモリを要求する定義がアプリの動作を止めない。打ち切りは Diagnostic として観測できる。
- R-012: `require` の解決は workflows ディレクトリ配下に閉じ、その外のファイルは読み込めない。
- R-013: Lua で定義された workflow は、Releash のアプリ内でソースの表示・編集の対象にならない。編集手段は外部エディタでの起動だけである。
- R-014: Lua で定義された workflow の検証結果は、アプリ内で件数と、ファイル名・行番号付きのメッセージとして観測できる。
- R-015: 互換性要件 — 既存の YAML 定義は、変更前と同じく一覧・取得・保存・診断・実行できる。
- R-016: 互換性要件 — builtin workflow は変更前と同じく YAML として同梱され、編集不可・削除不可・名前の重複拒否の扱いも変わらない。
- R-017: 互換性要件 — 同じ workflow name を複数のファイル（`.yml` と `.lua` を含む）が宣言した状態は、変更前と同じく拒否される。

# Assumptions / Open Questions

なし。
