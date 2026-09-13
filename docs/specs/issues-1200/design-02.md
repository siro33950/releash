# Design 02

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `28f0eeed`。作業ブランチは `feat/issues/1200`。
- 直前の Design は `docs/specs/issues-1200/design-01.md`。開始状態は、design-01 の周で実装した未コミットの変更を含む現在の作業ツリーである（`proto/client.proto`、`adaptor/protocol/client_wire.rs`、各ドメインの `shared.rs`、`command/terminal_surface/client_stream.rs`、`src/generated/`、`src/lib/clientProtocol.ts` 等の追加と、`api/terminal.rs`・`src/lib/terminalStream*.ts` の削除）。
- Requirements の変更: Assumptions の Q-004 が「未決」から「desktop から呼び出す箇所がない登録済み command 34 本を、他の command と同様にクライアント ws へ被覆する（R-001、R-002 の対象に含む）」へ確定した。R-001〜R-012、B-001〜B-014 と対応表に変更はなく、Requirements と Behavior は欠落なく対応している。
- open Thread は 15 件で、すべて `[FIX_POLICY]` 付きである。この周までに解消・見送りとなった Thread はない。
- レビューで基準 `28f0eeed` から存在し今回の差分が導入・悪化していないと判定された 2 件（`src/components/workspace/WorkspaceList.tsx` の登録のない `close_workspace_node` 呼び出し、`command/workflow/mod.rs` の facet テストが本番と別の操作実装を検証している件）は Thread になっておらず、今周の変える部分に含めない。

## 変える部分

