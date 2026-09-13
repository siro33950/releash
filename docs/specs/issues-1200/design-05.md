# Design 05

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `28f0eeed`。作業ブランチは `feat/issues/1200`。
- 直前の Design は `docs/specs/issues-1200/design-04.md`。開始状態は、design-01〜design-04 の周で実装した未コミットの変更を含む現在の作業ツリーである。
- Requirements・Behavior はこの周で変更していない。R-001〜R-014 と B-001〜B-016 は対応表で欠落なく対応している。
- この周までに解消した Thread: design-04 の開始状態に挙げた 20 件に加え、058bbf38-6661-4bd4-b84b-ff3d417f4d5b、308cacd0-135f-4199-81af-211b1df61a22、86b3b120-74d5-42bf-8010-e25a92fc697a、0dd55b97-100b-4d7c-bcaf-c7d17c6fd7ba、163eb233-23b7-4acb-b568-1e8793fd036c、7c9df8e1-3874-4131-906a-fd549981a520、d01766f0-7b64-47ca-9f41-acf05879dbba、4769fbe8-7edd-419b-933f-bf73d245ab8d。見送りとなった Thread はない。
- open Thread は 95748435-597e-4c0e-9d19-0c3bf1644cea の 1 件で、`[FIX_POLICY]` 付きである。design-04 の変える部分「同じ worktree への連続した workspace state 保存の受信順適用」の項目で、`[STILL_OPEN]` により再接続を挟む経路が未充足と示されている。現在の保存の待ち合わせはクライアント ws 接続ごとに初期化され、接続間で順序を引き継がない。

## 変える部分

- 同じ worktree への連続した workspace state 保存の受信順適用（未充足分）: 同じ worktree への `save_workspace_state` を続けて送った場合、同じクライアント ws 接続内に限らず、切断・再接続を挟んで別の接続から送った場合も、すべての保存の完了後に backend が保持し永続化される状態が最後に送った要求の状態になり、再起動後にその状態が復元されるようにする。受理済み command を切断後も完了まで実行する性質（解消済み Thread 88f0cfb5-2a03-4a3e-913d-64b9ff11850b の方針）は変えない。根拠: R-010「desktop の UI 機能が対象ドメインの command を呼ぶ操作は、クライアント ws 経由の要求で行われ、変更前と同じ結果になる」、B-011「画面には変更前と同じ結果が反映される」、Thread 95748435-597e-4c0e-9d19-0c3bf1644cea。ルート: 委任

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
- Q-005: Tauri Channel fallback の削除後、クライアント ws の切断・接続失敗の後に接続が確立した時点で terminal を再 attach・再同期する。
- attach 要求へのエラー応答: クライアント ws 上の attach 要求にエラー応答が返った場合、desktop の terminal はその応答のエラーを terminal のエラーとして表示し、接続が続いている間の自動再試行は要求しない（R-013）。
- 監視開始の要求への応答を受け取れなかった場合: 応答をクライアントが受け取る前に接続が切断された監視開始の要求で開始された監視を backend に残さない（R-014）。切断時に接続上の全監視を停止すること、接続の確立後に desktop が監視を開始し直すことは要求しない。
- `get_terminal_stream_endpoint` を `/v1/terminal` route とともに削除する。
- terminal の Tauri Channel fallback の削除をこの変更で行う。Issue #1201 も同じ撤去を A-flip の作業に挙げている。
- 作業単位を Issue #1200 の対象ドメインすべてとする。
