# Context

- 要求の正本は GitHub Issue #1490「CLIからUIと同一のworkflow diagnosticsを実行できるようにする」（https://github.com/siro33950/releash/issues/1490 、state: OPEN、label なし、milestone なし、comment なし）。Issue 本文以外の URL、添付、関連 Issue の参照はない。追加の自由文指示もない。
- Spec の配置先は `docs/specs/issues-1490`。
- workflow 診断の実体は `diagnostics::diagnose_all(workflows_dir, facets_base_dir)`（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:1442`）である。workflow source の parse と shape 検査、`ValidationError` 由来の resolve / typecheck / control-flow 検査、Facet 解決、facet 本文の template 検査、diagnostic code への mapping を、この一箇所がまとめて行う。
- 診断結果 DTO は `DiagnosticReport`（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:118-126`）で、`items` / `workflow_summaries` / `facet_summaries` / `facet_usage` を持つ。`DiagnosticSummary` は同 `:112-115`、`FacetUsageEntry` は同 `:129-133`。`DiagnosticItem`（同 `:34-56`）は `code`、`severity`（同 `:28-31` の `error` / `info`）、`stage`（同 `:60-64`）、`span`（`src-tauri/src/adaptor/gateway/workflow/span_map.rs:7-14` の `source` / `start_line` / `start_col` / `end_line` / `end_col`）、`message`、`workflow_name`、`node_name`、`facet_key`、`facet_kind`、`field` を持つ。frontend は同じ形を `DiagnosticReport`（`src/types/workflow.ts:278-283`）として受ける。
- `AGENTS.md` の「アーキテクチャ原則」により、アプリケーションロジックは Rust が所有する。入口は Tauri command（`adaptor/controller/command/`）、loopback HTTP local API（`adaptor/controller/api/`）、CLI（`cli/`）の 3 つで、同じ usecase を共有する。
- `src-tauri/src/cli/mod.rs:1-4` は CLI の経路規約として「workflow command/query は localhost local API を正とする。アプリ未起動時は read-only query だけ backend-owned read model の file-direct fallback を許可する」と定めている。fallback 実装は `src-tauri/src/cli/file_direct.rs`。
- `docs/glossary/DOMAIN.md:144-146` は Diagnostic を「WorkflowDefinition の parse、shape、resolve、typecheck、control-flow の検証結果であり、実行木や NodeExecution の lifecycle state ではない」と定めている。
- 現在状態の確認は、worktree 内コードの読解と `releash workflow --help` の実行によって行った。Releash アプリを起動しての UI 動作確認は行っていない。UI 経路の出力形は、`diagnose_all_cmd` が素通しする `diagnostics::diagnose_all` の既存テストで確認した。

# Outcome

対象者は、repository 内で workflow 定義（`workflows/*.yml` と `workflows/facets/{instructions,policies,knowledge}/*.md`）を書く開発者、および同じ定義を扱う agent である。

現在、workflow 定義の正式な診断は Releash の UI からしか実行できない。診断対象は適用済みの config directory に固定されているため、repository 内で書いた custom workflow を検証するには、いったん config directory へ適用したうえでアプリを起動し、Settings を開く必要がある。適用前に手元で行えるのは `yq` などによる YAML 構文 parse までで、WFS002 のような workflow shape error、schema 宣言と参照の不整合、Facet 参照の欠落、template 変数と input 宣言の不一致は検出できない。

変更後は、CLI から診断対象 directory を指定して、UI と同一の診断を実行できる。同じ入力に対して UI と CLI は同じ diagnostic code、severity、message、location、順序を返し、CLI は診断 error の有無を終了コードで表す。開発者と agent は、config directory へ適用する前に、repository 内の workflow directory をそのまま正式検証できる。

CLI の診断は Releash アプリが公開する local API を経由して実行するため、アプリの起動を要する。アプリが起動していない場合の観測結果は、local API の起動を要する既存 CLI command と同じにする。

# Current Behavior

## UI 経路

Settings の Automation セクションが `useAutomation` を通じて `diagnose_all_cmd` を invoke する（`src/hooks/useAutomation.ts:58`、`:82`、表示は `src/components/panels/AutomationSection.tsx`）。呼び出しの連鎖は次のとおりである。

