# Design 04

## 開始状態

design-03 の実装を確定した `ac5f3152`（`chore(development-v2): design-03 の実装を確定する`）を基準にする。直前の Design は `design-03.md`、差分の基準（base ブランチと派生点）は design-01 のまま `main` の `9aceb249` である。この周までに閉じた Thread は 44 件である。open Thread は 13 件あり、すべてに `[PLAN]` を付けた。うち 8 件（`cdd5fcfb`・`a70df8f4`・`7e817f4f`・`da35bce4`・`b7fa8a67`・`3fd2518f`・`b288c3a3`・`f7b48fd2`）は前の周の `[PLAN]` の条件を満たさず `[NOT_FIXED]` が付いたもので、`[PLAN]` を書き直した。`6a2a415a`（status の tuple から DTO への変換の重複）は、原因となった変更を戻すと決めたため `wontfix` で閉じた。

## 変える部分

- 依頼範囲外の変更を戻す: repository status の読み取り経路と domain の `StatusRepository` 一式の削除を `9aceb249` の形に戻す。根拠: Thread `08b0c62e-8ed2-4ba1-872f-8e8beb366f08`。ルート: 人間が固定（下記「固定するルート」）
- snapshot 待ちの client の未処理量: 続きから再開できない terminal の購読で、snapshot が届くまでの出力を送る量の制御の未処理量に数えない。根拠: Thread `19cff710-a497-4886-9d6f-307081b7ab76`。ルート: 委任
- 状態を変えていないときの通知の抑止: `StateSubscriptionRuntime::mutate` が closure の結果にかかわらず通知する形をやめる。根拠: Thread `75f43523-720b-4f13-9eb8-7d3d38b2e86f`。ルート: 委任
- 状態を変えたのに通知しない経路の解消: `start` が失敗しても `ensure_active` が対象を削除して世代を進める場合に通知する。根拠: Thread `cdd5fcfb-fd79-4899-ac30-bbf1a4c7393f`。ルート: 委任
- terminal の開始の出力順序の区間: 世代が変わっていたら取り直して区間をやり直し、送り待ちの量の取得と出力の購読の登録を有効な区間で連続して行う。根拠: Thread `a70df8f4-5109-4426-87f3-0168842daace`。ルート: 委任
- Output Data からの domain 型と Entity 起点の変換の除去: `TerminalSurfaceOutputSummary` から domain の型を外し、`From<&TerminalSurfaceSummary>` を無くす。根拠: Thread `b7fa8a67-6b00-4c87-839a-54dc73630735`。ルート: 委任
- テストの配置と名前: `usecase/state_subscription/terminal.rs` の隣接テストを置く。presenter に残る購読手順のテストを usecase 隣接か `src-tauri/tests/` へ移し、presenter の隣接テストを規約どおりのファイル名とモジュール名に集約する。根拠: Thread `da35bce4-e917-410a-a5ca-9c5881e58ab7`。ルート: 委任
- daemon の二重配線: Output Boundary の実装を 1 回だけ作り、Usecase と controller が同じ参照を得る配線を 1 つにする。根拠: Thread `7e817f4f-216b-4fbf-946e-196ba115d7d0`。ルート: 委任
- 識別子の上限の定数化: 判定とエラー文言の両方が名前付き定数を使う。根拠: Thread `3fd2518f-b1f2-43fa-bbdc-384920a19b5a`。ルート: 委任
- 失敗の対応表の網羅: `SnapshotRequired` を Connect の対応表のテストに足す。根拠: Thread `f7b48fd2-e322-41fb-9512-7f546b00b9a1`。ルート: 委任
- 新しいテストの構造: 購読 ID の境界のテストと、初回読取・再読取・停止のテストを Given / When / Then のコメントで区切る。根拠: Thread `b288c3a3-c637-4cc4-af8d-81012e8fc4f5`。ルート: 委任
- テストの命名: 実行開始応答のテストの名前を規約の形にする。根拠: Thread `c666e609-d2fc-4032-8434-16c0784cbb1f`。ルート: 委任
- 到達しない分岐の削除: `start_read` から Terminal の分岐を消す。根拠: Thread `6536805d-af12-4a87-a6c5-e83cf2f6618a`。ルート: 委任

## 固定するルート

- 依頼範囲外の変更は戻す。`domain/repository/entities/file_status.rs` の `FileStatus`・`FileDiffStat`・`RepositoryStatusScan`、`domain/repository/repository.rs` の `StatusRepository`、`usecase/repository_usecase.rs` の status 依存と `get_repository_status_scan`、`usecase/repository_dto.rs` の Entity から DTO への `From` 実装を `9aceb249` の形に戻し、`adaptor/gateway/repository/scanner.rs` を Usecase 経由の呼び出しに戻す。
- design-01・design-02・design-03 で固定したルートは維持する。購読の開始・停止の手順は Usecase にあり controller は Usecase を 1 回呼ぶだけであること、Output Boundary は配信状態を戻り値で返さないこと、配信の仕組みの実体は infrastructure が持ち presenter は変換と受け渡しだけを持つこと、購読対象・変化の発生源・watch 要件の型は usecase に置くこと、`WorkflowSubmitArtifactInput` は presenter に置き HTTP local API のリクエスト型は controller に置くこと、push は変換だけを presenter へ移すこと、Output Boundary へ寄せる画面へ届ける口を 6 つに限ること、`adaptor/protocol/` を無くすこと、usecase の Output Data から serde の属性を外すことである。

## 変えないもの

- `PerformanceOutput`（`usecase/telemetry.rs`）の実装が `adaptor/gateway/telemetry.rs` にあること。design-01 から変えていない。
- この変更が触れていない既存の層の逆依存。`infrastructure/terminal/terminal_emulator_test.rs` が `crate::domain::terminal_surface` を参照すること、`usecase/workflow/execution_archive_test.rs` が `crate::adaptor` を参照することは、いずれも `9aceb249` の時点で既にある。
- 転送のメッセージ、JSON のフィールド名と形、Connect のステータスコード、HTTP status。
- 画面（`src/`）のコード。
- `docs/architecture/` の記述。

## 未確定・リスク

- 依頼範囲外の変更を戻す範囲を、repository status の読み取り経路と domain の `StatusRepository` 一式に限れるかが未確認である。`adaptor/gateway/repository/status.rs` は今回 132 行が変わっており、戻す範囲を誤ると、この変更で直した別の項目まで戻る。
- snapshot の復元を待つ client を送る量の制御の対象から外すには、terminal の出力順序ロック（gateway）と送る量の制御（infrastructure）の両方に関わる。除外の入れ方によっては、復元後に数え始める時点の未処理量が実際とずれ、R-010 の停止と再開の条件が変わる。