- 呼び出し箇所のない 34 本のクライアント ws 被覆: Current Behavior に列挙した 34 本（repository 8 / code 11 / comment 2 / workflow 10 / app_config・git_host・application_lifecycle 各 1）を、クライアント ws の汎用エンベロープで `request_id` 付きの要求として呼べ、同じ `request_id` の応答で成功値またはエラーが返り、同じ状態・引数の Tauri invoke と同じ結果になるようにする。これらの除外を期待している `adaptor/protocol/client_test.rs` の assertion を要求に合わせて是正し、対象ドメインの登録済み command のうち ws 被覆外を menu と `get_client_endpoint` だけにする。根拠: R-001「対象ドメインに登録された command が提供する操作を、クライアント ws の汎用エンベロープで行える」、R-002、B-001、B-002、Assumptions Q-004、Thread 657a8904-7d8f-45bf-b25b-da320850505b。ルート: 委任
- terminal の ack / detach command の ws・Tauri 一致: `ack_terminal_surface_output` と `detach_terminal_surface` を ws から呼んだ場合と Tauri invoke から呼んだ場合で、引数の意味・結果・backend の output ack と detach の作用が一致するようにする（例: 未知の `attachmentId` への ack で両経路の結果が同じになる）。根拠: R-002「クライアント ws から呼んだ command は、同じ command を同じ引数で Tauri invoke から呼んだ場合と同じ usecase の結果（成功値またはエラー）を返す」、B-002、Thread 730d0086-c2bb-48b6-b58d-8c7ac94a87de。ルート: 委任
- 連続 resize の受信順適用: 同じクライアント接続で同じ terminal へ resize を連続して送ったとき、受信順と異なる順で適用されず、PTY の最終寸法が最後に送った要求の寸法と一致するようにする。根拠: R-004、B-004「入力と大きさの変更が terminal に反映される」、Thread f276349d-a7a9-4079-ae20-c5b8244fa45c。ルート: 委任
- stream 自然終了時の attachment 解放: terminal の stream が exit 等で自然終了したとき、最後の frame の配送を保ったまま、その attachment の接続内登録・共有 attachment 枠・backend 側 attachment を解放し、終了済み attachment が上限枠を占有して新しい attach が `ATTACHMENT_LIMIT` にならないようにする。根拠: R-004、B-004、Thread 6dda6197-0427-4bc2-9450-6ffe125e0bc8。ルート: 委任
- output 描画完了に結び付いた credit 解放: output 以外の frame や output の非末尾 chunk の受信による ack が、先行する未描画 output の backend 出力 credit を解放しないようにし、output N の credit は N の描画完了後に解放されるようにする。根拠: R-005、B-005、R-008、Thread d065aed3-741d-4ae2-99de-a9006f9dddd9。ルート: 委任
- 未知・detach 済み attachment への遅延 ack の非波及: detach 済み・未知の attachment に対する stream の ack が届いても、同じクライアント接続上の未完了要求は reject されず、他の attachment の stream と push が継続し、クライアント ws が閉じないようにする。根拠: R-004、B-004「同じ接続で req/resp と push のやり取りが続けられる」、Thread 56980c07-2a81-4dd7-9ed8-dfcbd7fb1814。ルート: 委任
- 相関済み応答の変換失敗時の reject: `request_id` で相関した応答の result / error の値変換に失敗したとき、その要求の Promise が未完了のまま残らず reject されるようにする。根拠: R-001「成功時は結果、失敗時はエラーが返る」、B-001、Thread 83ec7b9c-b29f-4db7-b664-236aee1bafac。ルート: 委任
- 保留 command 上限の切断・再接続をまたぐ適用: 遅い command が未完了のまま切断・再接続を繰り返しても、未完了の client command 処理の総数が保留上限で抑えられ、上限を超えた要求には `REQUEST_LIMIT` 相当のエラーが返るようにする。受理済み command を切断で途中破棄せず完了まで実行する性質は保つ。根拠: Thread 88f0cfb5-2a03-4a3e-913d-64b9ff11850b。ルート: 委任
- 接続 listener だけの再接続テスト: push 購読がなく接続 listener（`onClientConnection`）だけを購読した状態で、初回接続失敗後と切断後のそれぞれに再接続が行われ listener へ true が通知されることを確認するテストを追加し、再接続条件から接続 listener を外すとそのテストが失敗するようにする。根拠: R-012、B-013、B-014、Thread 7c015674-637f-4ad6-80da-cbaa63822840。ルート: 委任
- frontend テストでの入口の区別: 移行対象の操作が `invokeClient`（クライアント ws）へ、起動失敗系 2 本（`get_application_startup_outcome`、`quit_after_startup_failure`）が Tauri invoke へ送られることを、テストで入口ごとに区別して検証し、移行対象を Tauri invoke へ戻す変更や起動失敗系 2 本をクライアント ws へ変える変更で該当テストが失敗するようにする。E2E 経路の全置換は含めない。根拠: R-010、R-011、B-011、B-012、Thread 61e741a5-5396-43c2-b4e2-b08f762d2fe0。ルート: 委任
- 変更系・複合引数 command の parity 検証: 今回追加した変更系・複合引数 command（例: `create_worktree` / `remove_worktree`、staging 系、workflow の start / abort / stop / resume）の protocol 往復または dispatch 経由の配線を、実引数での成功値・usecase エラーの parity で検証し、引数の受け渡しやエラー配線を誤らせると protocol parity テストが失敗するようにする。全 command 一律のテスト義務は課さない。根拠: R-001、R-002、B-001、B-002、Thread bdfc10be-3f28-4643-99e1-ecac13d910b1。ルート: 「固定するルート」の検証方法（design-01 から維持）
- クライアント ws 専用 wire 型の配置: クライアント ws だけで使う Envelope / CommandRequest / CommandResponse / Stream / Ack 等の wire 型と変換を `adaptor/protocol/` に置かず、`docs/architecture/CONTROLLER.md` の local API 用 req/resp 型の配置（`controller/api/protocol.rs`）に一致させる。根拠: R-003 の実装、`docs/architecture/CONTROLLER.md`「local API だけで使うリクエスト／レスポンス型は controller/api/protocol.rs に置き、adaptor/protocol/ へは上げない」、Thread fcbd3ef1-94e5-4fd1-910a-d3ef2ef230f0。ルート: 委任
- ws 接続単位の terminal 処理の配置: ws 接続単位の attachment 表・frame 分割・ack window 等の local API 専用処理を Tauri 入口（`controller/command/`）配下に置かず、local API 入口がそれを `controller/command/` 配下から import しないようにする。根拠: R-004 の実装、`docs/architecture/CONTROLLER.md`「原則」の入口分離（`controller/command/` は Tauri コマンド、`controller/api/` は loopback HTTP local API）、Thread 44866668-eabd-4091-bb7a-fe66595f7e7e。ルート: 委任
- 共有 dispatch の usecase 直接呼び出し: 今回追加した共有 dispatch の各 handler が、Tauri command 関数と `AppHandle` の managed state を経由せず共通の usecase 操作を呼ぶようにし、usecase を通らない外部アクセス（例: watcher の `FileWatcherManager` 直呼び、external_editor の設定 gateway 直呼び）を ws 共通経路へ取り込まないようにする。Tauri 入口と local API 入口が同じ usecase を呼ぶ薄い入口として分離され、R-002 の parity は維持する。根拠: 「固定するルート」の判断①（design-01 から維持）「req/resp は…エンベロープ…を経由して usecase 共有 dispatch へ渡す」、`docs/architecture/CONTROLLER.md`「どちらの入口も同じ Usecase を呼ぶ」、Thread eb55012c-92e4-445d-9d04-ab7f8fd2c5e2。ルート: 「固定するルート」の判断①
- command 引数・結果と push payload の `.proto` 定義: 被覆 command の必須引数・結果と push payload の型を `.proto` に定義し、Rust と TypeScript の型をそこから生成して使い、command 契約の正を手書きの Rust Args や `JsonObject`・任意型への cast ではなく `.proto` にする。R-003（バイナリ frame）は維持する。根拠: 「固定するルート」の判断①（design-01 から維持）「`.proto` を protocol の正とし、Rust / client の型はそこから生成する」「push も proto message とする」、Thread d01766f0-7b64-47ca-9f41-acf05879dbba。ルート: 「固定するルート」の判断①

