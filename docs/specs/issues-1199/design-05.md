# Design 05

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `8f6a107a`。作業ブランチは `feat/issues/1199`。
- 直前の Design は `docs/specs/issues-1199/design-04.md`。Spec 工程ではコードを変更していないため、未コミットの作業ツリーを今周の開始状態として扱う。design-04 の「開始状態」に記した実装済みの内訳は現在も同じで、`src-tauri/src` / `src` / `tests` / `src-tauri/tests` に design-04 作成時点より新しい更新はない。
- design-04 の「変える部分」1 件（レイテンシ記録から計測テストへの参照と再実行記述の更新）は未達である。`docs/specs/issues-1199/latency.md:38` が参照する `../../../src-tauri/src/adaptor/controller/api/client_test.rs` は存在せず（`src-tauri/src/adaptor/controller/api/` の実一覧は `auth.rs` / `client.rs` / `error.rs` / `mod.rs` / `protocol.rs` / `protocol_test.rs` / `provider_lifecycle.rs` / `provider_lifecycle_controller_test.rs` / `terminal.rs` / `terminal_test.rs` / `workflow.rs`）、計測本体 `test_クライアントws_往復レイテンシ実測` は `src-tauri/tests/client_api/mod.rs:285` にある。この本体は `src-tauri/Cargo.toml:94-96` の `[[test]] name = "client_api"` による独立した統合テスト target に属し、`src-tauri/src/lib.rs` は取り込んでいないため、`latency.md:33` の `--lib` 指定は現行の target 構成でこの計測テストを選択しない。
- 今周に新しく開いた Thread `ac9b1a19-0109-4c7f-a98b-b301668d2f60` が指す状態を確認した。現行の `src-tauri/src/adaptor/gateway/app_config/repository_impl.rs:129-141` の `configured_secret_values` は Notion の `api_token` だけを収集し、HEAD `8f6a107a` の同関数にあった 8 文字以上の `config.server.token` を一覧へ加える処理は失われている。同ファイルの `load_or_create_config:261-284` は既存ファイルを通常は保存せず（`needs_write` は新規作成時と telemetry の旧形式移行時にだけ立つ）、旧 `[server]` セクションを含む既存ファイルは次の保存まで残る。
- この周に解消・見送り・不成立となった Thread はない。open Thread は上記 2 件で、いずれも `[FIX_POLICY]` が付いている。
- 今周の Requirements の変更はない。Behavior の変更もない。R-001〜R-015 と B-001〜B-020 は対応表で欠落なく対応しており、修正を要する誤り・不足・矛盾はない。

## 変える部分

- 移行期間中の旧 server token の秘匿対象への復帰: 旧 `[server]` セクションを含む既存の設定ファイルを読み込んだ状態で、8 文字以上の旧 server token が workflow の出力・artifact の秘匿対象に含まれ、`secret_masker` の既知パターンに一致しない単独の値として現れた場合も `[REDACTED]` になること。根拠: Thread `ac9b1a19-0109-4c7f-a98b-b301668d2f60` の `[FIX_POLICY]`、R-010「ws 経由にした 2 本以外の desktop 機能は、変更前と同じ経路・同じ結果で動作する」、R-014 / R-015。ルート: 委任
- レイテンシ記録から計測テストへの参照と再実行記述の更新: 実測記録の参照先が現存し往復レイテンシの計測本体へ到達すること、および再実行記述が現行の target 構成でその計測テストを選択すること（`#[ignore]` 付きのため ignored の指定が保たれること）。根拠: Thread `6577bf21-2b8d-44d4-b2ee-11d7315528f1` の `[FIX_POLICY]`、R-012「ws の往復レイテンシの実測値が `docs/specs/issues-1199/` 配下の文書に記録され、後続の予算判断から参照できる」、B-016。対象は `latency.md:38` の参照先と `latency.md:33` の再実行記述に限る。ルート: 委任

## 固定するルート

