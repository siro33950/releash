# Design

## The actual design

### Architecture

#### 診断の実行 owner を `WorkflowReadUsecase` へ移す

`diagnose_all` は現在 `WorkflowUsecase`（`src-tauri/src/usecase/workflow/mod.rs:461-463`）が持ち、`WorkflowDiagnosticsGateway` も `WorkflowUsecase` の field である（同 `:191`）。一方、CLI が経由する local API は `WorkflowUsecase` を持たない。`LocalApiState` が保持するのは `WorkflowReadUsecase` と `WorkflowRuntimeUsecase` だけである（`src-tauri/src/adaptor/controller/api/mod.rs:19-23`）。

Diagnostic は WorkflowDefinition の検証結果であり状態遷移を伴わない読み取りである（`docs/glossary/DOMAIN.md:144-146`）。よって `WorkflowDiagnosticsGateway` の保持と `diagnose_all` を `WorkflowReadUsecase` へ移し、`WorkflowUsecase::diagnose_all()` は `self.read` へ委譲する。

これで UI（Tauri command）と local API（CLI の経路）が同一の usecase メソッドと同一の gateway 実装を通る（R-006）。

#### CLI の経路は local API のみとする

`src-tauri/src/cli/mod.rs:1-4` は「workflow command/query は localhost local API を正とする。アプリ未起動時は read-only query だけ backend-owned read model の file-direct fallback を許可する」と定めている。fallback は許可であって義務ではない。診断は local API 経由だけで実行し、file-direct fallback を持たない（Scope、Non-goals）。経路が 1 本なので、経路差による diagnostic の増減は生じない（B-005）。

未起動時の観測結果は既存と同一にする（R-012、B-012）。`api_client::mutation`（`src-tauri/src/cli/api_client.rs:126-134`）は local API へ到達できないとき `app_must_be_running_error()`（同 `:148-150`）を返し、`CliError::Other` として終了コード `1` になる。診断もこの失敗をそのまま共有し、診断専用の失敗表現を作らない。

既存の `api_client` には「read-only かつ fallback を持たない」呼び出し口が無い。`read_with_fallback`（同 `:111-123`）は fallback を要求し、`mutation` は責務が状態遷移を指す。よって local API 呼び出しの分類（`request_classified`）と、`ApiRequestError::Unavailable` を `app_must_be_running_error()` へ写す処理（`require_running`）を read / mutation いずれにも属さない内部関数として置き、その上に `mutation` と read 用の `read_without_fallback` を並べる。`mutation` を流用せず入口を 2 つに保つのは、`file_direct.rs` の冒頭が定める read / mutation の区別を CLI 側で崩さないためである。

#### 診断対象は port の引数で渡す

`WorkflowDiagnosticsGateway::diagnose_all` は現在引数を取らず、対象 directory は gateway 構築時に固定される（`src-tauri/src/adaptor/gateway/workflow/diagnostics_gateway.rs:14-23`、`wiring.rs:334-335,358-361`）。実行時指定（R-002）を満たすため、対象を port の引数へ移す。

usecase 層に対象を表す型を置き、次の 2 つだけを表現する。

- 適用済み Workflow の config directory を対象とする（gateway が構築時に保持している `workflows_dir` / `facets_base_dir` をそのまま使う）。
- 呼び出し時に指定された directory を対象とする。

指定 directory は workflow source directory として使う。Facet base は `facet.rs` を単一 owner とする解決規則により、`<dir>/facets` が directory として存在すればそこを使い、存在しなければ従来 layout の `<dir>` を使う（R-003）。

適用済み config directory の path 所有者は gateway 側（`storage::workflows_dir()`、`src-tauri/src/adaptor/gateway/workflow/storage.rs:108-113`）に残る。CLI と local API handler はこの path を再計算しない。

#### 指定 directory の診断範囲は起点からの到達集合とする

