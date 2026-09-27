# Design 02

## 開始状態

design-01 の実装を確定した `a1952a6e`（`chore(development-v2): design-01 の実装を確定する`）を基準にする。直前の Design は `design-01.md`、差分の基準（base ブランチと派生点）は design-01 のまま `main` の `9aceb249` である。この周までに閉じた Thread は `fdfd3b5c-2ad4-472e-b8a5-386a543f2843`。open Thread は 16 件あり、すべてに `[PLAN]` を付けた。

## 変える部分

- 配信の実体の移設: 送り待ちの列、送信の駆動、session ごとの送る量の制御の実体を presenter から infrastructure へ移す。根拠: Thread `15dea66a-139c-4b91-ac0b-76139f9e2483`。ルート: 人間が固定（下記「固定するルート」）
- Output Boundary の絞り込み: `StateSubscriptionOutput` を、Output Data を外へ出す一方向の口にし、配信状態の照会と client ごとの送り待ち・cursor・terminal 入力の操作を usecase へ公開しない。根拠: Thread `d0cfed79-3c57-43ce-8b5c-7a932b6b05e2`。ルート: 委任
- 購読対象の語彙の置き場の訂正: 購読対象・変化の発生源・watch 要件の型は usecase に置き、転送の識別子の文字列表現だけを presenter へ出す。根拠: Thread `9af4979c-06d2-48b1-a7b0-5bba9b38c21d`。ルート: 人間が固定（下記「固定するルート」）
- presenter の状態の可視性: `StateSubscriptionPresenter` に残る状態から `pub(crate)` を無くす。根拠: Thread `cdd5fcfb-fd79-4899-ac30-bbf1a4c7393f`。ルート: 委任
- `WorkflowSubmitArtifactInput` の置き場の訂正: `adaptor/controller/api/protocol.rs` から presenter へ戻し、presenter から controller の型への参照を無くす。根拠: Thread `38665ae4-5ea6-435a-96b9-a3765a25eb71`。ルート: 人間が固定（下記「固定するルート」）
- 入力検証の差し戻し: 購読 ID の 128 バイト超過の判定を presenter から controller へ戻す。根拠: Thread `7e97feb4-3645-4526-939d-618a547ad8cd`。ルート: 委任
- 購読の入口の配線: `ClientApiDeps` が同じ配信状態を指す 2 つのフィールドを持つ形と、`with_state_subscriptions` の `#[cfg(test)]` による暗黙導出を無くす。根拠: Thread `7e817f4f-216b-4fbf-946e-196ba115d7d0`。ルート: 委任
- Entity から Output Data への変換の除去: `impl From<&TerminalSurfaceSummary> for TerminalSurfaceOutputSummary` を usecase から外し、usecase から購読対象への参照を無くす。根拠: Thread `3d0830b1-05a3-4391-b963-0eb8758fe0bd`。ルート: 委任
- 版の変換の重複の解消: `start_state_subscription` の両分岐にある同じ変換式を 1 つにする。根拠: Thread `0abe0761-1718-49ef-b64d-731f224b2149`。ルート: 委任
- infrastructure のテストの逆依存の解消: `infrastructure/state_subscription_test.rs` から usecase への参照を無くす。根拠: Thread `e2c66426-1dc1-4108-ba5c-5654dbe9d4d1`。ルート: 委任
- usecase のテストの逆依存の解消: `usecase/workflow/` のテストから presenter のレスポンス型への参照を無くす。根拠: Thread `9ba8da2c-e304-4b37-a851-1ea8119334e2`。ルート: 委任
- 購読手順のテストの差し戻し: usecase の購読手順を検証するテストを usecase の隣接テストへ戻し、層をまたぐシナリオは `src-tauri/tests/` へ置く。presenter から usecase の型へのテスト専用の `impl` を無くす。根拠: Thread `da35bce4-e917-410a-a5ca-9c5881e58ab7`。ルート: 委任
- notifier のテストの追従: `ClientRepositoryStateNotifier` のテストを実装と同じ presenter へ移す。根拠: Thread `f7745eda-3e82-4dcf-b9a9-dca1d87dbe9e`。ルート: 委任
- workflow API presenter のテストの構造: 4 つのテストを Given / When / Then のコメントで区切る。根拠: Thread `4b990796-bc9e-4074-9bef-46bef9653ccd`。ルート: 委任
- 実行一覧応答の optional フィールドのテスト: 値があるときの `currentNode` / `completedAt` / `errorReason` を照合するテストを足す。根拠: Thread `ea38cf0c-aeaf-4d37-b29d-753ffb02469d`。ルート: 委任
- provider lifecycle の Rejected 応答のテスト: `status` と 10 種類の reason 文字列の JSON を照合するテストを、実装と同じディレクトリに足す。根拠: Thread `75e53d38-554c-45c0-8e24-be91d1650d66`。ルート: 委任
- 購読メッセージの転送変換のテスト: いま `presenter/state_subscription_wire_test.rs` が確認しているのは BranchBase の Snapshot の target と args と種別だけで、版の値、payload、Change（Full と Delta）、Bookmark、Ready、および各 `StateValue` の転送への対応が固定されていない。根拠: B-008「画面が受け取るメッセージの種類、フィールド、版の値は、この変更の前と同じである」。ルート: 委任