1. `diagnose_all_cmd`（`src-tauri/src/adaptor/controller/command/workflow/diagnostics.rs:5-14`、登録は `src-tauri/src/adaptor/controller/command/workflow/mod.rs:52,98`）が `spawn_blocking` 内で
2. `WorkflowUsecase::diagnose_all`（`src-tauri/src/usecase/workflow/mod.rs:461-463`）を呼び、
3. `WorkflowDiagnosticsGateway::diagnose_all`（`src-tauri/src/usecase/workflow/ports.rs:83-85`）へ委譲し、
4. `WorkflowDiagnosticsFileGateway`（`src-tauri/src/adaptor/gateway/workflow/diagnostics_gateway.rs:9-33`）が `diagnostics::diagnose_all` を呼び、`DiagnosticReport` を `serde_json::to_value` で serialize して返す。

usecase の戻り値は `serde_json::Value` であり、DTO の serialize は gateway 境界で完了している。Tauri command はその値を素通しする。

## 診断対象 directory が config directory に固定されている

`WorkflowDiagnosticsFileGateway` は `workflows_dir` と `facets_base_dir` を construction 時に受け取り（`src-tauri/src/adaptor/gateway/workflow/diagnostics_gateway.rs:14-23`）、`diagnose_all` は引数を取らない（`src-tauri/src/usecase/workflow/ports.rs:84`）。composition は両方に `WorkflowDefinitionFileRepository::default_workflows_dir()` を渡している（`src-tauri/src/adaptor/controller/wiring.rs:334-335,358-361`）。その値は `storage::workflows_dir()`（`src-tauri/src/adaptor/gateway/workflow/storage.rs:108-113`）が返す `<config_dir>/releash/workflows` である。

診断対象を実行時に切り替える入口はない。repository 内の `workflows/` を対象に診断する経路も存在しない。

## 診断内容

`diagnose_all`（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:1442-1551`）は次を行う。

- `facets_base_dir` から `policies` / `knowledge` / `instructions` の全 Facet key を収集する。
- `workflows_dir` 上の workflow source を全件走査して診断し、ディスク上の定義と名前が衝突しない builtin workflow を追加で診断対象にする（`load_all_workflows`、同 `:1649-1726`）。
- 各 Facet について key 命名規則（FAC001）、builtin である旨の info（FAC000）、template 変数、workflow 側 input 宣言との突合を検査する。

diagnostic code は parse / shape 系（WFS001、WFS002、WFS006、WFS008 など）、resolve 系（WFR001 など）、typecheck 系（WFT001 など）、control-flow 系（WFC004 など）、Facet 系（FAC000、FAC001）に分かれる。`ValidationError` から code への mapping も同ファイルが持つ（同 `:1180-1240`）。schema 宣言の不正は `InvalidSchemaKind::InvalidDeclaration` を経由して WFS002 になる（同 `:1218`、判定は `src-tauri/src/domain/workflow/services/validation.rs:1370-1470`）。

## CLI に診断入口がない

CLI の top command は Workflow / Review / Hook の 3 つで（`src-tauri/src/cli/mod.rs:32-49`）、`releash workflow` の subcommand は `status` と `output`（`submit` / `get`）だけである（`src-tauri/src/cli/workflow.rs:12-25`、dispatch は `src-tauri/src/cli/mod.rs:120-145`）。`src-tauri/src/cli/` 配下に diagnostics を呼ぶコードはない。

Issue 本文は既存の確認手段として `releash workflow list` を挙げているが、`workflow list` subcommand は現在の CLI に存在しない。

## local API に診断 endpoint がない

local API の workflow router（`src-tauri/src/adaptor/controller/api/workflow.rs:50-93`）が公開しているのは、workflow 一覧、execution の一覧・作成・取得・log・approve・abort・stop・resume・retry、node execution の submit、artifact の validate・get である。diagnostics の endpoint はない。

## CLI の終了コード規則

`cli_result_exit_code`（`src-tauri/src/cli/common.rs:3-29`）は、成功で 0、`CliError::InvalidInput` で 2、`CliError::NotFound` で 4、`CliError::Other` で 1 を返す。処理そのものは成功したうえで、結果の内容（診断 error の有無など）に応じて終了コードを変える経路はない。

## 再現手順

1. repository 内の `workflows/` に、schema 宣言が不正な workflow 定義（例: `properties` 配下に不正な宣言を置いたもの）を追加する。
2. `releash workflow --help` を実行する。実際の出力は次のとおりで、診断を実行する subcommand はない。

```
workflow command / query サブコマンド。

Usage: releash workflow <COMMAND>

Commands:
  status  指定 execution の現在 read model を表示する。
  output  node の Artifact に対する typed CLI 入口。

Options:
  -h, --help  Print help