現在の `diagnose_all`（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:1442-1551`）は「その環境で使える全件」を列挙する。`load_all_workflows`（同 `:1649-1726`）は disk 上の定義に加えディスクと名前が衝突しない builtin workflow を必ず足し、`facet::list_facet_summaries`（`src-tauri/src/adaptor/gateway/workflow/facet.rs:215-`）は builtin facet を必ず足す。適用済み config directory を対象にする場合、builtin は実体が無くても `include_str!` で使えるためこの列挙は正しい。

指定 directory を対象にする場合は起点が異なる。診断が答えるのは「指定 directory の workflow が動くか」であり、判定範囲は指定 directory 配下の workflow 定義を起点として、そこから到達する Facet までである（R-013、B-013）。起点から到達しない workflow と Facet は結果に出さない。Facet 側の列挙には全件列挙ではなく既存の `collect_referenced_facet_keys`（`diagnostics.rs:1570-`）と同じ到達集合を使う。

到達判定と解決規則は変えない。`facet::facet_exists`（`facet.rs:133-`）は builtin facet を常に true とし、`facet::load_facet`（同 `:118-131`）は base_dir に実体が無ければ builtin へフォールバックする。指定 directory の workflow が builtin facet を参照していれば実行時に解決されるので、診断でも解決成功として扱い、その Facet 本文も判定範囲に入る。ここを変えると、実行できる workflow を診断が error にする。

切り替えるのは診断対象の列挙だけで、各対象へ適用する診断規則、diagnostic code、severity、message は変えない（R-011、B-011）。適用済み config directory を対象にする経路の結果は本変更の前後で同一である。

到達集合に入った Facet は、diagnostic が 1 件も出なくても `facet_summaries` に 0 件の entry を持つ。呼び出し側が「判定対象に入った Facet」と「起点から到達せず判定対象外だった Facet」を区別できる必要があるためである（B-013）。適用済み config directory を対象にする場合はこの entry を作らない。そちらは全件列挙なので判定対象と非対象の区別が生じず、entry を足すと既存の出力が変わる（R-011）。

disk 上の workflow を読み込む経路に置かない処理がひとつある。`facet::resolve_workflow_facets` の呼び出しは戻り値を捨てており、解決結果は診断のどの判定にも使われない。Facet 参照の検査は `facet_exists` を通る到達集合の側が行う。

#### 主要な変更対象

| path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/usecase/workflow/ports.rs` | `WorkflowDiagnosticsGateway` に対象引数を追加し、対象を表す型を定義する |
| `src-tauri/src/usecase/workflow/mod.rs` | `WorkflowReadUsecase` が diagnostics gateway を保持し `diagnose_all(target)` を持つ。`WorkflowUsecase::diagnose_all()` は委譲に変わる |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics_gateway.rs` | 対象から `workflows_dir` / `facets_base_dir` を解決する |
| `src-tauri/src/adaptor/protocol/workflow.rs` | 診断 DTO（`DiagnosticReport` / `DiagnosticItem` / `DiagnosticSpan` / `DiagnosticSummary` / `FacetUsageEntry` / `Severity` / `DiagnosticStage`）を wire model として置く |
| `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs`、`span_map.rs` | 診断 DTO の構築を所有する。対象種別に応じて診断対象の列挙を全件と到達集合で切り替える |
| `src-tauri/src/adaptor/controller/wiring.rs` | `WorkflowReadUsecase` の構築（`build_canonical_workflow_read_usecase` を含む）へ diagnostics gateway を配線する |
| `src-tauri/src/adaptor/controller/command/workflow/diagnostics.rs` | `diagnose_all_cmd` へ optional な対象 directory 引数を足す |
| `src-tauri/src/adaptor/controller/api/workflow.rs` | 診断 endpoint を追加する |
| `src-tauri/src/cli/workflow.rs`、新設 `src-tauri/src/cli/diagnostics.rs` | subcommand 定義と、対象解決・出力整形・終了コード導出 |
| `src-tauri/src/cli/api_client.rs` | 診断 endpoint の client と、fallback を持たない read 用の呼び出し口を足す |
| `src-tauri/src/cli/common.rs`、`src-tauri/src/cli/mod.rs` | CLI の成功値に終了コードを載せる |
| `src-tauri/src/infrastructure/local_api/client.rs` | query が空のとき URL へ `?` を付けない |
| `src-tauri/src/adaptor/controller/wiring.rs`、新設 `src-tauri/src/workflow_diagnostics_acceptance.rs` | 両経路を実 HTTP で突き合わせる acceptance harness と、その composition 入口 |

### Interface

#### CLI

```
releash workflow diagnostics [--dir <PATH>] [--json]
```

- `--dir` は診断対象の workflow source directory。Facet base は `<dir>/facets` が directory として存在すればそこを使い、存在しなければ `<dir>` を使う。省略時は適用済み Workflow の config directory を対象にする（B-002）。省略可能である必要があるため位置引数ではなく named flag にする。
- `--json` は既存 subcommand と同じ bool flag。
- 終了コード: `0`（severity error が 0 件）、`3`（severity error が 1 件以上）、および既存の `1` / `2` / `4`（command 自体の失敗）。

#### Tauri command

`diagnose_all_cmd` へ対象 directory を表す optional 引数を足す。省略時は適用済み Workflow の config directory を対象にし、戻り値の JSON shape は変えない。引数なしの既存呼び出し（`src/hooks/useAutomation.ts:58,82`）が返す結果は本変更の前後で同一である（R-011、B-011）。

UI 経路と CLI 経路の双方が対象 directory を受け取るため（R-002）、B-005 / B-011 を両経路の入口から観測して判定できる。

#### local API

```
GET /v1/workflow/diagnostics?dir=<absolute path>
```

- `dir` は省略可能。省略時は適用済み config directory を対象にする。
- response body は `DiagnosticReport` の JSON をそのまま返す。envelope で包まない（既存 read endpoint が DTO を直接返す形と同じ）。
- `dir` が空文字列または相対 path の場合は `ApiError::invalid_request`。query 型は既存 router と同じく `deny_unknown_fields` にする。
- `dir` が存在しない場合は `404 not_found`、directory ではない場合または列挙できない場合は既存の `WorkflowError::External` mapping により `500 workflow_error` とする。対象検査は gateway の Directory 分岐で共有する。

#### 内部境界

- `WorkflowDiagnosticsGateway` — 指定された対象 directory に対する診断結果 JSON を返す port。対象の解決規則（適用済み config directory がどこか）は実装側が所有する。
- `WorkflowReadUsecase::diagnose_all(target)` — UI（Tauri command）と local API が共有する唯一の診断呼び出し口。戻り値型は既存どおり `Result<serde_json::Value, WorkflowError>` を保つ（R-007、R-011）。

### Data Model

- 新しい永続 record はない。`DiagnosticReport` / `DiagnosticItem` / `DiagnosticSummary` / `FacetUsageEntry` / `DiagnosticSpan` の field 構成、serde 属性、serialize 結果は変更しない（R-011、B-006、B-011）。
- 診断 DTO は `adaptor/protocol/` に置く。CLI と Tauri command の双方が同じ型で読むためであり、複数の入口が共有する view 型の置き場所は `docs/architecture/CONTROLLER.md` が protocol と定めている。DTO の構築（`serde_saphyr` の位置情報から span を作る、`DiagnosticItem` を組み立てる）は外部世界を診断語彙へ写す変換なので gateway が所有し、protocol へは持ち込まない。
- CLI の human-readable 描画と error 件数の集計を型の上で行うため、上記 DTO へ `Deserialize` を追加する。`skip_serializing_if = "Option::is_none"` が付いている field には `serde(default)` を対にする必要がある。追加は逆方向の変換だけで、serialize 側の出力には影響しない。
- `--json` 出力は、経路から受け取った `serde_json::Value` を無加工でそのまま出す。描画のために typed へ戻した値を再 serialize しない（R-007、B-006）。
- 診断対象を表す型は usecase 層に置く値であり、永続化も versioning も持たない。

### Database

該当なし。

### UI/UX

#### human-readable 出力

`--json` を付けない場合、次を出す（B-007）。

- diagnostic 1 件につき 1 行。severity、code、発生位置、message、対象（workflow 名、または facet 種別と key）を含める。`span` を持たない item は位置欄を省く。
- 末尾に severity 別の件数（error 件数と info 件数）を出す。

#### `--help` の記載

`releash workflow diagnostics --help` に次を書く（R-010、B-010）。

- `--dir` の意味。指定 directory を workflow source として扱い、Facet base は `<dir>/facets` が directory として存在すればそこを、存在しなければ `<dir>` を使うこと。省略時は適用済み config directory が対象になること。
- 出力形式。既定は human-readable、`--json` で `DiagnosticReport` の JSON を出すこと。
- 終了コードの意味。`0` / `3` と、既存の失敗コードの区別。

#### UI 側

Settings の Automation セクションと `useAutomation` は変更しない（Non-goals）。`diagnose_all_cmd` の対象 directory 引数は省略可能であり、引数なしの既存 invoke は適用済み config directory を対象にし続ける。

### Algorithm

#### 終了コードの導出

report の `items` を走査し、severity が error の item が 1 件以上あれば `3`、それ以外（info のみ、および 0 件）は `0` とする（B-008、B-009）。

`workflow_summaries` / `facet_summaries` の `error_count` を合算しない。`add_diagnostic_to_workflow_and_facet`（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:2093-2104`）が 1 件の item を workflow 側と facet 側の両方の summary へ計上するため、summary の合算では二重計上になり件数が実体と一致しない。

