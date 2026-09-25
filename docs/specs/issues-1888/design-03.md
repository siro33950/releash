# Design 03

## 開始状態

- 差分の基準: base ブランチ `main`、派生点は `9194bd57`（branch `feat/issues/1888`）。
- 直前の Design は `docs/specs/issues-1888/design-02.md`。
- design-02 の「変える部分」は作業ツリーの未コミットの変更として全て実装済みである。`proto/client.proto` から `SubscribeTerminalSurfaces`、`AckTerminalSurfaceOutput`、`GetTerminalSurface`、`AttachTerminalSurface` / `DetachTerminalSurface` が消え、`GetOrSpawnTerminalV1` は `session_key` だけになっている（同 `:1112-1115`）。`attach_lock` と terminal 固有の件数の上限は `src-tauri/src/` に残っていない。送る量の制御の 3 つの値は `src-tauri/src/domain/terminal_surface/value_objects/output_flow_control.rs:3-5` が所有する。terminal は購読の差分として配信される（`src-tauri/src/usecase/state_subscription/terminal.rs`）。
- この周までに解消・見送りとなった Thread: design-02 の周に `[FIX_POLICY]` が付いていた `d5046d06-163d-45d6-aa4f-ccf41acd6f8a`（送る量の制御の通知単位の所有者）と `92a33786-cca0-4bd5-bb28-2fde6fe92617`（terminal が作り直されたときの版番号の境目）は design-02 の D8 および「変える部分」へ反映済みで、open Thread に残っていない。`[REJECTED]` と `[DEFERRED]` で閉じた Thread は無い。
- 今周の open Thread は 2 件で、どちらも修正と決まり `[FIX_POLICY]` が付いた状態で open のまま残る。`534f56b4-a8ac-4cb5-83a4-c4478d67ce8e`（terminal の状態配線の二重実行）、`8442f95a-335b-47c5-b0b2-d9b079b4348d`（Resize と InputUnavailable の差分変換の検証不足）。
- Requirements・Behavior は今周に変更していない。要求 R-001〜R-022 と受入条件 B-001〜B-027 の対応表に欠落・矛盾は無く、自動判断で補った箇所も無い。

## 変える部分

- terminal の状態配線の所有箇所を一つにする: `StateSubscriptionUsecase::with_terminal`（`src-tauri/src/usecase/state_subscription.rs:86-92`）による `connect_state` が、daemon の本番構成で `src-tauri/src/adaptor/controller/daemon.rs:417-418` と `src-tauri/src/adaptor/controller/api/client.rs:102-109` の二箇所から同じ `StateSubscriptionUsecase` と同じ `TerminalSurfaceApplication` に対して実行され、`set_state_sink` と `gateway.list_summaries()` 全件の `initialize`（`src-tauri/src/usecase/terminal_surface/application.rs:108-115`）が二度走る状態を、一度だけ実行される形にする。根拠: Thread `534f56b4-a8ac-4cb5-83a4-c4478d67ce8e`。ルート: 委任。
- Resize と InputUnavailable の差分変換を usecase 層のテストで検証する: `src-tauri/src/usecase/state_subscription/terminal.rs:288-324` が `TerminalSurfaceEvent::Resize` を cols / rows を持つ差分へ、`InputUnavailable` を cause を持つ差分へ変換する分岐について、`src-tauri/src/usecase/state_subscription/terminal_test.rs:302-318` が版番号 4 件と `Exit` の `exit_code` しか検証しておらず別の variant・別の payload へ誤変換されても通る状態を、誤変換で失敗する検証がある形にする。根拠: B-004「GIVEN client が terminal を購読していて、最初の状態を受け取っている / WHEN 出力、寸法の変更、終了、入力の受付不可のいずれかが起きる / THEN その変化が届く」（対応要求 R-003）、Thread `8442f95a-335b-47c5-b0b2-d9b079b4348d`。ルート: 委任。

## 固定するルート

今周に新しく固定した実装上の指定は無い。D1〜D11 は design-01・design-02 で固定したものを今周も維持し、範囲・粒度・理由は各 Design の記載のまま変えない（D1〜D7 は design-01、D8〜D11 は design-02）。design-02 で列挙した委任の範囲も同じまま維持する。今周の 2 件の変更は、いずれもその委任の範囲（非同期処理の組み方、コードの配置、テストの置き方）に入る。

## 変えないもの

design-02 の「変えないもの」を全て維持する。今周に人間が新しく明示した維持対象は無い。

## 未確定・リスク

なし。今周に自動判断した箇所は無く、`requirements.md` の Assumptions / Open Questions は「なし」のままである。未決のまま残した要求と、`[DEFERRED]` で人間へ渡した件も無い。
