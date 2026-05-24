# issues-1029: Workflow Engine Evolution - OutputForm CLI

関連: [GitHub Issue #1029](https://github.com/siro33950/releash/issues/1029) / マイルストーン [08] / [`workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md) / [`workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md) / 先行 [issues-1011](./issues-1011.md) / [issues-1013](./issues-1013.md) / [issues-1015](./issues-1015.md) / [issues-1019](./issues-1019.md) / [issues-1023](./issues-1023.md)

## 要求

**種別**: 新機能

**ゴール**: step agent が「自由文中に `<workflow_output>` ブロックを書く」という prose parsing 経由ではなく、`run_id` を主語とした typed CLI/API 経由で contract に従う structured output を engine に提出できるようにする。具体的には以下が満たされること。

- 利用者・Agent・外部 caller は `run_id` と `step_name` を主語に、step の output contract に従う JSON を CLI/API 経由で engine に提出できる。提出は file-direct 経路（[05]/[06] CLI と同じく、起動中の Releash app を介さず `workflow_runs/` 配下の run metadata / event log を直接読み書きする経路）で完結する。
- 提出経路として、以下の 3 CLI コマンドが存在する。各コマンドは対応する Tauri/local API 入口を engine 側に伴う。
  - `releash workflow output submit <run-id> --step <step-name> --type <contract> --json '{...}'`（`--json` の代替として `--file output.json` も受け付ける）
  - `releash workflow output validate <run-id> --step <step-name> --file output.json`
  - `releash workflow output get <run-id> --step <step-name>`
- `submit` は engine の既存 output contract validation（`workflow/contract.rs`）を再利用して入力 JSON を contract 適合か検査し、適合した場合のみ当該 step の `step_outputs` と `workflow_variables` を更新する。
- 適合した `submit` は append-only な `WorkflowEvent::OutputSubmitted`（または同義の typed event）を発行し、event log（NDJSON）と active state の双方に反映する。後続 step は従来通り `pass_output_from` で当該 output を参照できる。
- 不適合な入力（contract 違反・JSON parse 失敗・対象 step 不在・stale step 等）は決定論的な validation error として CLI 終了コード / API エラーで返り、`step_outputs` / `workflow_variables` / event log は副作用なしの状態に保たれる。`validate` は副作用なしで同一の validation 結果のみを返す。
- `get` は当該 step に対し既に提出済みの structured output（および提出時刻等の付随メタ情報）を返す。未提出の場合は決定論的に「未提出」を返す。
- 既存の `<workflow_output>` プロース抽出経路（step agent の自由文出力から contract を抽出して `step_outputs` を更新する経路）は本 issue で廃止する。CLI/API 経由の typed 提出が唯一の structured output 提出経路となる。
- 廃止に伴い、built-in workflow（`built-in/spec-driven-development.yml` および同梱されている他の built-in YAML）の step agent instruction を CLI 提出前提に書き換える。具体的には「step 完了時に `releash workflow output submit ...` を呼ぶことで contract を提出する」旨を step prompt / instruction 側で明示するように移行する。

**スコープ外**: 以下は本 issue の対象外。

- Workflow panel（[07]）への structured output 表示 UI・提出 UI の追加。submit 済 output を Workflow panel の step detail 上で structured に整形して見せる UI、Workflow panel から人間オペレータが output を submit する UI のいずれも本 issue では追加しない。step detail 上での output 表示は、既存の `WorkflowEvent` / `WorkflowState` 由来の素朴な表示（[07] で既に成立しているもの）の範囲に閉じる。
- Agent 向け Skill / Instructions file の追加（[15]）。step agent に「CLI をいつ・どう呼ぶか」を教える skill 文書化は本 issue では行わない。本 issue は built-in workflow の step prompt 内に必要な指示を埋め込む範囲に閉じる。
- bash node 実行系統（[13]）。
- Workflow template marketplace / external triggers / Web UI 移植（plan doc「採用しないもの」）。
- main-agent narrator への typed event 配信と user-facing report の整備（[16]）。`OutputSubmitted` event を main agent narrator に配信する経路は [16] の責務範囲に閉じる。
- Workflow run の起動 / approve / reject / abort CLI（[05]/[06] で既に確立済み）。本 issue はそれら CLI と並列に `workflow output` サブコマンド群を追加するに留まる。
- Remote セッション側（`src/remote/`）への structured output 提出 UI 提供。CLI/API 経路として engine 側に typed 入口を生やすため、Remote から将来この入口を利用することは妨げないが、本 issue で Remote 専用 UI を追加することはしない。

**現状温存**:

- 既存の typed command boundary（[04]/[06]）と `WorkflowEngine::dispatch_external` 単一入口は破壊しない。`OutputSubmitted` 系統の typed 入口も同じ `dispatch_external` 経由で engine に到達する。
- 既存の run_id 主語 read-only API / mutating API（[05]/[06]）と既存 UI command（[07]）はそのまま動き続ける。本 issue は `workflow_runs/` 配下の永続化形式に互換性を保ったまま `step_outputs` / event log を更新する。
- 既存の `step_outputs` を後続 step が `pass_output_from` で参照する semantics（[02] 以降で確立済み）は破壊しない。CLI 経由で更新された `step_outputs` は、従来 prose 抽出経由で更新されていた `step_outputs` と同一の slot に書き込まれ、後続 step から経路非依存に参照できる。
- 既存 output contract 定義（`workflow/contract.rs` 配下の contract 種別・facet 解決経路）は破壊しない。本 issue は新規の contract type を追加せず、既存 contract validation ロジックを CLI 入口から再利用する。
- 既存の Workflow panel（[07]）の表示構造・既存の自由対話 chat tab・既存の右パネル各モードは破壊しない。本 issue は engine 側の typed 提出経路と CLI 入口の追加・既存 prose 抽出経路の廃止・built-in workflow の step prompt 書き換えに限定する。

**背景**: マイルストーン [02]–[07] により、workflow engine は `run_id` を主語に state を観測・変化させる典型操作（list / status / logs / approve / reject / abort）を typed command boundary 経由で扱えるようになり、UI 側にも `run_id` 主語の第一級観測 surface（Workflow panel）が成立した。

一方、step agent から engine への「structured output の受け渡し」だけは依然として step agent の自由文出力に `<workflow_output>` ブロックを書かせ、engine がそれを prose parsing して `step_outputs` を更新する経路に閉じている。これは以下の問題を持つ。

- step agent の自由文出力が contract に適合しない（block 開始タグの欠落・JSON 構文崩れ・余計な markdown 装飾の混入等）と、engine 側は「無いものとして扱う / repair prompt を出す」しかなく、決定論的な validation gate にならない。
- agent が出力を contract 形式で書けるかは prompt 設計と LLM 出力の安定性に依存し、Agent / 外部 caller / 人間オペレータが「engine に明示的に contract を提出する」入口を持たない。
- step agent 以外の caller（外部スクリプト・人間オペレータ・将来の bash node など）から contract を engine に提出する経路が存在せず、step output が常に「agent の自由文」起点でしか更新されない。
- plan doc が完了条件として置く「step agent が prose parsing に頼らず structured output を提出できる」「invalid output が決定論的な validation error になる」を満たすには、agent-to-engine の structured output 経路を typed CLI/API として一段抽象化する必要がある。

本 issue は、`run_id` を主語に typed structured output を engine に提出する CLI/API を追加し、既存 prose 抽出経路を廃止して CLI 経路を唯一の structured output 提出経路に揃える。これにより、後続マイルストーン（[13] bash gate / [15] Skill / [16] Main Agent Mediation）が「agent / 外部 caller / 人間オペレータが engine に対し決定論的に contract を提出できる」を前提に組み立て可能になる。

**前提**:

- マイルストーン [02] Normalized Workflow 完了済み。`Workflow` / `NodeDefinition` / `WorkflowEvent` の新 schema が成立し、`workflow/contract.rs` 配下の output contract validation が新 schema 前提で動作する。
- マイルストーン [03] Run Store / Run ID 完了済み（[issues-1011](./issues-1011.md)）。`run_id` を一次キーとする run metadata と event log の永続化基盤が `workflow_runs/{run_id}/` に確立されており、CLI から file-direct で run metadata / event log を読み書きできる。
- マイルストーン [04] Command / Event Boundary 完了済み（[issues-1013](./issues-1013.md)）。`WorkflowCommand` typed 入口と `WorkflowEvent` 語彙、`WorkflowEngine::dispatch_external` 単一入口、認可・冪等性・stale target 判定が engine 内に閉じている。`OutputSubmitted` 系統の typed event を [04] で確立した event 語彙に追加する形で受け入れられる。
- マイルストーン [05] Read-Only Run APIs + CLI 完了済み（[issues-1015](./issues-1015.md)）。CLI が file-direct で `workflow_runs/` を読む基盤と Tauri/local API による run 観測経路が既に存在し、`workflow output get` / `workflow output validate` の実装基盤として再利用できる。
- マイルストーン [06] Mutating CLI 完了済み（[issues-1019](./issues-1019.md)）。CLI から file-direct で run 状態を変化させる経路と、CLI 経路・UI 経路が `dispatch_external` で合流する境界が確立されており、`workflow output submit` も同じ file-direct + typed command 合流のパターンを踏襲できる。
- マイルストーン [07] Workflow Panel / Command Center 完了済み（[issues-1023](./issues-1023.md)）。Workflow panel が `WorkflowEvent` 列を一次観測 surface としているため、本 issue で追加する `OutputSubmitted` event は追加実装なしに timeline 上の事実として観測できる。

## 振る舞い定義

```gherkin
Feature: Workflow step への構造化出力の提出

  Rule: 提出された構造化出力は contract に適合する場合のみ step output として確定する
    Scenario: contract に適合する構造化出力が提出される
      Given 進行中の workflow run に output contract を持つ step が存在する
      When 提出者が当該 step に対し contract に適合する構造化出力を提出する
      Then 当該 step の構造化出力として確定する
      And 後続 step から参照可能な状態になる
      And 当該提出が run の事実履歴に記録される

    Scenario: contract に適合しない構造化出力が提出される
      Given 進行中の workflow run に output contract を持つ step が存在する
      When 提出者が当該 step に対し contract に適合しない構造化出力を提出する
      Then 提出は拒否されたことが提出者に伝わる
      And 当該 step の構造化出力は提出前の状態のまま保たれる
      And 当該拒否は run のメイン履歴（OutputSubmitted）に残らない
      And 当該拒否は observability 用の補助履歴として記録される

    Scenario: 提出対象として妥当でない step に対し提出する
      Given workflow run が存在する
      And 提出対象として指定された step が当該 run に存在しない、あるいは既に出力を受け付けられる状態にない
      When 提出者が当該 step に対し構造化出力を提出する
      Then 提出は拒否されたことが提出者に伝わる
      And 当該 run の状態は提出前のまま保たれる
      And 当該拒否は observability 用の補助履歴として記録される

  Rule: 提出前に副作用なしで適合性を確認できる
    Scenario: 構造化出力の適合性のみを確認する
      Given 進行中の workflow run に output contract を持つ step が存在する
      When 提出者が当該 step に対し構造化出力の適合性確認のみを要求する
      Then 適合性の判定結果が提出者に伝わる
      And 当該 step の構造化出力および run の事実履歴は要求前の状態のまま保たれる

  Rule: 提出済みの構造化出力は経路非依存に参照できる
    Scenario: 提出済みの構造化出力を参照する
      Given workflow run の step に構造化出力が提出済みである
      When 参照者が当該 step の構造化出力を要求する
      Then 提出済みの構造化出力と提出に伴う付随情報が参照者に伝わる

    Scenario: 未提出の step の構造化出力を参照する
      Given workflow run に output contract を持つ step が存在する
      And 当該 step にはまだ構造化出力が提出されていない
      When 参照者が当該 step の構造化出力を要求する
      Then 未提出であることが参照者に伝わる

    Scenario: workflow に存在しない step の構造化出力を参照する
      Given workflow run が存在する
      And 参照者が指定した step が当該 workflow に存在しない
      When 参照者が当該 step の構造化出力を要求する
      Then 参照は拒否されたことが参照者に伝わる
      And 当該拒否は適合性確認（validate）の不在 step に対する判定と同じ扱いになる

    Scenario: 提出済みの構造化出力を後続 step が参照する
      Given workflow run の先行 step に構造化出力が提出済みである
      And 後続 step が当該 step の出力を入力として参照するよう定義されている
      When 後続 step が起動する
      Then 後続 step は提出済みの構造化出力を入力として受け取る

  Rule: 構造化出力の確定経路は明示的提出のみに統一される
    Scenario: step agent の自由文出力に構造化出力相当の表現が含まれる
      Given 進行中の workflow run に output contract を持つ step が存在する
      When 当該 step の agent が自由文出力の中に構造化出力相当の表現を含める
      But 明示的な提出は行わない
      Then 当該 step の構造化出力は未提出のまま保たれる
      And 後続 step からも未提出として扱われる

  Rule: CLI 入口は対象 run の実在を提出前に検証する
    Scenario: 存在しない run_id に対し mutation を要求する
      Given workflow run の観測経路に対象 run_id が存在しない
      When 提出者が当該 run_id に対し承認・却下・中止・構造化出力提出のいずれかを要求する
      Then 提出は CLI 入口で拒否されたことが提出者に伝わる
      And 受理キューには当該要求が投入されない

    Scenario: 観測経路の向き先が存在しない
      Given 観測経路として指定されたデータ領域そのものが存在しない
      When 提出者が観測経路を介して run 一覧・状態・履歴を要求する
      Then 観測は拒否されたことが提出者に伝わる
      And 「観測対象が 0 件」と区別される

  Rule: engine が判断した拒否は観測可能な補助履歴として記録される
    Scenario: 拒否規則を持たない承認待ち step に対し却下する
      Given 進行中の workflow run の承認待ち step に対する却下規則が定義されていない
      When 提出者が当該 step に対し却下を要求する
      Then 当該 step の状態は承認待ちのまま保たれる
      And 当該却下要求が受理された事実は run の事実履歴に記録される
      And 当該要求が engine によって拒否された事実が補助履歴として記録される

    Scenario: 承認規則と整合しない理由で却下要求が engine に拒否される
      Given 進行中の workflow run の承認待ち step に対し engine の認可・状態判定が承認外と判断する条件が成立している
      When 提出者が当該 step に対し却下を要求する
      Then 当該要求が engine によって拒否された事実が補助履歴として記録される
      And 補助履歴には拒否理由の分類が含まれる
```

## アーキテクチャ概要

### 責務配置

- **CLI 層（`src-tauri/src/cli/`）**:
  - 担当: `releash workflow output submit / validate / get` 3 サブコマンドの引数解析、`--json` と `--file` の受付、`submit` の pending command file 書き出し（[06] と同じ file-direct 経路）、`get` の `workflow_runs/` / `workflow_logs/` file-direct 読み出し（[05] 基盤の再利用）、`validate` の contract validator 呼び出し
  - 担当しない: state の直接更新、`step_outputs` への書き込み、`WorkflowEvent` の append、contract validation ロジック本体の実装
- **Tauri command 層（`src-tauri/src/workflow/commands.rs`）**:
  - 担当: `submit` / `validate` / `get` の local API 入口提供（in-process 経路として CLI と並列に存在し、UI/Remote/外部 caller から呼べる）、`engine.dispatch_external` への集約
  - 担当しない: pending file 経路（in-process 経路は pending を経由しない）、state mutation 本体
- **Workflow engine（`src-tauri/src/workflow/engine.rs`）**:
  - 担当: `WorkflowCommand::SubmitOutput`（新規 variant）を `dispatch_external` 単一入口で受理し、contract 適合判定 → `step_outputs` / `workflow_variables` 更新 → `WorkflowEvent::OutputSubmitted` append を 1 つの不可分なトランザクションとして実行、stale target / 不在 step / 認可違反の決定論的拒否、CLI 経路と in-process 経路の合流
  - 担当しない: agent 自由文からの structured output 抽出（本 issue で除去）
- **Contract module（`src-tauri/src/workflow/contract.rs`）**:
  - 担当: 既存 output contract 適合判定を pure validator として engine と `validate` CLI の双方から再利用される形で提供
  - 担当しない: prose 抽出（`<workflow_output>` block を agent 自由文から取り出すヘルパは本 issue で除去）、state への副作用
- **Event log / Run store（`src-tauri/src/workflow/event.rs`, `log.rs`, `run.rs`, `state.rs`）**:
  - 担当: `WorkflowEvent::OutputSubmitted` variant の追加、`WorkflowEvent::CliMutationRejected` variant の追加（観測経路用の補助履歴）、append-only NDJSON への書き込み、`step_outputs` slot の永続化（書き込み入口は engine 経由のみ）
- **Pending command queue（`src-tauri/src/workflow/pending_command.rs`, `pending_command_dispatcher.rs`）**:
  - 担当: `SubmitOutput` を `PendingCommandPayload` の新 variant として受け入れ、`workflow_pending/{pending,processing,processed}/` 経路で engine に取り次ぐ
- **Built-in workflow YAML（`workflows/built-in/spec-driven-development.yml` および同梱の他 built-in YAML）**:
  - 担当: step agent prompt に「step 完了時に `releash workflow output submit ...` 経由で contract を提出する」旨を明示
  - 担当しない: 自由文中に `<workflow_output>` を書かせる旧来の指示（本 issue で除去）

### データ/通信フロー

- **submit（CLI 経路）**: CLI が引数解析 → `PendingCommandPayload::SubmitOutput` を `workflow_pending/pending/` に書き出し → engine 側 pending dispatcher が pickup → `WorkflowCommand::SubmitOutput` に変換 → `engine.dispatch_external` → contract validate → 適合時のみ `step_outputs` / `workflow_variables` 更新 + `OutputSubmitted` event append（不適合時は副作用なしで決定論的エラー、event も残さない）→ CLI に outcome を返す
- **submit（in-process / Tauri command 経路）**: caller → `#[tauri::command]` → 同じ `engine.dispatch_external` 入口 → 上と同一の engine 処理に合流
- **validate**: CLI/API → `contract.rs` の pure validator を直接呼び出し → 判定結果のみ返却（pending file・event log・state のいずれにも触れない）
- **get**: CLI/API → `workflow_runs/{run_id}.json` と `workflow_logs/{run_id}.ndjson` を file-direct で読み、`step_outputs` slot を参照 → 提出済みなら構造化出力と付随メタを返却、未提出なら決定論的に「未提出」を返す
- **後続 step による参照**: engine が `pass_output_from` を解決する際に `step_outputs` slot を経路非依存に読む（CLI 経由でも in-process 経由でも同一 slot に書かれているため後続 step 側は提出経路を区別しない）

### 状態 Owner

- **`step_outputs` / `workflow_variables`**: workflow engine（`workflow/state.rs::WorkflowState` 上）が単一 owner。書き込み入口は `engine.dispatch_external` のみ
- **`WorkflowEvent` 列（NDJSON）**: `workflow/log.rs` が永続化 owner。append は engine 経由のみ
- **`WorkflowRun` metadata**: `workflow/run.rs` の `RunStore` が owner
- **Pending command queue（`workflow_pending/`）**: ファイルシステムが owner。CLI が producer、engine 側 pending dispatcher が consumer
- **CLI 入力（コマンド引数・`--json` / `--file` 内容）**: CLI プロセス内で完結。engine に届くのは `PendingCommandPayload` にシリアライズされた構造化入力のみ
- **Contract 定義**: `workflow/contract.rs` が静的に保持。本 issue では新規 contract 種別を追加しない

### 境界

- **CLI 層は engine state を直接読み書きしない**: `submit` は必ず pending file 経由、`get` は read-only file-direct のみ、`validate` は副作用のない pure validator 呼び出しのみ
- **Contract validation は engine と CLI（`validate`）の双方から再利用される pure 関数**: state mutation を伴わず、入力に対する判定だけを返す
- **Prose 抽出経路は engine 側から完全に除去**: step agent の自由文に含まれる `<workflow_output>` 相当の表現を engine は一切観測しない（振る舞い定義の「明示的提出は行わない」シナリオに対応）
- **OutputSubmitted の append は適合判定および state 更新と同一トランザクション境界内**: validation 失敗時はメイン履歴（OutputSubmitted）を残さず `step_outputs` / `workflow_variables` も不変、成功時は 3 者が原子的に揃う
- **CliMutationRejected はメイン履歴と独立した観測経路用補助履歴**: engine が CLI mutation（承認・却下・中止・構造化出力提出）を拒否した事実を observability 用途で記録する。accepted のメイン履歴（`OutputSubmitted` / `CliMutationRequested` / `ApprovalResolved` 等）に出ない条件と並列に発火し、CLI ユーザに「engine が無効と判断した」事実と理由分類を提供する
- **CLI 経路と in-process 経路は `dispatch_external` で合流**: 入口層に依らず同一の engine 処理を経るため、提出経路ごとの分岐は engine の責務範囲に閉じる
- **Workflow panel（[07]）への追加実装は本 issue の境界外**: `OutputSubmitted` event は既存 timeline 表示の中で素朴に観測されるに留め、structured output 表示専用 UI は追加しない

### 実装に委ねること

- CLI サブコマンドのパーサ構成詳細（clap の sub-builder 分割、`--json` と `--file` の相互排他の表現方法）
- `PendingCommandPayload` における `SubmitOutput` variant の field 名・シリアライズ表現
- engine 内 `SubmitOutput` handler の関数分割と helper 命名
- 不適合・stale・不在 step などのエラー種別に対する CLI exit code および API error code の具体値
- CLI 入口で run_id / data_dir / step の実在チェックを行うタイミング・順序・エラーメッセージ文面（CLI 入口バリデーション境界）
- `CliMutationRejected` event の拒否理由分類の追加・粗粒度設計（`run_not_found` / `no_reject_rule` / `step_not_accepting` / `contract_mismatch` / 他）
- `get` の返却ペイロードに含める付随メタ（提出時刻・event index 等）の項目選定と命名
- Built-in workflow YAML 内の step prompt 文面（CLI 提出を指示する具体的な日本語/英語表現）
- テストケースの具体的配置（contract 適合判定 reuse は `contract.rs` 内、submit 一貫トランザクションは `engine.rs` 内、CLI 経路は `cli` モジュール内、pending dispatcher との結合は `pending_command_dispatcher.rs` 内など）と各テストのデータ準備手順
- prose 抽出経路除去に伴う関連 dead code（旧 helper・旧 prompt 文字列など）の物理的な削除範囲

