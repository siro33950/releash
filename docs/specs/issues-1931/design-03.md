# Design 03

## 開始状態

design-02 の実装を確定した `6aa04e41`（`chore(development-v2): design-02 の実装を確定する`）を基準にする。直前の Design は `design-02.md`、差分の基準（base ブランチと派生点）は design-01 のまま `main` の `9aceb249` である。この周までに閉じた Thread は 27 件である。open Thread は 21 件あり、すべてに `[PLAN]` を付けた。うち 9 件（`425ebaac`・`15dea66a`・`cdd5fcfb`・`da35bce4`・`a70df8f4`・`e2c66426`・`9ba8da2c`・`ea38cf0c`・`7e817f4f`）は前の周の `[PLAN]` の条件を満たさず `[NOT_FIXED]` が付いたもので、`[PLAN]` を書き直した。

## 変える部分

- 購読の開始・停止の手順を Usecase へ戻す: いま controller が `usecase.start_subscription` の後に `presenter.present_start` を呼んでいる形をやめ、Usecase の 1 つの手順にする。根拠: Thread `425ebaac-c9cf-40a3-92f1-cf7cd77bcae9`。ルート: 人間が固定（下記「固定するルート」）
- 送る量の制御の実体の移設: presenter の `TerminalSurfaceEventHub` が持つ session ごとの実体を infrastructure へ移す。根拠: Thread `15dea66a-139c-4b91-ac0b-76139f9e2483`。ルート: design-02 の固定を維持
- terminal の開始での取りこぼしの解消: 送り待ちの量の取得と出力の購読の登録を、同じ出力順序の区間で連続して行う。根拠: Thread `a70df8f4-5109-4426-87f3-0168842daace`。ルート: 委任
- terminal の snapshot の作り直しの取りこぼしの解消: 作り直しの要求が重なったときに、後から足された client を落とさない。根拠: Thread `0acb1265-3820-40f7-9498-579a3c80fed2`。ルート: 委任
- 0 単位のイベントの送り待ちへの加算: `publish_delta` で単位を最低 1 に切り上げる形へ戻す。根拠: Thread `5e0ec6c4-9b9d-4900-af83-5181ea049b4d`。ルート: 委任
- bookmark の周期の起点: 要素を送るたびに周期を再設定するのをやめる。根拠: Thread `f19f74d2-5772-4c52-b38a-37026f2e4d1d`。ルート: 委任
- 購読の stream の転送変換: controller で要素ごとに変換を掛ける形をやめ、presenter 側で完結させる。根拠: Thread `7c4bb55d-5987-4000-b85d-527b67497f92`。ルート: 委任
- 購読の入口の配線: controller が購読の開始・停止で presenter を呼ぶ経路と、daemon が同じ配信の実体を 2 回渡す形を無くす。根拠: Thread `7e817f4f-216b-4fbf-946e-196ba115d7d0`。ルート: 委任
- 識別子の長さの判定の集約: controller の 2 つの入口に直接書かれた上限を 1 か所にする。根拠: Thread `3fd2518f-b1f2-43fa-bbdc-384920a19b5a`。ルート: 委任
- Entity から Output Data への詰め替えの除去: gateway へ移しただけで残っている変換を無くす。根拠: Thread `b7fa8a67-6b00-4c87-839a-54dc73630735`。ルート: 委任
- 保護する対象の集合の組み立ての集約: presenter の 2 か所にある同じ組み立てを 1 つにする。根拠: Thread `2e672c8c-7631-4c74-afee-5b101c4427bf`。ルート: 委任
- presenter の状態への到達経路の除去: 通知を伴わない書き換えができる経路を無くす。根拠: Thread `cdd5fcfb-fd79-4899-ac30-bbf1a4c7393f`。ルート: 委任
- テストの配置: `tests/state_subscription/` の各ファイルを presenter の `#[path]` 子モジュールから外して integration test とし、usecase の購読手順の単体テストを usecase の隣接テストに置く。根拠: Thread `da35bce4-e917-410a-a5ca-9c5881e58ab7`。ルート: 委任
- テストヘルパーの重複の解消: 新しい 2 つのテストが独自に定義している payload の比較と terminal 要素の取り出しを、共通のヘルパーに寄せる。根拠: Thread `52174c60-0624-4254-be9e-e66fad0d6f60`。ルート: 委任
- infrastructure のテストの逆依存: この変更が作った・触れたファイルから内側の層への参照を無くす。根拠: Thread `e2c66426-1dc1-4108-ba5c-5654dbe9d4d1`。ルート: 委任
- usecase のテストの逆依存: この変更で足した `crate::adaptor` への参照を無くす。根拠: Thread `9ba8da2c-e304-4b37-a851-1ea8119334e2`。ルート: 委任
- 失敗の対応表の網羅: 新設した `EncodingFailed` を Connect の対応表のテストに足す。根拠: Thread `f7b48fd2-e322-41fb-9512-7f546b00b9a1`。ルート: 委任
- 実行一覧応答のテスト: `completedAt` と `errorReason` が無いときの省略を確認する。根拠: Thread `ea38cf0c-aeaf-4d37-b29d-753ffb02469d`。ルート: 委任
- snapshot が未確定の Delta 対象の開始のテスト: 新設した分岐を通るテストを足す。根拠: Thread `7547b99c-ebfb-4e7b-90e4-139f4cdeae7f`。ルート: 委任
- terminal の開始中の切断のテスト: 新設した競合の分岐と巻き戻しを確認する。根拠: Thread `7ff9b43d-d554-4538-875d-d17b877af04a`。ルート: 委任
- 新しいテストの構造: この変更で足したテストを Given / When / Then のコメントで区切る。根拠: Thread `b288c3a3-c637-4cc4-af8d-81012e8fc4f5`。ルート: 委任

