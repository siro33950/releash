# Design 06

## 開始状態

- 差分の基準は `main` から派生した `feat/issues/1201` の `dad6171d` に、[Design 01](design-01.md)〜[Design 05](design-05.md) の周で加えた未コミットの実装（作業ツリーの変更と未追跡ファイル）を含めた状態。
- 直前の Design は [Design 05](design-05.md)。
- Design 05 の周で解消した Thread は 3 件（`e16c2801`、`c35cc4f6`、`7c00c878`）。見送りとなった Thread はない。
- Design 05 から open のまま残る Thread は 1 件（`e63c9f19`）で、Design 05 で対応した 4 操作は解消し、同じ原因の残存操作が `[STILL_OPEN]` で示され、受入条件の対象を残存操作へ広げた `[FIX_POLICY]` が追記されている。今周に新たに `[FIX_POLICY]` が付いた Thread は 1 件（`74bd95ac`）。open Thread 2 件をすべてこの周で変える。
- この周で Requirements・Behavior は変更していない。

## 変える部分

- 別世代へ再接続した後の旧世代の未送信の監視停止の結果不明表示: 切断中に停止した監視の `stop_watching`（旧世代の識別を持つ未送信の要求）が、期限内に別世代の backend へ再接続して backend から結果不明と判定されても、desktop が未送信の要求として扱い続けて接続回復待ち（未送信）と表示し、期限到達時に未実行として表示する状態を改め、backend の判定どおり操作結果を確認できないことを表示し、接続回復待ちの表示のまま残さず、期限後も未実行として表示しない。旧世代の監視の停止は、新世代の同じ数値 ID を持つ別の監視へ適用しない。根拠: R-007「結果不明の要求は、未実行または処理失敗として表示されない」、B-009「その要求は、結果不明の要求および結果が確定した要求と区別される」、R-011「変更要求の結果を確定できない場合は、結果不明が表示され、安全を確認できない再実行は行われない」、B-013、R-015「画面は待機中のまま残らず、…操作結果を確認できないことが表示される」、Thread `74bd95ac`。ルート: 委任（固定するルート「ws クライアントの一元化」は維持する）
- 送信済みの retry・設定保存・provider 操作の結果不明時の画面表示（Design 05 からの残存）: 結果を待つ要求である `retry_workspace_node`・`update_external_editor`・`set_releash_base`・`save_notion_config`・`delete_notion_config`・`update_provider_executable`・`reset_provider_executable`・`refresh_provider_availability` が送信済みのまま期限超過や切断で結果不明になると、呼び出し側が結果不明の通知を受け取らず、workspace node の Retrying... と操作無効、設定画面の Save の spinner と操作無効、provider availability 設定の保存・リセット・再取得の待機表示と入力無効が続く状態を改め、各画面は待機表示と操作無効のまま残らず、処理失敗ではなく操作結果を確認できないことを表示し、後から結果が確定した場合は確定した結果を各画面へ反映し、副作用を重複させない。Design 05 で対応した worktree 削除・branch 削除・workspace node の承認・workflow 設定の保存と、背景設定の結果不明表示は維持する。根拠: R-015「個別要求の期限超過を検知した場合、画面は待機中のまま残らず、…操作結果を確認できないことが表示される」、B-017「その要求は処理失敗として表示されず、操作結果を確認できないことが表示される」、R-007「結果不明の要求は、未実行または処理失敗として表示されない」、R-010「元の操作の結果として確定し、画面へ反映される」、Design 05 変える部分「送信済みの削除・承認・workflow 設定保存の結果不明時の画面表示」、Thread `e63c9f19`。ルート: 委任（固定するルート「ws クライアントの一元化」は維持する）

## 固定するルート

今周に新たに固定する実装上の指定なし。

次は Design 01 で固定し、Design 02〜Design 05 と今周も維持する。

- ws クライアントの一元化（Design 01）: frontend の ws クライアントは `src/lib/clientSocket.ts` の 1 箇所に限る。要求の追跡（未送信・結果不明・結果確定の区別）、未送信要求の期限付き待機、個別要求の期限、生存確認、接続状態の扱いは、呼び出し側（hook・component）ではなくこの 1 箇所で行い、呼び出し側は結果と状態を受け取るだけにする。期限の値と冪等性の分類は Rust が所有し、`clientSocket.ts` はそれを適用する側である。
  - 範囲: frontend のクライアント側処理全体。
  - 粒度: 配置の固定（内部構造は委任）。
- 既存の操作識別・照会機構の優先利用（Design 01）: 冪等でない command の結果確認・重複防止には、既存の caller attempt 機構（local event store の caller_attempts / operation_bindings、(principal, installation_id, kind, caller_request_id) を鍵とする admission）と、application quit の操作 journal（`list_pending_application_attempts` / `acknowledge_application_attempt` / `get_application_quit_operation`、`OutcomeUnknown`）を優先して利用する。既存機構に載せられない command の扱いは委任する。
  - 範囲: 冪等でない変更 command の結果確認・重複防止。
  - 粒度: 「優先して利用する」まで（全 command を既存機構へ寄せることは固定しない）。

## 変えないもの

- review の画像差分表示は、クライアント ws から取得したデータで行う（Design 02 で維持した Thread `aee49ad9` の人間の決定。R-004 / B-006 のとおり維持する）。

## 未確定・リスク

- 自動判断（Requirements の Assumptions）: R-002 と B-003 の Tauri IPC の用途に「起動が成功したか失敗したかを判定するための起動結果の取得」を含めた。この解釈が意図と異なる場合、起動成功時の判定を Tauri invoke 以外へ移す変更が R-002 を満たすために必要になる。
- 自動判断（Requirements の Assumptions）: R-024 で、冪等であることを「記録の破棄後も backend が操作の識別子で重複を防ぐ仕組みに載る」ことに含めなかった。この解釈が意図と異なる場合、記録が破棄された冪等な変更要求（例: `add_repo_path`）の利用者の再試行を元の操作として受け付ける変更が R-024 / B-029 を満たすために必要になり、その際に確定済みの後続操作を取り消さない条件（R-021）との両立が要る。
- 未決のまま残した要求: なし。
- `[DEFERRED]` で人間へ渡した件: なし。