#### 成功したまま非 zero で終える経路

現在の CLI は `Result<String, CliError>` を `cli_result_exit_code`（`src-tauri/src/cli/common.rs:3-29`）へ渡し、`Ok` は必ず `0` になる。「command は成功したが診断 error があった」を表現できないため、CLI の成功値を stdout と終了コードの組に変え、`cli_result_exit_code` の入力型をそれに合わせる。既存 subcommand は終了コード `0` を持つ成功値へ写像し、`1` / `2` / `4` の対応表は変えない。

診断 error の検出には未使用の `3` を割り当てる。`1`（`Other`）を再利用すると、I/O 失敗や serialize 失敗と「定義に error がある」が同じコードになり区別できなくなる。

#### `--dir` の解決と検証

CLI 入口で、`--dir` の値を process の cwd 基準で絶対 path へ解決し、存在しなければ `CliError::NotFound` として弾く。gateway は Directory target を診断する直前に `read_dir` を 1 回行い、directory の type と列挙可否を検証する。

- R-013 により指定 directory の診断は配下の workflow 定義を起点とするため、存在しない directory を渡すと起点が 0 件になり「診断対象なし・error 0 件」として終了コード `0` になる。typo と正常終了が区別できないので入口で弾く。`ensure_existing_data_dir`（`src-tauri/src/cli/common.rs:74-82`）が同じ理由で既に置かれており、その扱いに揃える。
- 絶対化は CLI 入口で行う。local API へは絶対 path だけを渡すため、診断対象が local API を提供するアプリ process の cwd に依存しない。
- gateway の `read_dir` が `NotFound` を返した場合は `WorkflowError::NotFound`、それ以外の I/O error（regular file の指定、権限不足など）は `WorkflowError::External` へ写す。CLI ではそれぞれ終了コード `4` と `1` になる。
- 入口検証は local API への到達より先に効く。アプリが起動しておらず、かつ `--dir` が存在しない場合の終了コードは `4` であって `1` ではない。R-012 が求めるのは「診断のために新しい失敗表現を作らないこと」であり、入力そのものが不正な場合まで未起動の失敗へ寄せることではない。到達可否より先に入力を弾くことで、アプリの起動状態によって同じ typo の観測結果が変わらない。

