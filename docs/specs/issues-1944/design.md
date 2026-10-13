# Design

## 変える部分
- Swift のフッター: `clients/macos/Sources/ReleashApp.swift:127-128` の空の枠を、左に provider の欄、右にサーバの状態と「サーバを停止」「サーバを再起動」を持つフッターに置き換える。根拠: R-001〜R-006、R-012、R-014〜R-016。ルート: 委任
- Claude の statusLine の注入: Claude の起動時に、statusLine のコマンドを渡す。根拠: R-007「Claude が statusLine に渡す使用率と reset が、フッターの Claude の欄に反映される」。ルート: 「固定するルート」1
- statusLine の転送: 注入した statusLine のコマンドが、受け取った JSON をサーバに送る。根拠: R-007。ルート: 「固定するルート」1
- 利用者の statusLine の引き継ぎ: 利用者が statusLine を設定している場合、注入したコマンドが利用者の statusLine を実行し、その出力を Claude に返す。根拠: R-008「その statusLine が今までどおり表示される」。ルート: 「固定するルート」1
- Claude の plan の読み取り: サーバが `~/.claude.json` の `oauthAccount` を読む。読めないときは plan を持たない。根拠: R-009、R-002「plan を読めないときは plan だけを出さず」。ルート: 「固定するルート」3
- Codex の rollout の読み取り: サーバが provider session から rollout を特定し、末尾の `token_count` の `rate_limits` を読む。根拠: R-010。ルート: 「固定するルート」2
- 使用率の read model と購読: provider ごとに最後に受け取った使用率と plan を持つ read model と、それを配信する購読の対象を足す。根拠: R-001、R-002、R-011。ルート: 「固定するルート」4
- hook の警告の表示: Swift が購読 `provider-hook-health` を受け取り、`warnings` を provider の欄に、`read_errors` と購読の失敗を一つの警告に出す。根拠: R-004、R-005。ルート: 委任
- CLI の版の比較: `status --json` に、protocol が同じで版が違うときにどちらが古いかを足す。根拠: R-013。ルート: 「固定するルート」5
- Swift の古さの表示: Swift が CLI の `status --json` の結果を、判定せずにそのままフッターに出す。根拠: R-014。ルート: 「固定するルート」5
- サーバの停止: 「サーバを停止」で、同梱した CLI の `releash server stop` を呼んでサーバを止める。止めた後はつなぎ直しをやめ、接続失敗の画面に「サーバは停止しています」を出す。根拠: R-015。ルート: 「固定するルート」7
- サーバの再起動: 「サーバを再起動」で、同梱した CLI の `releash server restart` を呼び、新しいサーバに接続し直す。根拠: R-016。ルート: 「固定するルート」6
- フッターの回帰の E2E: `clients/macos/UITests/` に、フッターに provider の使用率とサーバの状態が出ることを確かめるテストを足す。根拠: R-017。ルート: 委任

## 固定するルート
1. Claude の使用率
   - Claude の起動時に statusLine のコマンドを注入し、渡される JSON をサーバに転送する。
   - 利用者が自分の statusLine を持つ場合は、注入したコマンドがそれに引き継ぐ。
   - 理由: ISSUE #1944 の「サーバ側に追加が必要」がこの形を定めている。
2. Codex の使用率
   - provider session から rollout を特定し、末尾の `token_count` を読む。置き場所は Codex の履歴の gateway（`server/src/adaptor/gateway/agent_session/agent_session_history_gateway.rs`）の隣とする。
   - 理由: ISSUE #1944 の「サーバ側に追加が必要」がこの形を定めている。
3. Claude の plan
   - サーバが `~/.claude.json` の `oauthAccount` から読む。手元のファイルでは `organizationType`（例 `claude_max`）と `organizationRateLimitTier`（例 `default_claude_max_20x`）に値が入っている。
   - 値が無いときや、形が変わっていて読めないときは、plan だけを持たず、使用率の表示は続ける。
   - 理由: 利用者の決定。
4. 使用率の read model
   - サーバのメモリに、provider ごとの最後の値だけを持つ。event store に記録しない。
   - 理由: 使用率の正本は provider の側にあり、サーバが持つのはその写しである。写しを event store に事実として記録すると、外部の状態の古い写しが残る（管理役の決定）。
5. クライアントとサーバの古さの判定
   - 版の比較は CLI の `status --json` に足す。Swift は結果を表示するだけで、比較しない。
   - protocol が違う場合の扱い（接続失敗の画面に CLI の案内文を出す）は変えない。
   - 理由: マイルストーン #78 の「サーバの発見・互換判定・起動は、同梱した CLI を呼んで行う。同じ規則を Swift に実装しない」（管理役の決定）。
6. サーバの再起動
   - 同梱した CLI の `releash server restart` を呼んで行う。
   - 理由: ISSUE #1944 とマイルストーン #78 の「サーバの発見・互換判定・起動は、同梱した CLI を呼んで行う」。
7. サーバの停止
   - 同梱した CLI の `releash server stop` を呼んで行う。RPC の `StopDaemon` は使わない。
   - 理由: AGENTS.md の「構成で押さえる点」が、サーバの起動・停止は CLI の lib が提供すると定めている。Swift.app も起動には同梱の CLI を使っている（管理役の決定）。

## 変えないもの
- 接続失敗の画面の「再試行」が、CLI でサーバを起動して接続する経路。理由: 管理役の決定。停止の後の「再試行」も、この経路を使う。

## 未確定・リスク
- 環境変数 `CLAUDE_CONFIG_DIR` を設定している場合に、Claude が `.claude.json` を `~/.claude.json` 以外に置くかは確かめていない。置く場合、`~/.claude.json` を読む形では plan を読めない。サーバは `CLAUDE_CONFIG_DIR` を読んで Claude の設定ディレクトリを決めている（`server/src/adaptor/controller/daemon.rs:259`）。
- `~/.claude.json` の `oauthAccount` は公開された仕様ではない。Claude の更新で形が変わると plan を読めなくなる。その場合も R-002 に従い、plan だけを出さない。
- [03] #1759 が、サーバ、proto、Swift の `PaneWorkspace` と `AppModel` を変えている。同じファイルに触るため、後から merge する側で rebase が要る。
