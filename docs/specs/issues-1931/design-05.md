# Design 05

## 開始状態

design-04 の実装を確定した `72271b0b`（`chore(development-v2): design-04 の実装を確定する`）を基準にする。直前の Design は `design-04.md`、差分の基準（base ブランチと派生点）は design-01 のまま `main` の `9aceb249` である。この周までに閉じた Thread は 57 件である。open Thread は 12 件あり、すべてに `[PLAN]` を付けた。うち 3 件（`a70df8f4`・`cdd5fcfb`・`b288c3a3`）は前の周の `[PLAN]` の条件を満たさず `[NOT_FIXED]` が付いたもので、`[PLAN]` を書き直した。

## 変える部分

- NUL を含む識別子での panic: 購読の対象の識別子を作る経路で panic させず、失敗として返す。根拠: Thread `8f9c8168-4b23-4da6-90e3-da5da2cfe8d9`。ルート: 委任
- 配信対象を有効にする順序: registry に surface を登録した後に配信対象を有効にする。根拠: Thread `6f2d8ad7-ce6d-4609-a57c-8a2affe9eb1f`。ルート: 委任
- terminal の開始と出力順序の区間: 区間に入れなかった場合は開始を成功させず、世代を取り直してやり直す。根拠: Thread `a70df8f4-5109-4426-87f3-0168842daace`。ルート: 委任
- 通知の規則の統一: `update` の経路も `mutate` と同じく、配信の状態を変えたときだけ通知する。根拠: Thread `cdd5fcfb-fd79-4899-ac30-bbf1a4c7393f`。ルート: 委任
- 識別子の上限の置き場: 購読の識別子の上限を common から出し、controller の 2 つの入口が同じ定数を使う。根拠: Thread `65b6130a-418a-4bbd-9405-d25e52a57f4d`。ルート: 委任
- 組み立ての置き場: `StateSubscriptionPresenter` と `StateSubscriptionUsecase` の組み立てを daemon で行い、controller は組み立て済みのものを受け取る。根拠: Thread `d8e1ffee-ef09-49fd-bade-1734a596c69a`。ルート: 委任
- terminal の登録の項目の一元化: 登録に必要な項目の並びを 1 か所で表し、2 つの Output Boundary と Hub がそれを使う。根拠: Thread `e047edc9-0682-4714-ac1e-21c259292ea8`。ルート: 委任
- usecase の隣接テストの逆依存: gateway と presenter を組み立てるシナリオを `src-tauri/tests/` へ移す。根拠: Thread `2bb9b326-4aa2-4fba-9ac0-2324a7e236c1`。ルート: 委任
- テスト補助の重複: `WakeFlag` を `test_support/state_subscription.rs` に 1 つ置く。根拠: Thread `489afbf9-ddf3-403f-a124-bb83784131c0`。ルート: 委任
- 出力順序の区間内での summary 取得の失敗のテスト: その経路を再現するテストを足す。根拠: Thread `c3e4c861-2485-40df-a64a-756e0e0421bb`。ルート: 委任
- snapshot 待ちの間の未処理量のテスト: 失われた状態と復元後の両方で、送る量の制御が数える未処理量を確認するテストを足す。根拠: Thread `0353c41f-499c-43f4-99a5-467c33a1eaa7`。ルート: 委任
- 新しいテストの構造: 購読 ID の境界のテストを Given / When / Then のコメントで区切る。根拠: Thread `b288c3a3-c637-4cc4-af8d-81012e8fc4f5`。ルート: 委任

## 固定するルート

- design-01・design-02・design-03・design-04 で固定したルートは維持する。依頼範囲外の変更は戻すこと、購読の開始・停止の手順は Usecase にあり controller は Usecase を 1 回呼ぶだけであること、Output Boundary は配信状態を戻り値で返さないこと、配信の仕組みの実体は infrastructure が持ち presenter は変換と受け渡しだけを持つこと、購読対象・変化の発生源・watch 要件の型は usecase に置くこと、`WorkflowSubmitArtifactInput` は presenter に置き HTTP local API のリクエスト型は controller に置くこと、push は変換だけを presenter へ移すこと、Output Boundary へ寄せる画面へ届ける口を 6 つに限ること、`adaptor/protocol/` を無くすこと、usecase の Output Data から serde の属性を外すことである。

## 変えないもの

- `PerformanceOutput`（`usecase/telemetry.rs`）の実装が `adaptor/gateway/telemetry.rs` にあること。design-01 から変えていない。
- この変更が触れていない既存の層の逆依存。`infrastructure/terminal/terminal_emulator_test.rs` が `crate::domain::terminal_surface` を参照すること、`usecase/workflow/execution_archive_test.rs` が `crate::adaptor` を参照することは、いずれも `9aceb249` の時点で既にある。
- 転送のメッセージ、JSON のフィールド名と形、Connect のステータスコード、HTTP status。
- 画面（`src/`）のコード。
- `docs/architecture/` の記述。

## 未確定・リスク

- NUL を含む識別子の扱いを揃える先が未確定である。`TerminalSurfaceOwner` 側で拒否すると terminal の起動が失敗する入力が増え、購読の識別子側で受け付けると `SubscriptionTarget` の解析の前提が変わる。どちらにしても、画面と CLI から見える結果が `9aceb249` と同じである必要がある。
- 配信対象を有効にする順序を registry への登録の後へ移すと、登録から有効化までの間に出力が始まりうる。その間の出力が版の起点とずれると、R-009 の続きからの再開の条件が変わる。