### Infra

該当なし。

## Alternatives Considered

- **port の signature を変えず、対象ごとに gateway を新規構築する**: local API handler と CLI がリクエストごとに `WorkflowDiagnosticsFileGateway` を組み立てることになり、composition root の責務（`docs/architecture/CONTROLLER.md`）が handler へ散る。加えて、適用済み config directory の path を CLI 側が再計算する必要が生じ、path の所有者が二重化する。採らない。
- **CLI を local API 経由にせず、常に in-process で診断する**: `src-tauri/src/cli/mod.rs:1-4` の「workflow command/query は local API を正とする」に反する。適用済み config directory を対象にする場合、その directory の所有者は起動中のアプリである。採らない。
- **local API を一次経路にしつつ read-only の file-direct fallback を併設する**: `cli/mod.rs:1-4` は read-only query への fallback を許可しているため実装は可能だが、Scope が local API 経由の対応だけに限定されている。fallback を持つと未起動時に診断結果が返り、R-012 の「既存 CLI command と同じ失敗」を満たせない。採らない。
- **usecase port の戻り値を typed な `DiagnosticReport` にする**: `DiagnosticReport` は adaptor 層の型であり、usecase 層の port から名指しできない。既存どおり `serde_json::Value` を保つ。
- **診断 error の検出に既存の終了コード `1` を再利用する**: command 自体の失敗と診断結果が同じコードになり、CI から両者を区別できない。採らない。

