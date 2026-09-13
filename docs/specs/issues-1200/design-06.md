# Design 06

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `28f0eeed`。作業ブランチは `feat/issues/1200`。
- 直前の Design は `docs/specs/issues-1200/design-05.md`。開始状態は、design-01〜design-05 の周で実装した未コミットの変更を含む現在の作業ツリーである。
- Requirements・Behavior はこの周で変更していない。R-001〜R-014 と B-001〜B-016 は対応表で欠落なく対応している。
- この周までに解消した Thread: design-05 の開始状態に挙げた 28 件に加え、95748435-597e-4c0e-9d19-0c3bf1644cea。見送りとなった Thread はない。
- open Thread は 00bb37ab-af3e-464d-bd14-47695d4c9e00 の 1 件で、`[FIX_POLICY]` 付きである。現在の desktop の workspace state 保存は、クライアント ws への保存要求の完了を待たずに未保存マークを外しており、接続失敗・切断で要求が backend へ届かなかった場合もその状態を未保存として保持しない。

## 変える部分

- クライアント ws の接続失敗・切断で完了を確認できなかった workspace state 保存の未保存保持: クライアント ws の接続失敗または切断により backend へ届かなかった（完了を確認できなかった）workspace state の保存は未保存のまま保持され、接続回復後に新たな状態更新がなくても、後続の flush または unmount で最新の状態が保存され、再起動後にその状態が復元されるようにする。同じ worktree への連続した保存の受信順適用（解消済み Thread 95748435-597e-4c0e-9d19-0c3bf1644cea の結果）は変えない。根拠: R-010「desktop の UI 機能が対象ドメインの command を呼ぶ操作は、クライアント ws 経由の要求で行われ、変更前と同じ結果になる」、B-011「画面には変更前と同じ結果が反映される」、Thread 00bb37ab-af3e-464d-bd14-47695d4c9e00。ルート: 委任

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
