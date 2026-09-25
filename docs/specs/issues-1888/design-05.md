# Design 05

## 開始状態

- 差分の基準: base ブランチ `main`、派生点は `9194bd57`（branch `feat/issues/1888`）。
- 直前の Design は `docs/specs/issues-1888/design-04.md`。
- design-04 の「変える部分」4 件は、いずれも作業ツリーの未コミットの変更として実装済みである。
    - 入力の受付不可を購読の配信経路から外す: `proto/client.proto:2730-2738` の `TerminalEvent` は `reserved 5` / `reserved "input_unavailable"` となり、`oneof` は snapshot / output / resize / exit の 4 つだけである。`src-tauri/src/` と `src/` に購読経路の入力の受付不可は残っていない。`TerminalSurfaceInputUnavailableCause`（`src-tauri/src/domain/terminal_surface/gateway.rs:47-60`）は入力の呼び出しが返す失敗の理由としてだけ使われる（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:908-917`）。
    - `ClientApiDeps` から読み出されない field を無くす: `src-tauri/src/adaptor/controller/api/client.rs:34-41` に `terminal` field は無く、`with_terminal`（同 `:100-108`）は `state_subscriptions` への接続だけを行う。
    - terminal 専用の配信経路を本番ビルドから無くす: `src-tauri/src/adaptor/controller/terminal_surface_runtime.rs` に `StateSubscriptionUsecase` と terminal 専用の公開 `subscribe` は無い。terminal を受け取る acceptance は共通の購読経路を使う（`src-tauri/src/terminal_subscription_acceptance.rs:42`、`src-tauri/src/lib.rs:13`）。
    - terminal の状態配線を一度だけにする: 本番構成で `connect_state` を呼ぶ `with_terminal`（`src-tauri/src/usecase/state_subscription.rs:86-92`）の実行箇所は `src-tauri/src/adaptor/controller/api/mod.rs:45` から辿る `api/client.rs:100-108` の一つだけで、`src-tauri/src/adaptor/controller/daemon.rs` からは呼ばれない。
- この周までに解消・見送りとなった Thread: `913227b3-b61f-44ba-95f2-89a3bd0f1aea`（既に購読中の terminal を再購読したときの初期状態の配信）は不成立として `[REJECTED]` で resolve 済みである。design-04 の周に `[FIX_POLICY]` が付いていた `44040ee0-5f9f-4c49-b187-8648cdf6eb7b`、`db6a0b9b-c1dd-4333-8cd5-ccfd45f3495b`、`5c1b7fc7-c178-4501-9eef-3ebfdfec1004`、`534f56b4-a8ac-4cb5-83a4-c4478d67ce8e` は design-04 の「変える部分」へ反映され、open に残っていない。`[DEFERRED]` で人間へ渡した件は無い。
- 今周の open Thread は 0 件であり、`[FIX_POLICY]` が付いた Thread は無い。
- Requirements・Behavior は今周に変更していない。要求 R-001〜R-023 と受入条件 B-001〜B-028 の対応表に欠落・矛盾は無く、自動判断で補った箇所も無い。

## 変える部分

なし。Requirements・Behavior の変更が無く、`[FIX_POLICY]` が付いた open Thread も無い。design-04 で挙げた 4 件は開始状態で既に満たされている。

## 固定するルート

固定する実装上の指定なし。今周に人間が新しく固定した実装上の指定は無い。D1〜D7（design-01）と D8〜D11（design-02）は今周も維持し、範囲・粒度・理由は各 Design の記載のまま変えない。

## 変えないもの

design-04 の「変えないもの」を全て維持する。今周に人間が新しく明示した維持対象は無い。

## 未確定・リスク

なし。今周に自動判断した箇所は無く、`requirements.md` の Assumptions / Open Questions は「なし」のままである。未決のまま残した要求と、`[DEFERRED]` で人間へ渡した件も無い。
