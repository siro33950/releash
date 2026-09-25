# Design 07

## 開始状態

- 差分の基準: base ブランチ `main`、派生点は `8611683a`。branch `feat/issues/1888` の HEAD は `8748d4ca`（`refactor(state): terminal の配信を共通の状態購読に統合する (#1888)`）。design-06 の実装は未コミットの変更として作業ツリーに入っており、この状態を今周の開始状態とする。対象は `src-tauri/src/domain/state_subscription/mod.rs`、`src-tauri/src/domain/terminal_surface/gateway.rs`、`src-tauri/src/domain/terminal_surface/entities/terminal_surface_input_ingress.rs`、`src-tauri/src/usecase/state_subscription.rs`、`src-tauri/src/usecase/state_subscription/terminal.rs`、`src-tauri/src/adaptor/gateway/terminal_surface/{event_hub.rs,runtime_gateway_impl.rs,event_fault_relay.rs}` と各テスト、`tests/helpers/tauri-mock.ts`、`tests/client-streams.spec.ts` である。
- 直前の Design は `docs/specs/issues-1888/design-06.md`。
- design-06 の「変える部分」4 件は開始状態で満たされている。`tests/helpers/tauri-mock.ts` から `input_unavailable` の分岐が消えている。`TerminalSurfaceStateSink::remove`（`src-tauri/src/usecase/state_subscription/terminal.rs:244`）と `Subscriptions::unregister`（`src-tauri/src/domain/state_subscription/mod.rs:205`）、`Subscriptions::next`（同:655）の購読解放により、terminal の lifecycle 終了時に配信の経路と差分履歴が解放される。`start_terminal`（terminal.rs:46-62）から `register_delta` の呼び出しが無くなり、購読開始が古い世代の登録へ戻らない。`tests/helpers/tauri-mock.ts` の `StartStateSubscription` が terminal の再購読で新しい入力 attachment を登録する。
- この周までに解消・見送りとなった Thread: design-06 の周に `[FIX_POLICY]` が付いた 4 件（`2c86a287-bc3e-4044-9410-692b0635b4b5`、`ca18fda8-3fa2-495b-a365-8721f178dd7b`、`7a9b8310-800b-46fb-a8c5-596aa3a8cb1c`、`08f9f765-3c48-483e-95c9-4bbc1689d69f`）は resolve 済みで open に残っていない。`[DEFERRED]` で人間へ渡した件は無い。
- 今周の open Thread は 2 件で、いずれも `[FIX_POLICY]` が付いた状態で open のまま残る。`cdefe8d1-ef5c-4ccf-b017-814c6c438a55`、`99104ae5-5dca-4a78-96b6-b43882bfdee0`。
- Requirements・Behavior は今周に変更していない。R-001〜R-023 と B-001〜B-028 は欠番・重複なく並び、対応表は R-001〜R-023 を全て記載する。`requirements.md` の Assumptions / Open Questions は「なし」のままで、自動判断で補った箇所は無い。

## 変える部分

- 同じ owner の terminal を作り直した後も、購読を続けている client の入力と処理済みの量の通知を働かせる: `TerminalSurfaceStateSink::remove`（`src-tauri/src/usecase/state_subscription/terminal.rs:244-262`）が対象の `terminal_inputs` を全て削除し、`remove_surface`（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:872-891`）と `remove_runtime`（同:1095-1100）が `input_ingress` を削除する。作り直し後の `initialize`（terminal.rs:225-242）が `register_delta`（`src-tauri/src/domain/state_subscription/mod.rs:330-363`）で既存購読を overflowed にし、その購読対象は `snapshot_requests`（同:495）に載るが、`refresh_terminal`（terminal.rs:128）は `terminal_inputs` に残る client だけを `reset_output` する。削除済みのため対象が空になり、`subscribe_output` による attachment の再登録も `terminal_inputs` への再挿入も起きない。その結果、購読を続けている client の入力は旧 attachment id のまま `StaleAttachment`（`src-tauri/src/domain/terminal_surface/entities/terminal_surface_input_ingress.rs:66-79`）となり、`terminal_processed`（terminal.rs:189-210）は Missing を返す。明示的な再購読を行わずに入力が新しい terminal へ届き、処理済みの量の通知が Missing にならない形にする。根拠: R-021「terminal への入力は、購読へ移した後も送れる。」／B-024、および R-008「送る量の制御の通知単位は daemon が定め、terminal の購読の最初の状態として client へ届く。UI は、terminal の出力を処理し終えた量を積み、受け取った通知単位に達するたびに、その量を daemon へ知らせる。」／B-010。Thread `cdefe8d1-ef5c-4ccf-b017-814c6c438a55`。ルート: 委任。
- 終了した terminal が同じ owner で直ちに作り直されても、終了時点で購読していた client へ終了を届ける: `unregister`（`src-tauri/src/domain/state_subscription/mod.rs:205-221`）は送り待ちが残り overflowed でない購読を消さないため、送り待ちの Exit は `remove` の後も購読に残る。一方 `register_delta`（同:330-363）は同じ購読対象の既存購読の pending を clear して overflowed にする。`get_or_spawn_with_process`（`src-tauri/src/usecase/terminal_surface/spawn_usecase.rs:220-236`）は終了済み terminal の `remove_surface` / `remove_runtime` の直後に同じ session_key で作り直しへ進み、`wait_runtime_output_drain`（runtime_gateway_impl.rs:1079-1093）は runtime 側の出力の掃き出しを待つだけで client の stream が取り出すまでは待たない。したがって作り直しが先行すると、送り待ちの Exit が破棄され購読中の client へ終了が届かない。根拠: R-003「terminal の購読では、最初に現在の状態（再現する画面、その寸法、終了の有無と終了コード）が届き、以後は出力、寸法の変更、終了が届く。」／B-004。Thread `99104ae5-5dca-4a78-96b6-b43882bfdee0`。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。今周に人間が新しく固定した実装上の指定は無い。D1〜D7（design-01）と D8〜D11（design-02）は今周も維持し、範囲・粒度・理由は各 Design の記載のまま変えない。design-02 で列挙した委任の範囲（proto の具体的な形、非同期処理の組み方、コードの配置、テストの置き方）も同じまま維持し、今周の 2 件はいずれもその範囲に入る。

## 変えないもの

design-06 の「変えないもの」を全て維持する。今周に人間が新しく明示した維持対象は無い。

## 未確定・リスク

なし。今周に自動判断した箇所は無く、`requirements.md` の Assumptions / Open Questions は「なし」のままである。未決のまま残した要求と、`[DEFERRED]` で人間へ渡した件も無い。
