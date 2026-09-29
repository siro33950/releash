# Design

## 変える部分
- 配信する 8 種類の Output Data 化: `Failures`・`Terminal`・`Workflows`・`Workflow`・`NotionConfig`・`ProviderAvailability`・`WorkflowConfig`・`ProviderHookHealth` が運ぶ型を、domain の型を含まない usecase の Output Data にし、それを作る読み取りと presenter の変換を Output Data からの変換にする。根拠: R-001「購読で配信する状態は、domain の型を含まない Output Data だけでできている」、B-001、R-009、B-009。ルート: 委任
- `WorkflowConfig` の配信の変換: presenter が gateway の `workflow_to_model` と保存用の型 `WorkflowSection` を使わず、Output Data から転送の形に変える。根拠: R-002「presenter は gateway に依存しない」、B-002。ルート: 配信は Output Data から変換する
- `update_workflow_config` の入力: controller が gateway の保存用の型 `WorkflowSection` と `workflow_to_domain` を使わず、転送の形から Input Data に変えて Usecase に渡す。presenter の `wire::WorkflowSection` と gateway の `WorkflowSection` の間の変換（両方の向き）を無くす。根拠: R-002、R-003「`update_workflow_config` の入力は Input Data として Usecase に渡り、controller は gateway の保存用の型と変換を使わない」、B-003。ルート: 保存の入力は Input Data から変換し、controller は Input Data を Usecase に渡す
- `FactReadError` の対応付け: presenter の `impl ConnectFailure for FactReadError` を無くす。gateway が `FactReadError` を domain の失敗に変えてから外へ出し、gateway のテストもそれに合わせる。根拠: R-002、R-004「gateway の失敗は、gateway の中で domain の失敗に変えてから presenter に届く」、B-004。ルート: gateway が domain の失敗（`TechnicalFailure` 等。#1929 の規則）に変えてから外へ出し、presenter は domain の失敗だけを対応付ける
- provider の Output Data の一本化: usecase に provider を表す Output Data の型と `ProviderKind` からの写し方を 1 つずつ置き、`usecase/state_subscription/reads.rs`・`usecase/workflow/dto.rs`・`adaptor/presenter/agent_session.rs` の 2 か所・`adaptor/gateway/agent_session/agent_session_query_service.rs`・`agent_session_history_query_service.rs`、新しく作る `ProviderAvailability` と `ProviderHookHealth` の Output Data を、それに揃える。根拠: R-005「provider を表す Output Data と、domain の provider からそこへの写し方は、usecase にそれぞれ 1 つだけある」、B-005。ルート: usecase に型と写し方を 1 つずつ置き、列挙した箇所を全てそれに揃える
- provider の CLI を変える 3 つの操作の応答: `update_provider_executable`・`reset_provider_executable`・`refresh_provider_availability` の応答を、状態を含まない形にする。根拠: R-006「状態を返さない」、B-006。ルート: 委任
- provider の設定の画面の状態の出どころ: `useProviderAvailabilitySettings` が、3 つの操作の戻り値で表示の状態を置き換えるのをやめ、購読で受け取った状態だけを使う。reset で他の provider の保存していない入力を残すことと、refresh の失敗で直前の一覧を残してエラーを出すことは保つ。根拠: R-006「provider の設定の画面は、表示する状態を購読だけから受け取る」、R-007、R-008、B-006、B-007、B-008。ルート: 委任
- 使われなくなるコードの削除: 上の変更で使われなくなる型、関数、変換、proto のメッセージを消す。根拠: R-010、B-010。ルート: 委任

## 固定するルート
- 配信は Output Data から、保存の入力は Input Data から転送の形と相互に変換する。controller は gateway の保存用の型と変換を使わず、Input Data を Usecase に渡す。理由: 規約で adaptor が依存してよいのは usecase と domain だけであり、presenter と controller が gateway の保存用の型を経由すると、#1897 の「presenter が gateway に依存しない」を満たせない。
- gateway のエラー型は gateway の外へ出さない。gateway が domain の失敗（`TechnicalFailure` 等。#1929 の規則）に変えてから外へ出し、presenter は domain の失敗だけを対応付ける。gateway のテストもそれに合わせる。理由: 問題は、gateway のエラー型が presenter まで漏れていることにある。
- provider を表す Output Data の型と `ProviderKind` からの写し方は、usecase に 1 つずつ置く。一本化の対象は、Current Behavior の 6 か所、新しく作る 2 つの Output Data、gateway の 2 つの QueryService とする。理由: 正しい写し方を 1 つ足して既存を残すと、重複を増やすことになる。QueryService の戻り値も Output Data であり、この変更が扱う型と同じ種類である。

## 変えないもの
- provider を記録の key や保存の形へ文字列化する処理（`usecase/provider_lifecycle/hook_health.rs:276-281` の `provider_label`、`adaptor/gateway/local_event_store/*_codec.rs` ほか）。理由: 記録の key と保存の形は変わってはいけない識別子であり、画面へ出す形と連動させない。

## 未確定・リスク
なし
