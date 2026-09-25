# Design 04

## 開始状態

- 差分の基準: base ブランチ `main`、派生点は `9194bd57`（branch `feat/issues/1888`）。
- 直前の Design は `docs/specs/issues-1888/design-03.md`。
- design-03 の「変える部分」のうち、Resize と InputUnavailable の差分変換の usecase 層での検証は実装済みである（`src-tauri/src/usecase/state_subscription/terminal_test.rs:302-335`）。terminal の状態配線を一度だけにする件は未達のままである。`src-tauri/src/adaptor/controller/terminal_surface_runtime.rs:94-98` が内部の `StateSubscriptionUsecase` を `with_terminal` で組み立てて 1 回目の `connect_state` を行い、`src-tauri/src/adaptor/controller/api/client.rs:102-108` が本番の `StateSubscriptionUsecase` へ同じ `TerminalSurfaceApplication` を接続して 2 回目を行う。
- 今周に Requirements・Behavior を変更した。R-003 から入力の受付不可の配信を外して縮小し、B-004 の `WHEN` を「出力、寸法の変更、終了のいずれかが起きる」へ縮小した。R-023 と B-028 を追加し、対応表へ `| R-023 | B-028 |` を加えた。Scope / Non-goals に、変更する対象として入力が受け付けられなかったことを購読で配信する経路の削除を、変更しない対象として入力が受け付けられなかったときの画面側の扱いを加えた。
- この周までに解消・見送りとなった Thread: `8442f95a-335b-47c5-b0b2-d9b079b4348d`（Resize と InputUnavailable の差分変換の検証不足）は resolve 済みである。`28af1bfc-12c5-43fb-8048-14b485512079`（`TerminalSurfaceApplication::get` が未使用）は `[REJECTED]` で resolve 済みである。`[DEFERRED]` で人間へ渡した件は無い。
- 今周の open Thread は 4 件で、いずれも修正と決まり `[FIX_POLICY]` が付いた状態で open のまま残る。`44040ee0-5f9f-4c49-b187-8648cdf6eb7b`、`db6a0b9b-c1dd-4333-8cd5-ccfd45f3495b`、`5c1b7fc7-c178-4501-9eef-3ebfdfec1004`、`534f56b4-a8ac-4cb5-83a4-c4478d67ce8e`。

## 変える部分

- 入力の受付不可を購読の配信経路から外す: terminal の購読の差分として入力の受付不可が届く経路を無くし、入力が受け付けられなかったことは入力の呼び出しの応答だけで伝わる形にする。購読経路のためだけに残るコードは残さない。現在の残存箇所は `src-tauri/src/usecase/state_subscription/terminal.rs:266,321-325`、`src-tauri/src/adaptor/protocol/client/mod.rs:90-93`、`src-tauri/src/adaptor/protocol/terminal.rs:145,186-191`、`proto/client.proto:2732,2761`、`src/lib/clientProtocol.ts:76`、`src/hooks/useTerminal.ts:505` である。根拠: R-023「terminal の購読では、入力が受け付けられなかったことは届かない。入力が受け付けられなかったことは、入力の呼び出しの応答で伝わる。」、B-028、および R-022・B-025。Thread `44040ee0-5f9f-4c49-b187-8648cdf6eb7b`。ルート: 委任。
- `ClientApiDeps` から読み出されない field を無くす: `src-tauri/src/adaptor/controller/api/client.rs:38` の `terminal` field は同 `:109` の代入だけで、`src-tauri/src/` 全体に読み出しが無い。派生点 `9194bd57` では同ファイルの `DetachTerminalSurface` の分岐が読んでいたため、この field は本変更で未使用になった。`with_terminal` は `state_subscriptions` への接続に必要な範囲だけを扱う形にする。根拠: R-022「この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。」、B-025。Thread `db6a0b9b-c1dd-4333-8cd5-ccfd45f3495b`。ルート: 委任。
- terminal 専用の配信経路を本番ビルドから無くす: `src-tauri/src/adaptor/controller/terminal_surface_runtime.rs:94-98` の terminal 専用の `StateSubscriptionUsecase` と、同 `:213-269` の terminal の状態・変化だけを `TerminalSurfaceWireAttachment` として返す公開 `subscribe` が、本番 daemon が使う共通の購読経路とは別の配信経路として本番ビルドに残っている。terminal の配信を共通の購読経路の一つだけにし、この経路を使っている acceptance test（`src-tauri/tests/terminal_surface_acceptance.rs`、`src-tauri/tests/workflow_control_plane_acceptance_test.rs`、`src-tauri/src/agent_session_tui_acceptance.rs`）も共通の購読経路で terminal を受け取る形にする。根拠: Thread `5c1b7fc7-c178-4501-9eef-3ebfdfec1004`（規約 `AGENTS.md:49,51,205,206`）。ルート: 委任。
- terminal の状態配線を一度だけにする: 同じ `TerminalSurfaceApplication` に対する `connect_state`（`src-tauri/src/usecase/terminal_surface/application.rs:108-115` の `set_state_sink` と `gateway.list_summaries()` 全件の `initialize`）が、daemon の本番構成で `terminal_surface_runtime.rs:94-98` と `api/client.rs:102-108` の二箇所から実行される状態を、一つの組み立て箇所が所有して一度だけ実行される形にする。根拠: Thread `534f56b4-a8ac-4cb5-83a4-c4478d67ce8e`（規約 `AGENTS.md:49-51,205,207`）。ルート: 委任。

## 固定するルート

今周に新しく固定した実装上の指定は無い。D1〜D11 は design-01・design-02 で固定したものを今周も維持し、範囲・粒度・理由は各 Design の記載のまま変えない（D1〜D7 は design-01、D8〜D11 は design-02。D8 は design-02 が design-01 の D8 を置き換えたもの）。design-02 で列挙した委任の範囲も同じまま維持する。今周の 4 件の変更は、いずれもその委任の範囲（proto の具体的な形、非同期処理の組み方、コードの配置、テストの置き方）に入る。

## 変えないもの

design-02 の「変えないもの」を全て維持する。今周に人間が新しく明示した維持対象は次の 2 点である。

- terminal の版番号の規則（R-004、B-005）。理由: 入力の受付不可を購読から外す判断は版番号の規則を動かさないため。
- 入力が受け付けられなかったときの画面側の扱い（エラーの通知と terminal のつなぎ直し）。理由: 入力の呼び出しの失敗の応答に対する既存の扱いをそのまま使うため。

## 未確定・リスク

なし。今周に自動判断した箇所は無く、`requirements.md` の Assumptions / Open Questions は「なし」のままである。未決のまま残した要求と、`[DEFERRED]` で人間へ渡した件も無い。
