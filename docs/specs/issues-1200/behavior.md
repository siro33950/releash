## B-001: 対象 command の ws 経由の呼び出し

GIVEN クライアント ws 接続が確立している
WHEN クライアントが対象ドメインに登録された command 名と引数を持つ要求を `request_id` 付きで送る
THEN 同じ `request_id` を持つ応答が返る
AND その応答は、command が成功した場合は結果を、失敗した場合はエラーを持つ

## B-002: Tauri invoke との結果の一致

GIVEN 対象ドメインに登録された command が Tauri invoke とクライアント ws の双方から呼べる
WHEN 同じ状態で同じ引数を与えて、その command を Tauri invoke 経由とクライアント ws 経由でそれぞれ呼ぶ
THEN 双方が同じ成功値、または同じエラーを返す

## B-003: Protocol Buffers バイナリ frame

GIVEN クライアント ws 接続が確立している
WHEN クライアントが req/resp の要求を送り、応答・push・stream の frame を受け取る
THEN 送受信される frame はバイナリ frame である
AND 各 frame は `.proto` に定義されたメッセージとして復号できる

## B-004: terminal の単一接続での動作

GIVEN クライアント ws 接続が 1 本確立している
WHEN クライアントがその接続で terminal に attach し、terminal へ入力し、terminal の大きさを変える
THEN terminal の出力が同じ接続で届く
AND 入力と大きさの変更が terminal に反映される
AND 同じ接続で req/resp と push のやり取りが続けられる

## B-005: attachment 単位の順序と ack

GIVEN クライアント ws 接続上で terminal への attach が確立している
WHEN terminal が出力を stream として送る
THEN 届く stream frame は `attachment_id` と、その attachment 内の順序を示す `sequence` を持つ
AND クライアントは `attachment_id` と受信済みの `sequence` を `ack` として送れる

## B-006: frame 上限を超えるデータの分割

GIVEN クライアント ws 接続上で terminal への attach が確立している
WHEN frame 上限を超える大きさの出力が発生する
THEN 送られる各 frame は frame 上限以下である
AND クライアントが受け取る出力は、欠落と順序の入れ替わりなく元の出力と一致する

## B-007: `/v1/terminal` の不在

GIVEN local API が起動している
WHEN クライアントが有効な token を提示して `/v1/terminal` へ接続する
THEN WebSocket 接続は確立しない

## B-008: desktop terminal が Tauri Channel を使わない

GIVEN desktop が起動し、terminal が表示されている
WHEN terminal が出力を表示し、利用者が terminal へ入力する
THEN 出力の受信と入力の送信はクライアント ws だけで行われる
AND Tauri Channel による出力の受信は行われない

## B-009: CLI / provider hook の HTTP 入口の維持

GIVEN local API が起動している
WHEN CLI または provider hook が master token で workflow / provider-lifecycle の HTTP endpoint を呼ぶ
THEN 変更前と同じ endpoint で受理される
AND 変更前と同じ結果が返る

## B-010: `get_terminal_stream_endpoint` の不在

GIVEN desktop が起動している
WHEN renderer が `get_terminal_stream_endpoint` を Tauri invoke で呼ぶ
THEN 接続情報は返らず、エラーが返る

## B-011: desktop の UI 機能の ws 経由の動作

GIVEN desktop が起動し、クライアント ws 接続が確立している
WHEN 利用者が、対象ドメインの command（`get_application_startup_outcome` と `quit_after_startup_failure` を除く）を呼ぶ UI 機能を操作する
THEN その command はクライアント ws の要求として送られる
AND 画面には変更前と同じ結果が反映される

## B-012: 起動失敗時の起動失敗画面

GIVEN 起動 authority が失敗し、local API が bind されていない
WHEN desktop の画面が表示される
THEN desktop は `get_application_startup_outcome` を Tauri invoke で呼び、起動失敗画面を表示する
AND 利用者が終了を選ぶと、desktop は `quit_after_startup_failure` を Tauri invoke で呼び、アプリケーションが終了する

## B-013: 切断後の terminal の再 attach

GIVEN desktop に terminal が表示され、クライアント ws 上で attach している
WHEN クライアント ws が予期せず切断され、その後クライアント ws の接続が確立する
THEN terminal はクライアント ws 上で再 attach され、再同期した出力が表示される
AND 利用者の入力が terminal に反映される

## B-014: 接続失敗後の terminal の attach

GIVEN クライアント ws に接続できず、desktop の terminal が attach できていない
WHEN クライアント ws の接続が確立する
THEN terminal はクライアント ws 上で attach され、出力が表示される
AND 利用者の入力が terminal に反映される

## B-015: attach 要求へのエラー応答の表示

GIVEN desktop が terminal を表示し、クライアント ws 接続が確立している
WHEN terminal の attach 要求にクライアント ws 上でエラー応答が返る
THEN desktop はその応答のエラーを terminal のエラーとして表示する

## B-016: 応答を受け取れなかった監視開始要求の監視を残さない

GIVEN クライアント ws 接続上で監視開始の要求（`start_watching` または `start_git_dir_watching`）が受理されている
WHEN その要求への応答をクライアントが受け取る前に、そのクライアント ws 接続が切断される
THEN その要求で開始された監視は backend に残らない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007, B-010 |
| R-008 | B-008 |
| R-009 | B-009 |
| R-010 | B-011 |
| R-011 | B-012 |
| R-012 | B-013, B-014 |
| R-013 | B-015 |
| R-014 | B-016 |