- 今周に新しく固定する実装上の指定はない。
- design-01 で固定したルートを今周も維持する。local API（`adaptor/controller/api/`、axum）上への ws route 新設、`/v1/terminal` を雛形とするエンベロープ、`Sec-WebSocket-Protocol` bearer 認証、`CommandRequest` / `CommandResponse` と typed push、`attachment_id`＋`sequence` / `ack` と frame 上限・分割の規約、`CommandRouter` の transport 非依存化、push sink の単一化と対象 8 イベント、クライアント token の `TerminalStreamEndpoint` 経路、`[server]` 設定の削除対象、旧 ws shell の非再利用。

## 変えないもの

- `ServerSection` / `ServerConfig` / `TlsConfig` の削除と、アプリケーションが保存する設定ファイルに `[server]` セクションを含めないこと。理由: R-011 が要求し、Thread `ac9b1a19-0109-4c7f-a98b-b301668d2f60` の `[FIX_POLICY]` が受入条件としてこれらを復活させないことを定めているため。
- 旧 `[server]` セクションを含む既存の設定ファイルの読み込みが失敗しないことと、`[server]` の除去時期が保存時であること。理由: R-014 / R-015 が要求し、同 `[FIX_POLICY]` が受入条件に含めているため。
- Notion の `api_token` に対する既存の秘匿結果。理由: 同 `[FIX_POLICY]` が受入条件に含めているため。
- `latency.md` に記録済みの実測値、計測条件、計測範囲と判断上の制限の記述。理由: Thread `6577bf21-2b8d-44d4-b2ee-11d7315528f1` の `[FIX_POLICY]` が受入条件としてこれらの保持を定めており、今周の変更対象を参照先と再実行記述に限っているため（design-04 から維持）。
- 往復レイテンシの許容閾値と GO/NO-GO 判定。理由: 同 `[FIX_POLICY]` が追加しないことを受入条件に含めており、Requirements の Q-004 と一致するため（design-04 から維持）。
- ws 経由にした 2 本（`get_current_branch` / `workflow-execution-changed`）以外の desktop 機能の経路と結果。理由: R-010 が変更前と同じ経路・同じ結果での動作を要求しているため（design-01 から維持）。
- `menu-event` / `native-file-drop` の既存経路。理由: UI shell の事象であり push sink 単一化の対象外と決まっているため（R-006 / B-009、design-01 から維持）。
- エンベロープ定義としての frame 上限・分割の規約。理由: R-004 / B-006 が引き続きこの規約を要求するため（design-03 から維持）。
- A2 で行う stream フロー制御の送受信時の適用。理由: Non-goals および Q-006 と一致し、今周の変更がこの適用を必要としないため（design-02 から維持）。

## 未確定・リスク

この周までに自動判断した箇所は次のとおり。いずれも人間の確認を経ていない。`[DEFERRED]` で人間へ渡した件はない。

- 自動判断（Q-008、design-02 から継続）: 既存の設定ファイルに残る `[server]` セクションは、読み込み時に除去せず、次に設定を保存したときに除去する。この判断により旧 server token が物理的に残る期間があり、今周の「移行期間中の旧 server token の秘匿対象への復帰」はその期間を前提とする。除去時期の判断が変われば、この変更の対象期間も変わる。内容と影響は design-02 の「未確定・リスク」を参照する。
- 自動判断（Q-009、design-03 から継続）: frame 上限値は、renderer へ渡す接続情報に含めない。接続情報としての配布が必要と判断された場合、Non-goals と Q-009 が変わる。
- 自動判断（Q-001〜Q-007、design-01 から継続）: ws 経由にする req/resp 1 本を `get_current_branch`、push 1 本を `workflow-execution-changed` に確定した件、レイテンシ実測値の記録先を `docs/specs/issues-1199/` 配下に確定し許容値（閾値）を定めていない件、ブランチ取得失敗時に Tauri invoke へフォールバックしない件、stream フロー制御と frame 上限・分割を A1 ではエンベロープ定義の規約までとする件、クライアント token を既存の非 master token と同一の 1 本とする件。内容と影響は design-01 の「未確定・リスク」を参照する。