## 固定するルート

- 購読の開始・停止の手順は Usecase に戻す。controller は Usecase の操作を 1 回呼ぶだけにし、配信側の開始・停止を呼ばない。開始時に配信側へ snapshot の要否を問い合わせず、続きから再開できない場合は、既存の overflow から snapshot を要求する経路（配信側が要求し、Usecase が読み直して publish する）に任せる。cursor は Input Data として Usecase が受け取り、そのまま Output Boundary へ渡す。Output Boundary は配信状態（snapshot の要否、送り待ちの量、有効な対象）を戻り値で返さない。terminal の出力順序ロックと出力の購読の登録は、Usecase の手順が Output Boundary の 1 つの操作として呼ぶ。stream の取得だけは、push の `ClientPushGateway` と同じく controller が配信の口から得てよい。
  - この結果、terminal で続きから再開できない購読の snapshot は、開始の中ではなく開始の後に非同期で届く。画面が snapshot を受け取ること（B-011）と、続きから再開できるかの条件（R-009）は変わらない。
- design-01 と design-02 で固定した残りのルートは維持する。配信の仕組みの実体は infrastructure が持ち presenter は変換と受け渡しだけを持つこと、購読対象・変化の発生源・watch 要件の型は usecase に置き転送の識別子の文字列表現だけを presenter が持つこと、`WorkflowSubmitArtifactInput` は presenter に置くこと、HTTP local API のリクエスト型は `adaptor/controller/api/protocol.rs` に置くこと、push は変換だけを presenter へ移すこと、Output Boundary へ寄せる画面へ届ける口を 6 つに限ること、`adaptor/protocol/` を無くすこと、usecase の Output Data から serde の属性を外すことである。

## 変えないもの

- `PerformanceOutput`（`usecase/telemetry.rs`）の実装が `adaptor/gateway/telemetry.rs` にあること。design-01 から変えていない。
- この変更が触れていない既存の層の逆依存。`infrastructure/terminal/terminal_emulator_test.rs` が `crate::domain::terminal_surface` を参照すること、`usecase/workflow/execution_archive_test.rs` が `crate::adaptor` を参照することは、いずれも `9aceb249` の時点で既にあり、この変更が持ち込んだものではない。
- 転送のメッセージ、JSON のフィールド名と形、Connect のステータスコード、HTTP status。
- 画面（`src/`）のコード。
- `docs/architecture/` の記述。

## 未確定・リスク

- 開始時に snapshot の要否を問い合わせないと、terminal の続きから再開できない購読は、開始の直後に snapshot 待ちの状態から始まる。この間の送り待ちの量の初期値が実際の未処理量とずれると、R-010 の停止と再開の条件が変わる。
- 送る量の制御の実体を infrastructure へ移すと、terminal の出力順序ロック（gateway が持つ）と送る量の制御の更新が別の層にまたがる。Thread `a70df8f4` が求める「送り待ちの量の取得と出力の購読の登録を同じ出力順序の区間で行う」を満たせる配線になるかは未確認である。
