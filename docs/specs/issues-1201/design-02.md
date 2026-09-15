# Design 02

## 開始状態

- 差分の基準は `main` から派生した `feat/issues/1201` の `dad6171d` に、[Design 01](design-01.md) の周で加えた未コミットの実装（作業ツリーの変更と未追跡ファイル）を含めた状態。
- 直前の Design は [Design 01](design-01.md)。
- この周までに解消・見送りとなった Thread はない。open Thread 17 件（`aee49ad9`、`3c37c509`、`406cdbeb`、`7e6e23ac`、`8dd3c8ed`、`93a1b045`、`6bcf9576`、`ac6bfeb8`、`c75e9413`、`f46da0ee`、`0034c1ef`、`bdeb42f6`、`c63aa922`、`d1277549`、`f3b20e38`、`a1a61c03`、`7c00c878`）はすべて `[FIX_POLICY]` 付きで、この周で変える。
- Design 01 の「未確定・リスク」のうち backend の再起動（R-011、B-013）の判定構成は、Requirements の Assumptions（別の世代の backend へ再接続させた状態で判定する）で確定した。

## 変える部分

- 非画像 binary の差分表示での blob 取得の撤去: `get_review_blob` による取得を kind=image に限り、非画像 binary は `get_review_file_view` のメタデータだけで「Binary file」を表示し、blob 取得の成否に表示を左右させない。根拠: Thread `aee49ad9`、R-001「変更前と同じ結果が画面へ反映される」、B-001、R-004「review の画像差分表示は、…クライアント ws から取得したデータで行われ、変更前と同じ画像が表示される」、B-006。ルート: 委任
- 操作の同一性・再試行の受理・再実行可否の判定の Rust への移設: 変更要求と元の操作との同一性、再試行の受理、結果不明後の再実行可否を Rust が判定し、`clientSocket.ts` は操作識別子の受け渡しと Rust の判定結果の適用・状態表示だけを行う。frontend が引数の比較で元の操作を判定して Rust に渡す前に拒否する経路と、command 名と分類の組み合わせで再送可否を独自に決める経路を残さない。根拠: Thread `3c37c509`、Requirements Context「操作の受理・重複判定・結果照会は Rust が所有し、frontend は状態表示と利用者の操作受付を行う」、R-009、B-012、R-008、B-025、R-021、B-024。ルート: 委任（固定するルート「ws クライアントの一元化」は維持する）
- 操作状態の受理・遷移の domain 集約への集約: 操作の世代に基づく受理、結果の記録と照会、監視確定の状態遷移を domain の集約が所有し、controller は usecase を呼ぶだけで受理判定を持たず、gateway は自前の状態機械・判断・controller の wire 型を保持しないようにする。Tauri 以外の入口からも同じ usecase 経由で同じ判定を使えるようにする。根拠: Thread `406cdbeb`、Requirements Context「操作の受理・重複判定・結果照会は Rust が所有」、`docs/architecture/CONTROLLER.md`・`GATEWAY.md`・`DOMAIN.md`。ルート: 委任
- domain の操作方針からの転送形式の除去: domain の操作方針（冪等な変更要求の送り直し順序の対象）を操作の種類と対象のドメインの語彙で表し、frontend の JSON 引数名（camelCase のキー）との対応を adaptor 側の境界で完結させる。冪等な変更要求の送り直し順序の挙動は変えない。根拠: Thread `7e6e23ac`、`docs/architecture/DOMAIN.md`「転送形式から独立」、Requirements Context「各 command の変更要求が冪等か…の分類は Rust が所有する」。ルート: 委任
- 期限後に届いた監視開始・監視復旧の成功の扱い: 監視開始・監視復旧の成功応答が期限後に届いても、監視 ID を利用側の所有先へ対応付けて file-change 通知の照合と停止をできるようにし、所有先が既に無い監視は停止して、受領確認によって所有先の無い監視が backend に残り続けないようにする。根拠: Thread `93a1b045`、R-016、B-016、R-019「期限を超えてから応答が届いた要求についても、…画面には現在の状態が反映される」、B-019、R-013、B-017。ルート: 委任
- backend の世代変更をまたぐ監視のローカル識別の衝突解消: 既存監視の復旧と切断中に要求した新規監視の開始が世代変更をまたいで交差しても、ローカルの監視識別が衝突せず、復旧後の通知と停止がそれぞれ元の所有先に対応するようにし、その交差を通すテストで検証する。根拠: Thread `6bcf9576`、R-016「接続の回復後は、必要な状態の再取得・購読の復旧…が行われ、現在の状態が画面へ反映される」、B-016。ルート: 委任
- push を購読しない画面の期限後応答・接続回復時の状態反映: 期限後に確定応答が届いた場合と接続が回復した場合に、push を購読しない画面（設定画面の `get_workflow_config` / `get_external_editor` / `detect_editors` など）も現在の状態を反映し、初期値と結果不明エラーのまま残らないようにする。根拠: Thread `ac6bfeb8`、R-019、B-019、R-016、B-016、R-015、B-017。ルート: 委任
- worktree 作成画面の結果不明の表示: `create_worktree` の応答待ち期限超過や切断で結果不明になった場合、作成画面は「Failed to create」などの処理失敗を表示せず操作結果を確認できないことを表示し、未送信は未実行、backend の確定エラーは失敗として区別して表示する。根拠: Thread `c75e9413`、R-007「結果不明の要求は、未実行または処理失敗として表示されない」、R-013、B-017「その要求は処理失敗として表示されず、操作結果を確認できないことが表示される」、R-015。ルート: 委任
- 初回接続時の状態取得の重複送信の解消: 初回接続時に、mount 時の初回取得と同じ状態取得（`useRepoList` の `get_repo_paths`、`useWorktreeList` の一覧取得など）を重複して送らないようにし、再接続時の再取得は維持する。根拠: Thread `f46da0ee`、R-016「接続の回復後は、必要な状態の再取得…が行われ」、AGENTS.md「状態の所有者を明確にする」。ルート: 委任
- 未使用になった `canClose` / `can_close` の除去: 撤去済みの Close 操作のためだけに残る `canClose` / `can_close` を domain・usecase・gateway・protocol 変換・proto・生成 TS 型・frontend 型・テスト fixture から除去し、workspace tree の表示と他の capability は変えない。根拠: Thread `8dd3c8ed`、Design 01「未登録 command の Tauri invoke 呼び出しの撤去」、R-002、B-003。ルート: 委任
- E2E ヘルパーの廃止済み agent notice 分岐の削除: `tests/helpers/tauri-mock.ts` に残る `get_agent_session_notice` / `update_agent_session_notice` の分岐と、その専用の Map・revision を削除する。観測可能な挙動は変えない。根拠: 人間の決定（前工程の out_of_scope 所見を削除対象とした）、R-005、B-007。ルート: 委任
- worktree 作成完了から一覧更新までの検証の追加: worktree 作成完了から Tauri event を使わない frontend 内の通知を経て一覧が再取得され、新しい worktree が表示されるまでを検証するテストを加える。根拠: Thread `0034c1ef`、R-001、B-001、R-002、B-003、Design 01「frontend 内の Tauri event 送信の撤去」。ルート: 委任
- ws へ移行した購読先の再接続時の再取得の検証の追加: クライアント ws へ移行した購読先（`useRepoList`、`useWorktreeList`、`useGitEventRefresh`、`useAutomation`、`agentSessionEvents`）について、切断中に backend の状態が変わり後続の push がない場合も、接続の回復時に再取得して現在の状態を反映することを検証するテストを加える。根拠: Thread `bdeb42f6`、R-016、B-016。ルート: 委任
- Hello で届く Rust の期限値の適用の検証の追加: backend が Hello で生成済み既定値と異なる生存確認間隔・応答期限・操作別期限を返したとき、その値が実際の待機と状態遷移に使われることを検証するテストと、Rust が Hello に設定する値を確認するテストを加える。根拠: Thread `c63aa922`、Requirements Context「期限・失敗分類・復旧方針…は Rust が所有する」、R-012、B-014、B-015、R-013、B-017。ルート: 委任
- 結果照会の pending 後の定期再照会による確定の検証の追加: 接続回復後の結果照会で pending を受けた後、利用者の操作や手動の照会なしに再照会して元の要求 ID の確定結果を反映し、副作用が一度であることを検証するテストを加える。根拠: Thread `d1277549`、R-010「受理後・完了前…に切断された変更要求のうち結果を確定できるものは、接続の回復後に元の操作の結果として確定し、画面へ反映される」、B-010、B-011。ルート: 委任
- 期限切れの冪等な変更要求の利用者による再試行の検証の追加: 期限切れの冪等な変更要求を利用者が再試行し、結果照会・同じ要求 ID での送り直し・結果の確定までを通り、新しい操作 ID や重複した副作用が生じないことを検証するテストを加える。根拠: Thread `f3b20e38`、R-009「利用者が結果不明の変更要求を再試行した場合、その再試行は元の操作と対応付けられ、副作用が重複しない」、B-012、R-021、B-024。ルート: 委任
- 別世代の backend での復旧送り直しの受理と重複防止の検証の追加: 別世代の backend へ再接続した状態で、再起動後の再実行が許可された冪等な要求の復旧送り直しが Rust の入口で受理・実行され、同じ要求 ID の再送で二度目の副作用が生じないことを Rust の実行経路で検証するテストを加える。根拠: Thread `a1a61c03`、R-021「送り直された場合も、backend の状態はその要求を一度だけ実行した場合と同じになる」、B-024、R-011、B-013、Requirements Assumptions（別の世代の backend へ再接続させた状態で判定する）。ルート: 委任
- 同じ対象に作用する異なる冪等 command の送り直し交差の最終状態の保証と検証: 同じ対象に作用する異なる冪等 command（`add_repo_path` と `remove_repo_path`）の結果不明・世代変更・後続操作・復旧の送り直しが交差しても、最終状態が元の操作順に一度ずつ実行した状態になるようにし、その最終状態と副作用を状態を持つ対象を通して検証するテストを加える。根拠: Thread `7c00c878`、R-021、B-024。ルート: 委任