## Cross-cutting concerns

### 任意 directory を読む read endpoint を local API へ足すことの扱い

新設 endpoint は、呼び出し元が指定した directory 配下の workflow source と Facet を読み、その内容に由来する message を返す。到達できるのは master token を持つ呼び出し元だけであり、master token は renderer JS へ渡さない既存構成を変えない（terminal router だけが別 token を受理する、`src-tauri/src/adaptor/controller/api/mod.rs:45-56`）。master token を持つのは同一ユーザーの local process（CLI）であり、その process は元から同じ file 権限を持つため、権限の拡大は生じない。

### 検証手段が自明でない受入条件

- B-005（UI 経路と CLI 経路の一致）: 同一の一時 directory を対象 directory として UI 経路（`diagnose_all_cmd`）と CLI 経路の双方へ渡し、返る診断結果を比較する。両経路が対象 directory を引数で受けるため、内部 usecase を直接呼ばずに経路の入口から比較できる。
- B-008 / B-009（終了コード）: 終了コード導出を report 値からの純粋関数として分離し、error あり / info のみ / 0 件の 3 入力で確認する。
- local API 経由の経路（実 HTTP 通信）は `src-tauri/tests/` の統合テストで確認する（`docs/architecture/TEST.md` 統合テスト）。harness は `#[cfg(debug_assertions)]` の acceptance module に置き、`wiring::build_workflow_services_with_gateways` へ harness 用の gateway を渡して UI 経路の usecase を組む。test 専用の fake gateway を debug ビルドへ広げない。既存の `workflow_control_plane_acceptance` / `provider_lifecycle_acceptance` と同じ作法である。
- B-012（未起動時）: local API の discovery file が無い状態で診断 command を実行し、local API の起動を要する既存 CLI command と同じ失敗表示および同じ終了コードになることを確認する。

### 互換境界

- Tauri command（`diagnose_all_cmd`）の名前、引数、戻り値の JSON shape は変えない。
- 既存 local API endpoint と既存 CLI subcommand の入出力および終了コードは変えない。
- `WorkflowDiagnosticsGateway` は crate 内部の port であり、外部契約ではないため移行手段は要らない。

## Risks

- `DiagnosticReport.items` の並びは `load_all_workflows` の `read_dir` 列挙順に依存しており（`diagnostics.rs:1657`）、実装は順序を明示的に固定していない。B-005 は「diagnostic の並び順が一致する」を求めるが、順序を固定する変更は UI 経路の出力順も変えるため R-011 / B-011 と衝突しうる。同一 directory を同一 filesystem 状態で読む限り両経路の順序は一致するという前提で設計しており、この前提が成立しない場合は順序固定の可否を Requirements 側の判断として確認する必要がある。