## 固定するルート

- 今周に新たに固定する実装上の指定なし。
- design-01 の「固定するルート」をすべて維持する（判断①、判断⑧、判断⑦、土台、対象ドメインの単位、terminal_surface、検証方法）。

## 変えないもの

- design-01 の「変えないもの」を維持する（CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口と結果）。

## 未確定・リスク

`[DEFERRED]` で人間へ渡した件はない。「自動判断: 未決」のまま残した要求はない。この周までの自動判断（`requirements.md` の Assumptions）はいずれも人間の確認を経ていない。

### 自動判断

- Q-001: クライアント ws のエンベロープ（req/resp、push、stream）をこの変更で Protocol Buffers バイナリ frame に置き換える。A1（PR #1768）と `docs/specs/issues-1199/` は JSON text frame を前提としている。
- Q-002: desktop の該当 UI 機能の呼び出しをこの変更でクライアント ws 経由へ差し替える。Issue #1201（A-flip）は呼び出し側の差し替えを A-flip の作業に挙げている。
- Q-002 に伴う判断: `get_application_startup_outcome` と `quit_after_startup_failure` の desktop からの呼び出しは Tauri invoke のまま維持する。
- Q-003: telemetry / watcher / application_lifecycle は登録済みの全 command を被覆し、UI shell に残す分の線引きは A-flip で決める。
- Q-004: desktop から呼び出す箇所がない登録済み command 34 本を、他の command と同様にクライアント ws へ被覆する。
- Q-005: Tauri Channel fallback の削除後、クライアント ws の切断・接続失敗の後に接続が確立した時点で terminal を再 attach・再同期する。attach 要求にエラー応答が返った場合の挙動は定めていない。
- `get_terminal_stream_endpoint` を `/v1/terminal` route とともに削除する。
- terminal の Tauri Channel fallback の削除をこの変更で行う。Issue #1201 も同じ撤去を A-flip の作業に挙げている。
- 作業単位を Issue #1200 の対象ドメインすべてとする。