```

3. そのため手元で行えるのは `yq` などによる YAML 構文 parse までで、WFS002 は検出されない。
4. 同じ定義を `<config_dir>/releash/workflows` へ配置し、Releash を起動して Settings の Automation を開くと、`diagnose_all_cmd` の結果として WFS002 が表示される。`diagnose_all_cmd` は `diagnostics::diagnose_all` の結果を serialize して素通しするため、この項目に付く値は既存テスト `test_診断_yamlとluaの不正permissionは同じwfs002でfield位置を示す`（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:2304-2332`）で確認できる。同テストは不正な宣言に対し `code = "WFS002"`、`stage = ParseShape`、`field = Some("permission")`、`span.start_line = 7` を assert している。

# Scope / Non-goals

## Scope

- CLI から workflow diagnostics を実行する入口の追加。
- local API への workflow diagnostics endpoint の追加。CLI はこの endpoint を経由して診断する。
- 診断対象 directory を実行時に指定できるようにすること（workflow source と Facet base の双方）。
- CLI の診断結果出力（human-readable および JSON）と終了コード。
- CLI の `--help` 記載内容。
- UI と CLI が同じ usecase、同じ `WorkflowDiagnosticsGateway`、同じ診断実装を共有する構成。

## Non-goals

- workflow diagnostics の既存規則、diagnostic code、severity、message、span の変更。
- UI 専用または CLI 専用の診断規則の追加。
- CLI の file-direct fallback（`src-tauri/src/cli/file_direct.rs`）へ診断を追加すること。診断はアプリ起動中の local API 経由だけで実行する。
- CLI による診断結果の自動修正。
- UI 側の診断表示（`AutomationSection` / `SettingsModal`）の変更。
- diagnostics 以外の CLI subcommand の変更。
- workflow 定義構文そのものの変更。
- 診断以外の workflow 操作（適用、実行、保存）を CLI へ追加すること。

# Requirements

- R-001: CLI から workflow diagnostics を実行できる command を提供する。
- R-002: UI 経路と CLI 経路の双方が、診断対象として、適用済み Workflow の config directory と、利用者が指定した directory の双方を扱える。指定できる directory には repository 内の custom workflow directory を含む。
- R-003: 指定した directory を workflow source directory として扱う。Facet base は `<dir>/facets` が directory として存在すればそこを使い、存在しなければ `<dir>` を使う。
- R-004: 指定した directory 配下の Facet を含めて診断する。対象 workflow が config directory へ適用されていない状態でも診断できる。
- R-005: 同一の対象 directory に対して、CLI が返す診断結果は UI が受け取る `DiagnosticReport` と、diagnostic code、severity、message、location（span）、`items` の順序、workflow summary、facet summary、facet usage のすべてにおいて一致する。
- R-006: UI と CLI は同じ Rust usecase と同じ `WorkflowDiagnosticsGateway` を経由して診断を実行する。parse、schema 検証、routing 検証、Facet 解決、template 検証、diagnostic mapping を CLI 側へ再実装しない。CLI 経路に診断規則の分岐や複製を持たない。
- R-007: JSON 出力を選んだ場合、CLI は UI が受け取るものと同じ `DiagnosticReport` DTO をそのまま serialize した JSON を出力する。CLI 固有の field 追加や再構成を行わない。
- R-008: JSON 出力を選ばない場合、CLI は human-readable な形式で診断結果を出力する。
- R-009: 診断を実行できたとき、severity が `error` の diagnostic が 1 件以上あれば CLI process は non-zero の終了コードで終了し、`error` が 0 件であれば 0 で終了する。
- R-010: CLI の `--help` に、診断対象 directory の指定方法、終了コードの意味、出力形式を記載する。
- R-011: 既存の diagnostic 規則、diagnostic code、severity、message、および `DiagnosticReport` の wire shape を変更しない。UI 経路（`diagnose_all_cmd`）が返す結果は、本変更の前後で同一である。
- R-012: Releash アプリが起動しておらず local API へ到達できないとき、CLI の診断 command は診断結果を出力せず、local API の起動を要する既存 CLI command と同じ失敗表示および同じ終了コードで終了する。本変更で新しい失敗表現を追加しない。
- R-013: 指定した directory を対象にする場合、診断は指定 directory 配下の workflow 定義を起点とし、そこから実行時と同じ解決規則で到達する Facet を含めて判定する。起点から到達しない workflow および Facet について diagnostic を出さない。適用済み Workflow の config directory を対象にする場合の診断範囲は変更しない。

# Assumptions / Open Questions

## Assumptions

なし。

## Open Questions

なし。