## 固定するルート

- 配信の仕組みの実体は infrastructure が持つ。presenter は Output Data を転送の形へ変換してから配信の口へ渡す。infrastructure は、版、送り待ちの列、bookmark、送信の駆動と stream、session ごとの送る量の制御の実体を持つ。snapshot の作り直しと、続きから再開できるかの判定も、転送の形のまま行う。presenter に残すのは変換と、配信の口への受け渡しだけである。
- 購読対象・変化の発生源・watch 要件の型は usecase に置く。design-01 の「変える部分」がこれらの移動先を presenter と infrastructure に固定したのは誤りであり、この周で解除する。R-007 で usecase に残す購読の手順がこれらを使い、usecase は presenter に依存できないためである。usecase から出すのは、転送の識別子の文字列表現（`name:長さ:引数` の組み立てと解析）だけであり、これは presenter が持つ。
- `WorkflowSubmitArtifactInput` は presenter に置く。design-01 がこれを HTTP local API のリクエスト型に分類したのは誤りであり、この周で解除する。非テストの利用は Connect ClientService の入口（`adaptor/controller/client/workflow/output.rs`）だけである。HTTP local API のリクエスト型を `adaptor/controller/api/protocol.rs` に置くこと自体は design-01 のまま維持する。
- design-01 で固定した残りのルートは維持する。購読の開始・停止・再読み取り・watch の調整の手順を usecase の Usecase として残すこと、push は変換だけを presenter へ移し送信機構と購読の入口を変えないこと、Output Boundary へ寄せる画面へ届ける口を 6 つに限ること、`adaptor/protocol/` を無くすこと、usecase の Output Data から serde の属性を外すことである。

## 変えないもの

- `PerformanceOutput`（`usecase/telemetry.rs`）の実装が `adaptor/gateway/telemetry.rs` にあること。design-01 のまま、この周でも変えない。
- 転送のメッセージ、JSON のフィールド名と形、Connect のステータスコード、HTTP status。
- 画面（`src/`）のコード。
- `docs/architecture/` の記述。

## 未確定・リスク

- 配信エンジンが転送の形を扱うようにすると、送り待ちの量（terminal の出力の UTF-16 単位）をエンジンが値から導けなくなる。量を値と一緒に渡す形にできないと、R-010 の送る量の制御の条件が変わる。
- 購読の stream を infrastructure が駆動すると、controller が受け取るのは転送の形の frame になる。これを Connect の server streaming へ、既存の push（`EncodedPush`）と同じ扱いで載せられるかは未確認である。載せられない場合、presenter か controller に送信の駆動が残る。
- Output Boundary を一方向に絞ると、usecase の購読手順が今まで配信状態に問い合わせて決めていた分岐（対象が既に配信中か、snapshot が要るか）を、usecase 自身の状態で決めることになる。usecase の状態と infrastructure の配信状態がずれると、R-009 の再開と snapshot の条件が変わる。
