# Context

- 正本: [#1955 `[11] 購読の出力が domain の型と gateway に依存し、設定の状態の出どころが 2 つある`](https://github.com/siro33950/releash/issues/1955)
- 補助資料: #1897（通信の共通化の完了確認。「Output Data が domain の型を含まない。presenter が gateway に依存しない（#1955）」と「単発の呼び出しは状態を返さない」を確かめる）、#1898（設定を購読に移した）、#1931（状態の配信を Output Boundary と presenter に移した）、#1954（Output Boundary を一方向にした）、#1956（購読の読み取りの失敗の出し方）。
- 依存: #1898、#1931、#1954。いずれも merge 済みである。
- 基準は `main` の `c519ef9f`。Current Behavior はこの commit で確認した。
- 正本の「今の作り」が挙げる domain の型は 3 つだが、`c519ef9f` で確認すると、購読で配信する状態のうち 8 種類が domain の型を含む。正本が 3 つしか挙げていないのは調査の見落としであり、範囲を絞る判断ではない。規約の「Output Data は Entity を参照しない単純なデータ」は全種類に掛かる。
- 正本は presenter が gateway に依存する箇所として `state_subscription_wire.rs:180` だけを挙げるが、`c519ef9f` の本番コードでは 3 か所ある（Current Behavior に記す）。規約で adaptor が依存してよいのは usecase と domain だけである。
- 正本が #1954 に回した「失敗の presenter が gateway の `FailureRecordStore` を持つこと」と「状態の変化の知らせの経路」は、#1954 で扱われた。#1954 により失敗の記録の型（`FailureRecord` ほか）が domain へ移り、`usecase/failure.rs:4` は domain から `pub use` している。そのため、配信する `Failures` が domain の型を運んでいる。配信する型を Output Data にするのはこの変更の範囲である。
- 規約の正本は `docs/architecture/`。この変更が根拠にする規定は次のとおり。
  - `README.md`「部品の一覧」: 「usecase / Input Data / Output Data / 操作の入力と出力。Entity を参照しない単純なデータ。失敗も出力として表す」。「adaptor/presenter / Presenter、転送のメッセージ型 / Output Data を転送の形とステータスコードに変える」。「adaptor/controller / Controller / 外からのきっかけ（転送の要求、時刻・起動、OS の通知）を Input Data に変えて、Usecase を呼ぶ」。
  - `README.md`「依存方向」: 「adaptor（gateway / controller / presenter）は usecase と domain に依存してよい（依存は内向き）」。
  - `README.md`「横断的な設計原則」: 「同じ操作の実装は 1 つに集約する」。
  - `USECASE.md`「Input Data / Output Data」: 「表示・転送の都合で形が決まる。ドメイン（Entity）から導かれない」。
  - `USECASE.md`「QueryService」: 「trait を usecase に置き、adaptor/gateway が実装する。Output Data を返す」。
  - `PRESENTER.md`「原則」: 「Usecase が出した Output Data を、転送の形（Connect のメッセージ、HTTP local API のレスポンス）に変える」。
  - `PRESENTER.md`「失敗」: 「Usecase が出した失敗を、転送の失敗（Connect のステータスコード、HTTP status）へ対応付けるのは presenter の 1 か所だけである」。
- 同じ箇所に触れる ISSUE: #1956 は `usecase/state_subscription/reads.rs` と `usecase/workflow/mod.rs` を変える。後から merge する側が、先に merge した変更に合わせる。
- 参照する既存実装: `src-tauri/src/usecase/state_subscription/value.rs`・`reads.rs`、`src-tauri/src/adaptor/presenter/state_subscription_wire.rs`・`agent_session.rs`・`notion.rs`・`connect.rs`・`client/conversions.rs`・`terminal.rs`、`src-tauri/src/adaptor/gateway/app_config/config_models.rs`、`src-tauri/src/adaptor/gateway/workflow/fact_log.rs`、`src-tauri/src/adaptor/gateway/agent_session/agent_session_query_service.rs`・`agent_session_history_query_service.rs`、`src-tauri/src/adaptor/controller/client/app_config/commands.rs`、`src-tauri/src/adaptor/controller/client/agent_session/provider_tui.rs`・`shared.rs`、`src-tauri/src/usecase/agent_session/provider_availability.rs`、`src-tauri/src/usecase/failure.rs`、`src-tauri/src/usecase/terminal_surface/application.rs`、`src-tauri/src/usecase/workflow/dto.rs`、`src-tauri/src/usecase/provider_lifecycle/hook_health.rs`、`proto/client.proto`、`src/hooks/useProviderAvailabilitySettings.ts`、`src/components/panels/SettingsModal.test.tsx`。

# Outcome

対象者は、daemon と画面を実装・保守する開発者と、設定の画面で provider の CLI を設定する利用者である。

現在、購読で配信する状態の型（Output Data）の一部が domain の型を運んでいる。そのため、domain の型を変えると、配信の形と presenter が一緒に変わる。presenter は、転送の形に変えるときや失敗を対応付けるときに gateway の型と変換に依存しており、規約の依存方向に無い向きになっている。provider を表す Output Data は usecase に 2 つあり、domain の `ProviderKind` からの写し方が 6 か所に重複している。provider の設定の画面は、購読で受け取った状態と、単発の呼び出しの戻り値の両方で表示を置き換える。どちらが後に届くかで表示される状態が決まり、「単発の呼び出しは状態を返さない」を満たさない。

変更後は、次の状態になる。

- 購読で配信する状態は、Entity を参照しない Output Data だけでできている。
- presenter は、usecase と domain にだけ依存して、転送の形と失敗の対応付けを行う。
- provider を表す Output Data と、そこへの写し方は 1 つずつである。
- provider の設定の画面は、状態を購読だけから受け取る。provider の CLI を変える操作は、状態を返さない。

画面が購読で受け取る値と、provider の設定の画面での利用者から見える振る舞いは変わらない。

# Current Behavior

`c519ef9f` のコードで確認した挙動である。

## 配信する状態のうち 8 種類が domain の型を含む

`src-tauri/src/usecase/state_subscription/value.rs:18-56` の `StateValue` のうち、次の 8 種類が domain の型を含む。残る 27 種類は、std の型と usecase の型だけでできている。

| 配信する状態 | domain の型の位置 |
|---|---|
| `Failures(FailurePage)` | `FailurePage.items[].record: FailureRecord`（`usecase/failure.rs:48-56`）。`FailureRecord` は `domain::failure` の型で、`usecase/failure.rs:4` が `pub use` している |
| `Terminal(TerminalSurfaceStreamItem)` | `TerminalSurfaceStreamItem::Snapshot(TerminalSurface)`（`usecase/terminal_surface/application.rs:55-56`）。`TerminalSurface` は domain の Entity（`domain/terminal_surface/entities/terminal_surface.rs`） |
| `Workflows(Vec<WorkflowSummaryDto>)` | `WorkflowSummaryDto.source_format: domain::WorkflowSourceFormat`（`usecase/workflow/dto.rs:183`） |
| `Workflow(Option<WorkflowDto>)` | `WorkflowDto.source_format: domain::WorkflowSourceFormat`（`usecase/workflow/dto.rs:15`） |
| `NotionConfig(Option<NotionRepoConfig>)` | 運ぶ型そのものが `domain::app_config::value_objects::NotionRepoConfig`（`value.rs:48`） |
| `ProviderAvailability(ProviderRegistry)` | 運ぶ型そのものが domain の集約 `domain::agent_session::aggregates::ProviderRegistry`（`value.rs:49`） |
| `WorkflowConfig(WorkflowConfig)` | 運ぶ型そのものが `domain::app_config::value_objects::WorkflowConfig`（`value.rs:52`） |
| `ProviderHookHealth(Vec<ProviderHookHealthWarning>)` | `.provider: ProviderKind`、`.reason: ProviderLifecycleUnavailableReason`（`usecase/provider_lifecycle/hook_health.rs:9-13`） |

読み取りは `usecase/state_subscription/reads.rs` にある。`NotionConfig` は `:369-370` で `NotionUsecase::get_config`（`usecase/notion/usecase.rs:72-77`）を呼び、`ProviderAvailability` は `:372-373` で `ProviderAvailabilityUsecase::snapshot`（`usecase/agent_session/provider_availability.rs:51-56`）を呼び、`WorkflowConfig` は `:392-393` で `AppConfigUsecase::get_workflow_config`（`usecase/app_config/usecase.rs:39-41`）を呼ぶ。いずれも domain の型をそのまま返す。

presenter は、これらを domain の型から転送の形に変えている。

- `ProviderAvailability`: `adaptor/presenter/agent_session.rs:55-90` の `From<ProviderRegistry> for ProviderAvailabilitySnapshotResponse`。
- `NotionConfig`: `adaptor/presenter/notion.rs:195-203` の `From<NotionRepoConfig> for NotionRepoConfigView`。
- `ProviderHookHealth`: `adaptor/presenter/agent_session.rs:92-115`。
- `Failures`: `adaptor/presenter/state_subscription_wire.rs:10-36`。

## presenter が gateway に依存している箇所が 3 つある

テストを除く本番コードで、presenter が gateway を参照している箇所は次の 3 つである。

1. `adaptor/presenter/state_subscription_wire.rs:178-183`: `WorkflowConfig` を、gateway の `workflow_to_model`（`adaptor/gateway/app_config/config_models.rs:242-246`）で保存用の型 `WorkflowSection`（`config_models.rs:116-120`）に変えてから、転送の形にしている。
2. `adaptor/presenter/client/conversions.rs:3582-3604`: gateway の保存用の型 `WorkflowSection` と、転送の型 `wire::WorkflowSection` の間の変換である。向きは両方ある。
   - gateway → wire は、1 が使う。
   - wire → gateway は、`update_workflow_config` の controller（`adaptor/controller/client/app_config/commands.rs:4,57-68`）が使う。controller は、受け取った gateway の型を gateway の `workflow_to_domain` で domain の `WorkflowConfig` に変えて、Usecase に渡している。
3. `adaptor/presenter/connect.rs:590-597`: gateway のエラー `fact_log::FactReadError`（`adaptor/gateway/workflow/fact_log.rs:28-33`）を Connect のステータスコードに対応付けている。`FactReadError` は gateway の外に出ておらず、gateway の外で参照しているのはこの impl だけである。この impl を使っているのは、gateway のテスト（`adaptor/gateway/workflow/fact_log_test.rs:2495`、`adaptor/gateway/agent_session/session_facts_test.rs:4` ほか）である。

## provider を表す Output Data と写し方が重複している

usecase には、provider を表す Output Data が 2 つある。

- `AgentSessionProviderDto`（`usecase/agent_session/agent_session_query.rs`）
- `SessionProviderDto`（`usecase/workflow/dto.rs`）

domain の `ProviderKind`（`domain/provider_lifecycle/value_objects/provider_kind.rs:2-5`、`Claude`・`Codex`）を、provider を表す Output Data または転送の形に写す `match` は、次の 6 か所にある。

- `usecase/state_subscription/reads.rs:259-273`（`AgentSessionProviderDto` へ）
- `usecase/workflow/dto.rs:360-366`（`SessionProviderDto` へ）
- `adaptor/presenter/agent_session.rs:65-70`（文字列 `"claude"`・`"codex"` へ）
- `adaptor/presenter/agent_session.rs:100-106`（`ProviderHookHealthProviderResponse` へ）
- `adaptor/gateway/agent_session/agent_session_query_service.rs:165`（`AgentSessionProviderDto` へ）
- `adaptor/gateway/agent_session/agent_session_history_query_service.rs:137`（`AgentSessionProviderDto` へ）

## provider の設定の画面は、状態を 2 か所から受け取る

- `update_provider_executable`、`reset_provider_executable`、`refresh_provider_availability` の応答は、どれも状態全体 `ProviderAvailabilitySnapshotResponse` である（`proto/client.proto:283,292,317`）。
- 状態を変える他の操作（`update_workflow_config`・`update_external_editor` ほか）の応答は `Unit` である（`proto/client.proto:309-318`）。
- 3 つの操作の controller（`adaptor/controller/client/agent_session/provider_tui.rs:20-52`、`shared.rs:145,171,259`）は、Usecase が返す `ProviderRegistry` を応答に変えている。
- 3 つの操作を呼んでいるのは、provider の設定の画面だけである。CLI と HTTP local API からは呼ばれていない。
- Usecase の 3 つの操作は、どれも最後に購読へ状態の変化を知らせる（`usecase/agent_session/provider_availability.rs:113-123` の `rebuild_registry`）。
- 画面の `useProviderAvailabilitySettings` は、表示に使う `snapshot` を次の 2 か所で置き換える。どちらが後に届くかで、表示される状態が決まる。
  - 購読で受け取った状態: `src/hooks/useProviderAvailabilitySettings.ts:50-52,76-86`
  - 3 つの操作の戻り値: `:110-116`、`:131-141`、`:155`
- 画面の振る舞いのうち、次の 2 つを既存のテストが確かめている（`src/components/panels/SettingsModal.test.tsx`）。
  - 一方の provider を reset しても、もう一方の保存していない入力は消えない（`:562-599`）。今は、reset の戻り値を受けて入力を組み直している（`useProviderAvailabilitySettings.ts:131-141`）。
  - refresh が失敗したときは、直前の一覧を表示したまま、エラーを表示する（`:601-638`）。

# Scope / Non-goals

今回変更する対象。

- 購読で配信する状態のうち、domain の型を含む 8 種類（`Failures`・`Terminal`・`Workflows`・`Workflow`・`NotionConfig`・`ProviderAvailability`・`WorkflowConfig`・`ProviderHookHealth`）の型を、domain の型を含まない Output Data にすること。これらを作る読み取りと、転送の形への変換を含む。
- presenter から gateway への 3 つの依存を無くすこと。
  - `WorkflowConfig` の配信を Output Data から変換すること。
  - `update_workflow_config` の入力を、gateway の保存用の型ではなく Input Data にし、controller が Input Data を Usecase に渡すこと。
  - gateway のエラー `FactReadError` を gateway の中で domain の失敗に変えてから外へ出し、presenter が domain の失敗だけを対応付けること。gateway のテストもこれに合わせる。
- provider を表す Output Data と、`ProviderKind` からの写し方を usecase に 1 つずつにすること。上の 6 か所、新しく作る `ProviderAvailability` と `ProviderHookHealth` の Output Data、gateway の 2 つの QueryService を、それに揃える。
- `update_provider_executable`・`reset_provider_executable`・`refresh_provider_availability` が状態を返さないようにすること。provider の設定の画面が、状態を購読だけから受け取るようにすること。
- 上記に伴い使われなくなるコードの削除。

今回変更しない対象。

- 読み取りの失敗の出し方。#1956 が扱う。
- Notion の設定の保存の入力（`adaptor/presenter/notion.rs:205-213` が転送の型を domain の `NotionRepoConfig` に変えること、`usecase/notion/usecase.rs:62-66` の `save_config` が domain の型を受け取ること）。MS99 の目的（通信の一本化）にも、この変更の目的（配信の型と presenter の gateway 依存）にも入らず、層の責務の見直し（マイルストーン #97）の範囲であるため。
- 状態の変化の知らせの経路。#1954 で扱った。
- provider を記録の key や保存の形へ文字列化する処理（`usecase/provider_lifecycle/hook_health.rs:276-281` の `provider_label`、`adaptor/gateway/local_event_store/*_codec.rs` ほか）。これらは変わってはいけない識別子であり、画面へ出す形と連動させない。重複は Output Data とは別の問題として別途扱う。
- `docs/architecture/` の記述。

# Requirements

- R-001: 購読で配信する状態は、domain の型を含まない Output Data だけでできている。
- R-002: presenter は gateway に依存しない。presenter が転送の形とステータスコードに変えるのは、usecase と domain の型だけである。
- R-003: `update_workflow_config` の入力は Input Data として Usecase に渡り、controller は gateway の保存用の型と変換を使わない。
- R-004: gateway の失敗は、gateway の中で domain の失敗に変えてから presenter に届く。
- R-005: provider を表す Output Data と、domain の provider からそこへの写し方は、usecase にそれぞれ 1 つだけある。provider を Output Data として返す読み取りと配信は、すべてそれを使う。
- R-006: `update_provider_executable`・`reset_provider_executable`・`refresh_provider_availability` は、状態を返さない。provider の設定の画面は、表示する状態を購読だけから受け取る。
- R-007: provider の設定の画面で、一方の provider を reset しても、もう一方の保存していない入力は消えない。
- R-008: provider の設定の画面で、refresh が失敗したときは、直前の一覧を表示したまま、エラーを表示する。
- R-009: 購読で配信する状態のうち R-001 で変える 8 種類について、画面が受け取る値はこの変更の前と同じである。転送の形が変わるのは、R-006 の 3 つの操作の応答だけである。
- R-010: この変更で使われなくなるコードは残らない。

# Assumptions

人間が明示的に受け入れた仮定は無い。
