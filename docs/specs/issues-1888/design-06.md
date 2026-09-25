# Design 06

## 開始状態

- 差分の基準: base ブランチ `main`、派生点は `8611683a`。branch `feat/issues/1888` の HEAD は `8748d4ca`（`refactor(state): terminal の配信を共通の状態購読に統合する (#1888)`）で、作業ツリーに未コミットの変更は無い。design-05 までの実装はこの commit に入っている。
- 直前の Design は `docs/specs/issues-1888/design-05.md`。
- design-05 の「変える部分」は「なし」であり、design-04 までの 4 件（入力の受付不可を購読の配信経路から外す、`ClientApiDeps` から読み出されない field を無くす、terminal 専用の配信経路を本番ビルドから無くす、terminal の状態配線を一度だけにする）は開始状態で満たされている。
- この周までに解消・見送りとなった Thread: `913227b3-b61f-44ba-95f2-89a3bd0f1aea`（既に購読中の terminal を再購読したときの初期状態の配信）は `[REJECTED]` で resolve 済み、`8442f95a-335b-47c5-b0b2-d9b079b4348d`（Resize と InputUnavailable の差分変換の検証不足）は resolve 済み、`28af1bfc-12c5-43fb-8048-14b485512079`（`TerminalSurfaceApplication::get` が未使用）は `[REJECTED]` で resolve 済みである。design-04 の周に `[FIX_POLICY]` が付いていた 4 件（`44040ee0-5f9f-4c49-b187-8648cdf6eb7b`、`db6a0b9b-c1dd-4333-8cd5-ccfd45f3495b`、`5c1b7fc7-c178-4501-9eef-3ebfdfec1004`、`534f56b4-a8ac-4cb5-83a4-c4478d67ce8e`）は open に残っていない。`[DEFERRED]` で人間へ渡した件は無い。
- 今周の open Thread は 4 件で、いずれも `[FIX_POLICY]` が付いた状態で open のまま残る。`2c86a287-bc3e-4044-9410-692b0635b4b5`、`ca18fda8-3fa2-495b-a365-8721f178dd7b`、`7a9b8310-800b-46fb-a8c5-596aa3a8cb1c`、`08f9f765-3c48-483e-95c9-4bbc1689d69f`。
- Requirements・Behavior は今周に変更していない。要求 R-001〜R-023 と受入条件 B-001〜B-028 の対応表に欠落・矛盾は無く、自動判断で補った箇所も無い。

## 変える部分

- 購読の配信経路から外した入力の受付不可を test helper からも無くす: `tests/helpers/tauri-mock.ts` の terminal の event mock に `item.type === "input_unavailable"` の分岐（同ファイルの `__releashTerminalEvent` の中）が残っている。`src/lib/terminalSurfaceStream.ts` の `TerminalSurfaceStreamItem` は snapshot / output / resize / exit の 4 つだけで、`proto/client.proto` は `input_unavailable` を reserved としているため、この分岐は到達しない。根拠: R-022「この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。対象は Rust、TypeScript、proto、およびそれらだけを対象とするテストである。」、B-025「この変更で使われなくなったコードは残らない。」Thread `2c86a287-bc3e-4044-9410-692b0635b4b5`。ルート: 委任。
- terminal の lifecycle 終了時に配信の経路と差分履歴を解放する: `src-tauri/src/usecase/state_subscription/terminal.rs:233-237` の `TerminalSurfaceStateSink::initialize` が `terminal_routes`（`session_key` から購読対象への対応）へ挿入し購読対象を差分で登録する一方、対になる解放が無い。`terminal_routes` への `remove` は `src-tauri/src/` 全体に存在せず、`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:857-866` の `remove_surface` は state sink を呼ばない。`src-tauri/src/domain/state_subscription/mod.rs:570-581` の `release_inactive_snapshots` は `Delivery::Full` の history だけを clear するため、差分で進む terminal の履歴は残る。terminal を作成・削除するたびに保持状態が増え続けない形にする。根拠: `AGENTS.md`「アーキテクチャ原則 / 状態の所有者を明確にする」の「full-retention 設計を避ける。」および「レビュー観点」の「full-retention / full-recompute 経路を増やしていないか。」R-001〜R-005 は削除済み terminal の経路と差分履歴の保持を要求していない。Thread `ca18fda8-3fa2-495b-a365-8721f178dd7b`。ルート: 委任。
- 購読開始と runtime の作り直しの競合で登録が古い世代へ戻らないようにする: `src-tauri/src/usecase/state_subscription/terminal.rs:46-62` は `get_summary` を `with_output_order` より前に行い、取得済みの `runtime_generation` の区間で無条件に `register_delta` する。`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:841-849` の `with_output_order` は指定した runtime が既に消えていれば lock を取らずに closure を実行し、同 `:766-777` の `insert_surface` は新しい summary で `initialize` する。`src-tauri/src/domain/state_subscription/mod.rs:312-347` の `register_delta` は epoch が一致しないとき対象を置換するため、新しい runtime の `initialize` の後に古い closure が走ると登録が古い epoch へ戻る。根拠: R-003「terminal の購読では、最初に現在の状態（再現する画面、その寸法、終了の有無と終了コード）が届き、以後は出力、寸法の変更、終了が届く。」、B-003、R-005「terminal の購読をつなぎ直すときは、最後に受け取った版番号から再開する。その版番号から再開できない場合は、現在の状態が最初から届く。」、B-006、B-007。Thread `7a9b8310-800b-46fb-a8c5-596aa3a8cb1c`。ルート: 委任。
- test helper の mock を terminal の再購読の契約に合わせる: `tests/helpers/tauri-mock.ts` の `StartStateSubscription` は `targets.has(target)` による早期 return を terminal の分岐より前に置くため、同一 target の terminal を再購読しても新しい入力 attachment を登録せず最初の状態も配信しない。`src/lib/client.ts` の `subscribeState` は terminal の既存 entry でも `terminalInputId` を更新し version を未指定へ戻して購読開始を送り直し、同ファイルの `subscribeTerminalState` は最初の item を受け取るまで完了しない。根拠: R-003、B-003、R-019「terminal の購読の開始と停止は、他の購読対象と同じ手段で行える。terminal 専用の attach の呼び出しは無い。」、B-022。Thread `08f9f765-3c48-483e-95c9-4bbc1689d69f`。ルート: 委任。

## 固定するルート

固定する実装上の指定なし。今周に人間が新しく固定した実装上の指定は無い。D1〜D7（design-01）と D8〜D11（design-02）は今周も維持し、範囲・粒度・理由は各 Design の記載のまま変えない。design-02 で列挙した委任の範囲（proto の具体的な形、非同期処理の組み方、コードの配置、テストの置き方）も同じまま維持し、今周の 4 件はいずれもその範囲に入る。

## 変えないもの

design-05 の「変えないもの」を全て維持する。今周に人間が新しく明示した維持対象は無い。

## 未確定・リスク

なし。今周に自動判断した箇所は無く、`requirements.md` の Assumptions / Open Questions は「なし」のままである。未決のまま残した要求と、`[DEFERRED]` で人間へ渡した件も無い。