## 固定するルート

今周に新たに固定する実装上の指定なし。

次は Design 01 で固定し、今周も維持する（Thread `3c37c509` の修正方針で維持を明示）。

- ws クライアントの一元化（Design 01）: frontend の ws クライアントは `src/lib/clientSocket.ts` の 1 箇所に限る。要求の追跡（未送信・結果不明・結果確定の区別）、未送信要求の期限付き待機、個別要求の期限、生存確認、接続状態の扱いは、呼び出し側（hook・component）ではなくこの 1 箇所で行い、呼び出し側は結果と状態を受け取るだけにする。期限の値と冪等性の分類は Rust が所有し、`clientSocket.ts` はそれを適用する側である。
  - 範囲: frontend のクライアント側処理全体。
  - 粒度: 配置の固定（内部構造は委任）。
- 既存の操作識別・照会機構の優先利用（Design 01）: 冪等でない command の結果確認・重複防止には、既存の caller attempt 機構（local event store の caller_attempts / operation_bindings、(principal, installation_id, kind, caller_request_id) を鍵とする admission）と、application quit の操作 journal（`list_pending_application_attempts` / `acknowledge_application_attempt` / `get_application_quit_operation`、`OutcomeUnknown`）を優先して利用する。既存機構に載せられない command の扱いは委任する。
  - 範囲: 冪等でない変更 command の結果確認・重複防止。
  - 粒度: 「優先して利用する」まで（全 command を既存機構へ寄せることは固定しない）。

## 変えないもの

- review の画像差分表示は、クライアント ws から取得したデータで行う（Thread `aee49ad9` の人間の決定。非画像 binary の取得を撤去しても、画像差分表示の取得経路は R-004 / B-006 のとおり維持する）。

## 未確定・リスク

- 自動判断（Requirements の Assumptions）: R-002 と B-003 の Tauri IPC の用途に「起動が成功したか失敗したかを判定するための起動結果の取得」を含めた。この解釈が意図と異なる場合、起動成功時の判定を Tauri invoke 以外へ移す変更が R-002 を満たすために必要になる。
- 未決のまま残した要求: なし。
- `[DEFERRED]` で人間へ渡した件: なし。
