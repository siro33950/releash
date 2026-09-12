# Design 06

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `8f6a107a`。作業ブランチは `feat/issues/1199`。
- 直前の Design は `docs/specs/issues-1199/design-05.md`。Spec 工程ではコードを変更していないため、未コミットの作業ツリーを今周の開始状態として扱う。design-05 作成後に更新されたのは `src-tauri/src/adaptor/gateway/app_config/repository_impl.rs`、`src-tauri/src/adaptor/gateway/app_config/repository_impl_test.rs`、`src-tauri/tests/client_api/mod.rs`、`src-tauri/src/client_api_acceptance.rs` である。
- design-05 の「変える部分」のうち、移行期間中の旧 server token の秘匿対象への復帰は満たされている。`src-tauri/src/adaptor/gateway/app_config/repository_impl.rs:128-156` の `configured_secret_values` が `self.config_path` を読み直し、`[server]` の `token` が 8 文字以上であれば秘匿対象の一覧へ加える。
- design-05 の「変える部分」のうち、レイテンシ記録から計測テストへの参照と再実行記述の更新は未達である。`docs/specs/issues-1199/latency.md` は design-05 作成後に更新されておらず、`latency.md:38` が参照する `../../../src-tauri/src/adaptor/controller/api/client_test.rs` は存在せず（`src-tauri/src/adaptor/controller/api/` の実一覧は `auth.rs` / `client.rs` / `error.rs` / `mod.rs` / `protocol.rs` / `protocol_test.rs` / `provider_lifecycle.rs` / `provider_lifecycle_controller_test.rs` / `terminal.rs` / `terminal_test.rs` / `workflow.rs`）、計測本体 `test_クライアントws_往復レイテンシ実測` は `src-tauri/tests/client_api/mod.rs:323` にある。この本体は `src-tauri/Cargo.toml` の `[[test]] name = "client_api" path = "tests/client_api/mod.rs"` による独立した統合テスト target に属し、`src-tauri/src/lib.rs` が取り込むのは `pub mod client_api_acceptance;` だけであるため、`latency.md:33` の `--lib` 指定は現行の target 構成でこの計測テストを選択しない。
- 今周に新しく開いた Thread `74b93fae-d34c-4d86-ab5d-936718748783` が指す状態を確認した。上記の `configured_secret_values` は、メモリ上の設定から 8 文字以上の Notion `api_token` を一覧へ収集した後に `self.config_path` を `fs::read_to_string` で読み直す。`ErrorKind::NotFound` だけが収集済みの一覧を返して早期復帰し、その他の I/O エラーは `AppConfigError::Repository("設定ファイル読み込み失敗: …")`、`toml::from_str` の失敗は `AppConfigError::Repository("秘匿対象の旧設定のパース失敗")` を返して収集済みの一覧を捨てる。呼出元 `src-tauri/src/adaptor/gateway/workflow/secret_source.rs:12` はこの `Err` を `unwrap_or_default()` で空配列に変換する。派生点 `8f6a107a` の同関数はメモリ上の設定だけから収集し、ディスク I/O を持たなかった。追加済みの `repository_impl_test.rs:118-143` の 2 テストは `ReleashConfig::default()`（Notion 無し）で `Err` と文言だけを断定している。
- この周に解消・見送り・不成立となった Thread はない。open Thread は上記 2 件で、いずれも `[FIX_POLICY]` が付いている。
- 今周の Requirements の変更はない。Behavior の変更もない。R-001〜R-015 と B-001〜B-020 は対応表で欠落なく対応しており、修正を要する誤り・不足・矛盾はない。

## 変える部分

- 収集済みの秘匿値が設定ファイル取得失敗で失われないこと: メモリ上の設定から得られた 8 文字以上の Notion `api_token` が、その後の設定ファイル取得失敗（`NotFound` 以外の I/O エラー、および TOML パース失敗）を理由に秘匿対象から落ちず、workflow の command 表示、`build_command_artifact` による artifact、approval comment と提出 artifact で `[REDACTED]` に置換されること。根拠: Thread `74b93fae-d34c-4d86-ab5d-936718748783` の `[FIX_POLICY]`、R-010「ws 経由にした 2 本以外の desktop 機能は、変更前と同じ経路・同じ結果で動作する」、B-014「THEN 変更前と同じ経路で処理され、変更前と同じ結果になる」。ルート: 委任
- レイテンシ記録から計測テストへの参照と再実行記述の更新: 実測記録の参照先が現存し往復レイテンシの計測本体へ到達すること、および再実行記述が現行の target 構成でその計測テストを選択すること（`#[ignore]` 付きのため ignored の指定が保たれること）。根拠: Thread `6577bf21-2b8d-44d4-b2ee-11d7315528f1` の `[FIX_POLICY]`、R-012「ws の往復レイテンシの実測値が `docs/specs/issues-1199/` 配下の文書に記録され、後続の予算判断から参照できる」、B-016。対象は `latency.md:38` の参照先と `latency.md:33` の再実行記述に限る。ルート: 委任

