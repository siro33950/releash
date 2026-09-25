# Design 08

## 開始状態

- 差分の基準: base ブランチ `main`、派生点は `8611683a`。branch `feat/issues/1888` の HEAD は `8748d4ca`（`refactor(state): terminal の配信を共通の状態購読に統合する (#1888)`）。design-07 の実装は未コミットの変更として作業ツリーに入っており、この状態を今周の開始状態とする。対象は `src-tauri/src/domain/state_subscription/mod.rs`、`src-tauri/src/domain/terminal_surface/gateway.rs`、`src-tauri/src/domain/terminal_surface/entities/terminal_surface_input_ingress.rs`、`src-tauri/src/usecase/state_subscription.rs`、`src-tauri/src/usecase/state_subscription/terminal.rs`、`src-tauri/src/adaptor/gateway/terminal_surface/{event_hub.rs,runtime_gateway_impl.rs,event_fault_relay.rs}` と各テスト、`tests/helpers/tauri-mock.ts`、`tests/client-streams.spec.ts` である。
- 直前の Design は `docs/specs/issues-1888/design-07.md`。
- design-07 の「変える部分」2 件は開始状態で満たされている。`register_delta`（`src-tauri/src/domain/state_subscription/mod.rs:328-361`）が既存購読の送り待ちを捨てなくなり、`publish_delta`（同:406-452）は overflowed な購読へ積まず、`next`（同:661-690）は overflowed でも送り待ちを先に出し切り、出し切った時点で対象が消えていれば購読を外す。これにより終了した terminal が直ちに作り直されても、送り待ちの Exit が購読中の client へ届く。`Subscriptions::is_subscribed`（同:362-369）と `TerminalSurfaceStateSink::remove`（`src-tauri/src/usecase/state_subscription/terminal.rs:240-263`）の `inputs.retain` が購読の残る client の `terminal_inputs` を維持し、`remove_surface`（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:872-891`）は subscribed が true なら `input_ingress` を消さず、`refresh_terminal`（terminal.rs:128-172）は `snapshot_requests` に載る client を `reset_output` する。
- この周までに解消・見送りとなった Thread: design-07 の周に `[FIX_POLICY]` が付いた 2 件（`cdefe8d1-ef5c-4ccf-b017-814c6c438a55`、`99104ae5-5dca-4a78-96b6-b43882bfdee0`）は resolve 済みで open に残っていない。`[DEFERRED]` で人間へ渡した件は無い。
- 今周の open Thread は 2 件で、いずれも `[FIX_POLICY]` が付いた状態で open のまま残る。`5219f238-204b-43eb-bcd9-cf5beee55c9e`、`78141b07-aec4-47ce-9607-8a972e4b325a`。
- Requirements・Behavior は今周に変更していない。R-001〜R-023 と B-001〜B-028 は欠番・重複なく並び、対応表は R-001〜R-023 を全て記載する。`requirements.md` の Assumptions / Open Questions は「なし」のままで、自動判断で補った箇所は無い。

## 変える部分

- 送り待ちが空の client も、同じ owner の terminal の作り直しをまたいで購読を保つ: `unregister`（`src-tauri/src/domain/state_subscription/mod.rs:205-218`）は送り待ちが空の購読を `client.subscriptions` と `order` から削除する。作り直しの `initialize`（`src-tauri/src/usecase/state_subscription/terminal.rs:224-238`）が呼ぶ `register_delta`（mod.rs:328-361）は既存の購読だけを overflowed にするため、削除された client は `snapshot_requests`（同:501-516）にも `active_targets`（同:609-614）にも戻らない。あわせて `TerminalSurfaceStateSink::remove`（terminal.rs:240-263）の `inputs.retain` は `is_subscribed` が false になったその client の `terminal_inputs` を消し、`remove_surface`（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:872-891`）は subscribed が false なら `input_ingress` を消す。`get_or_spawn_with_process`（`src-tauri/src/usecase/terminal_surface/spawn_usecase.rs:212-240`）は client の送り待ちの状態に関係なく終了済み terminal を削除して作り直すため、最新まで追随した通常の client で到達する。結果、stream は生きたまま対象 terminal だけ更新されず、入力は `StaleAttachment`（`src-tauri/src/domain/terminal_surface/entities/terminal_surface_input_ingress.rs:74-77`）となり、`terminal_processed`（terminal.rs:189-216）は Missing を返す。送り待ちが空の状態で作り直されても、明示的な再購読なしに新しい terminal の最初の状態と以後の差分が届き、その client の入力が terminal へ届き、処理済みの量の通知が Missing にならない形にする。根拠: R-002「terminal の変化は差分として届き、UI は受け取った差分を画面へ反映する。」／B-002、R-021「terminal への入力は、購読へ移した後も送れる。」／B-024、R-008「送る量の制御の通知単位は daemon が定め、terminal の購読の最初の状態として client へ届く。UI は、terminal の出力を処理し終えた量を積み、受け取った通知単位に達するたびに、その量を daemon へ知らせる。」／B-010。Thread `5219f238-204b-43eb-bcd9-cf5beee55c9e`。ルート: 委任。
- lifecycle の削除が一時停止中の出力元を解放することを検証できるようにする: `TerminalSurfaceEventHub::remove`（`src-tauri/src/adaptor/gateway/terminal_surface/event_hub.rs:166-177`）は `state_sink.remove` の結果にかかわらず `release_output` を呼ぶが、この経路を通るテストが無い。`event_hub_test.rs` の 4 件は `unsubscribe_output` 経由（同:79-94）と `release_output` の直接呼び出し（同:97-117）だけを通り、`remove` を呼ばない。`runtime_gateway_impl_test.rs:1144` の `remove_surface` は出力を高水位へ上げる前（同:1178 の publish、1187 の `wait_output` より前）に呼ばれるため、一時停止中の `remove` を通らない。`remove` から `release_output` の呼び出しを外すと失敗するテストがあり、一時停止中の出力元が `remove` 経由で解放されることを検証できる形にする。根拠: `docs/architecture/TEST.md` の層別の表「| `adaptor/gateway/` | **必須** | 外部システムとの境界、モデル変換の検証 |」。対象の経路が支える振る舞いは R-014「ある terminal の出力元が送る量の制御で一時停止している間も、他の terminal の出力は届き続ける。」／B-017 と R-016「ある terminal の出力元が送る量の制御で一時停止している間も、その terminal の寸法の変更は受け付けられる。」／B-019。Thread `78141b07-aec4-47ce-9607-8a972e4b325a`。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。今周に人間が新しく固定した実装上の指定は無い。D1〜D7（design-01）と D8〜D11（design-02）は今周も維持し、範囲・粒度・理由は各 Design の記載のまま変えない。design-02 で列挙した委任の範囲（proto の具体的な形、非同期処理の組み方、コードの配置、テストの置き方）も同じまま維持し、今周の 2 件はいずれもその範囲に入る。

## 変えないもの

design-07 の「変えないもの」を全て維持する。今周に人間が新しく明示した維持対象は無い。

## 未確定・リスク

なし。今周に自動判断した箇所は無く、`requirements.md` の Assumptions / Open Questions は「なし」のままである。未決のまま残した要求と、`[DEFERRED]` で人間へ渡した件も無い。