## 固定するルート

- 今周に新しく固定する実装上の指定はない。
- design-01 で固定したルートを今周も維持する。local API（`adaptor/controller/api/`、axum）上への ws route 新設、`/v1/terminal` を雛形とするエンベロープ、`Sec-WebSocket-Protocol` bearer 認証、`CommandRequest` / `CommandResponse` と typed push、`attachment_id`＋`sequence` / `ack` と frame 上限・分割の規約、`CommandRouter` の transport 非依存化、push sink の単一化と対象 8 イベント、クライアント token の `TerminalStreamEndpoint` 経路、`[server]` 設定の削除対象、旧 ws shell の非再利用。

## 変えないもの

- 移行期間中の旧 `[server]` token が秘匿対象に含まれること。理由: Thread `74b93fae-d34c-4d86-ab5d-936718748783` の `[FIX_POLICY]` が、design-05 の旧 `[server]` token の秘匿対象への復帰の同時成立を受入条件に含めているため。
- Notion の `api_token` に対する既存の秘匿結果。理由: 同 `[FIX_POLICY]` が受入条件に含めているため（design-05 から維持）。
- エラー文言に秘匿値そのものを含めないこと。理由: 同 `[FIX_POLICY]` が既存の断定の保持を受入条件に含めているため。
- `ServerSection` / `ServerConfig` / `TlsConfig` の削除と、アプリケーションが保存する設定ファイルに `[server]` セクションを含めないこと。理由: R-011 が要求し、同 `[FIX_POLICY]` がこれらを復活させないことを受入条件に含めているため（design-05 から維持）。
- 旧 `[server]` セクションを含む既存の設定ファイルの読み込みが失敗しないことと、`[server]` の除去時期が保存時であること。理由: R-014 / R-015 が要求し、同 `[FIX_POLICY]` が受入条件に含めているため（design-05 から維持）。
- `latency.md` に記録済みの実測値、計測条件、計測範囲と判断上の制限の記述。理由: Thread `6577bf21-2b8d-44d4-b2ee-11d7315528f1` の `[FIX_POLICY]` が受入条件としてこれらの保持を定めており、今周の変更対象を参照先と再実行記述に限っているため（design-04 から維持）。
- 往復レイテンシの許容閾値と GO/NO-GO 判定。理由: 同 `[FIX_POLICY]` が追加しないことを受入条件に含めており、Requirements の Q-004 と一致するため（design-04 から維持）。
- ws 経由にした 2 本（`get_current_branch` / `workflow-execution-changed`）以外の desktop 機能の経路と結果。理由: R-010 が変更前と同じ経路・同じ結果での動作を要求しているため（design-01 から維持）。
- `menu-event` / `native-file-drop` の既存経路。理由: UI shell の事象であり push sink 単一化の対象外と決まっているため（R-006 / B-009、design-01 から維持）。
- エンベロープ定義としての frame 上限・分割の規約。理由: R-004 / B-006 が引き続きこの規約を要求するため（design-03 から維持）。
- A2 で行う stream フロー制御の送受信時の適用。理由: Non-goals および Q-006 と一致し、今周の変更がこの適用を必要としないため（design-02 から維持）。

## 未確定・リスク

この周までに自動判断した箇所は次のとおり。いずれも人間の確認を経ていない。`[DEFERRED]` で人間へ渡した件はない。

- 自動判断（Q-008、design-02 から継続）: 既存の設定ファイルに残る `[server]` セクションは、読み込み時に除去せず、次に設定を保存したときに除去する。この判断により旧 server token が物理的に残る期間があり、その期間の秘匿のために `configured_secret_values` が設定ファイルを読み直す構成になっている。今周の「収集済みの秘匿値が設定ファイル取得失敗で失われないこと」はこの構成を前提とする。除去時期の判断が変われば、読み直しの要否とこの変更の前提も変わる。内容と影響は design-02 の「未確定・リスク」を参照する。
- 自動判断（Q-009、design-03 から継続）: frame 上限値は、renderer へ渡す接続情報に含めない。接続情報としての配布が必要と判断された場合、Non-goals と Q-009 が変わる。
- 自動判断（Q-001〜Q-007、design-01 から継続）: ws 経由にする req/resp 1 本を `get_current_branch`、push 1 本を `workflow-execution-changed` に確定した件、レイテンシ実測値の記録先を `docs/specs/issues-1199/` 配下に確定し許容値（閾値）を定めていない件、ブランチ取得失敗時に Tauri invoke へフォールバックしない件、stream フロー制御と frame 上限・分割を A1 ではエンベロープ定義の規約までとする件、クライアント token を既存の非 master token と同一の 1 本とする件。内容と影響は design-01 の「未確定・リスク」を参照する。
