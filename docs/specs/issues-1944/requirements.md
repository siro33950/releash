# Context

- 入力: https://github.com/siro33950/releash/issues/1944
- 補助資料: #1766（Sparkle による公開）、#1823（OpenCode の使用量）、#1204（SwiftUI クライアントの器）、mock の画像（https://github.com/user-attachments/assets/d694fd1e-4f04-4c55-817c-0600deb9c17a の下端のフッター）
- マイルストーン #78（https://github.com/siro33950/releash/milestone/78 ）の説明文が定める前提:
  - サーバの発見・互換判定・起動は、同梱した CLI を呼んで行う。同じ規則を Swift に実装しない。Swift.app はサーバを監督しない（勝手に起動し直さない）。
  - provider の hook の警告は、フッターの provider の欄に出す。
  - モックは、配置と、出す情報と、できる操作の正である。色と質感の正ではない。
  - 新しい機能のための RPC と購読は足してよい。形はどのクライアントからも同じ形で使えるものにする。
- #1766 は「Sparkle で画面だけを更新した後にサーバが古い場合は、フッターが再起動を案内する」と定めている。この場面では protocol が同じで版だけが違う。新しい版の通知とクライアントを更新する操作は #1766 で作る。
- 外部の仕様:
  - Claude の statusLine は、標準入力に JSON を受け取る。`rate_limits.five_hour` と `rate_limits.seven_day` に `used_percentage`（0〜100）と `resets_at`（Unix epoch 秒）が入る。`rate_limits` は claude.ai の Pro と Max の利用者だけに、最初の API 応答の後から入る。窓はそれぞれ欠けることがある。plan の項目は無い（https://code.claude.com/docs/en/statusline ）。
  - Claude の起動には `--settings <file-or-json>` があり、statusLine を起動時に渡せる。
  - `--settings` で渡す設定は、user・project・local の設定より優先され、managed settings より劣後する。managed settings が statusLine を定めている場合、起動時に渡す statusLine は使われない（https://code.claude.com/docs/en/settings の「Settings precedence」）。
  - Claude の使用率を返す claude.ai の API（`/api/oauth/usage`）は、Claude のログインの OAuth token で呼ぶ。Anthropic の文書は、OAuth を Claude Code などの Anthropic のアプリでの通常の利用に限っている。さらに "developers may not collect, store, or intermediate Claude.ai credentials or session tokens" と定めている（https://code.claude.com/docs/en/legal-and-compliance の「Authentication and credential use」）。このため、サーバがこの token を読んでこの API を呼ぶ形はとらない。
  - Codex の rollout（`~/.codex/sessions/**/*.jsonl`）の `token_count` に `rate_limits` がある。`primary` と `secondary` の窓ごとに `used_percent`、`window_minutes`、`resets_at` を持ち、`plan_type` を持つ。exec モードでは null になる。rollout のパスは `~/.codex/state_5.sqlite` の `threads.rollout_path` にある。
  - Claude の plan は、`~/.claude.json` の `oauthAccount`（`organizationType`、`organizationRateLimitTier`）に入っている。公開された仕様ではない。
- ISSUE 本文との食い違い（管理役・利用者の決定）:
  - 「クライアントが古い／サーバが古い」は、protocol に加えて版も比べて判定する。版の比較は CLI の `status --json` に足し、Swift は結果を表示するだけにする。protocol が違う場合の扱い（接続失敗の画面に CLI の案内文を出す）は今のままにする。
  - 証跡を PR に載せる仕組みは利用者の決定で削除済みである。ISSUE の「手元で確認し、証跡を PR に貼る」は、この文書の要求に含めない。

# Outcome

対象者は、Releash のネイティブ UI（SwiftUI クライアント）で agent の Session と workflow を動かす開発者である。

今は、provider の使用率と reset までの時間を Releash の画面で確かめられない。上限に近づいたことに気づくには、provider の TUI を一つずつ見る必要がある。hook の警告を出す場所も、サーバの版や状態を出す場所もない。サーバを止める・再起動するには CLI を使う必要がある。

変更後は、開発者がウィンドウ下端のフッターで次のことをできる。
- provider ごとの使用率、reset までの時間、plan を見る。上限に近いときは警告で気づく。
- provider の hook の警告を、その provider の欄で見る。
- サーバの版、互換、状態、起動からの時間を見る。サーバとクライアントのどちらが古いかを知る。
- サーバを停止・再起動する。

# Current Behavior

開発開始時点（main `3bc6dd04`）で確認した状態。

Swift のクライアント:
- フッターは枠だけで、中身は無い。高さ `UIStyle.footerHeight` の色を塗っただけの領域である（`clients/macos/Sources/ReleashApp.swift:127-128`）。
- 購読しているのは `workspaces`、`repository-group-state`、`workflows`、`providers`、`workspace-state` である（`clients/macos/Sources/AppModel.swift:71-93`、`:267`）。`provider-hook-health` と `daemon-info` は購読していない。`StopDaemon` と `GetServerInfo` は呼んでいない。
- 接続は、同梱した CLI の `status --json` の `compatibility` が `compatible` のときだけ行う。それ以外は接続失敗の画面に CLI の案内文を出す（`clients/macos/Sources/CLI.swift:73-76`、`ReleashApp.swift:41-49`）。
- 接続が切れると、`recoverStream` が CLI の discover（サーバを起動しない）でつなぎ直しを繰り返す（`AppModel.swift:173-178`）。失敗のたびに `failure` を設定し（`:236`）、「操作に失敗しました」の alert が出る（`ReleashApp.swift:155-162`）。
- 接続失敗の画面の「再試行」は `start()` を呼び、CLI の `server start` でサーバを起動してから接続する（`ReleashApp.swift:47`、`AppModel.swift:55-68`、`CLI.swift:54-63`）。

CLI:
- `status --json` は `client.release`、`client.protocol`、`server`（ServerInfo）、`uptime_seconds`、`compatibility`、`guidance` を返す（`clients/cli/src/server.rs:18-69`）。
- `compatibility` は protocol の番号だけで決まる。等しければ `compatible`、CLI 側が大きければ `server_older`、小さければ `client_older` である（`clients/cli/src/compatibility.rs:9-15`）。版は比べない。
- `releash server restart` は、動いているサーバを止めてから起動する（`clients/cli/src/server.rs:121-133`）。

サーバと proto:
- provider の使用率と reset は、proto・サーバ・画面のどこにも無い。`rate_limits`、`used_percent`、`statusLine` を扱うコードは無い（`server/src`、`proto`、`clients/cli/src` を検索して確認）。
- hook の受信が読むのは session_id、transcript_path、hook_event_name、error、error_details、agent_id、tool_name である（`server/src/adaptor/gateway/provider_lifecycle/payload.rs:115-130`）。
- Claude は `--plugin-dir` で hook を注入して起動する。statusLine は渡していない（`server/src/adaptor/gateway/provider_lifecycle/launch_spec.rs:123-137`、`:253-283`）。
- Codex の履歴の読み取りは `~/.codex/state_5.sqlite` を使い、rollout は読まない（`server/src/adaptor/gateway/agent_session/agent_session_history_gateway.rs:241-260`、`:321-330`）。
- hook の警告は購読 `provider-hook-health` で届く。値は `warnings`（警告ごとに provider、起動の ID、理由）と `read_errors`（hook の状態の記録を読めなかったときのメッセージ。provider を持たない）である（`proto/client.proto:1840-1862`、`server/src/adaptor/presenter/state_subscription_wire.rs:176-185`）。
- サーバの情報は `ServerInfo`（版、daemon_id、pid、起動時刻、protocol、capabilities、serving_status、CLI の設置結果）にあり（`proto/client.proto:2665-2680`）、購読 `daemon-info`（`server/src/adaptor/presenter/state_subscription_target.rs:34`）と `GetServerInfo`（`proto/client.proto:2662`）で読める。
- サーバを止める RPC `StopDaemon` がある（`proto/client.proto:2625`）。再起動の RPC は無い。

# Scope / Non-goals

Scope:
- Swift のフッターの中身（provider の欄、hook の警告、サーバの状態と操作）
- Claude の使用率と plan、Codex の使用率と plan をサーバが取得し、購読で配信すること
- CLI の `status --json` への版の比較の追加
- サーバの停止と再起動の後の画面の振る舞い
- フッターの回帰の E2E

Non-goals:
- 新しい版の通知と、クライアントを更新する操作（#1766）
- OpenCode の使用率（#1823）
- Claude の `context_window` と、Claude apps gateway の `spend_limit`
- managed settings が statusLine を定めている環境での Claude の使用率
- 使用率の永続化
- protocol が違う場合の接続失敗の画面の変更
- 証跡を PR に載せる仕組み
- React の画面（`clients/desktop/src/`）の変更

# Requirements

- R-001: フッターの左に、provider ごとの欄を出す。欄には provider のアイコン、使用率のバー、窓ごとの使用率と窓の長さ（例「78% 5h · 41% wk」）を出す。
- R-002: provider の欄をクリックすると、窓ごとの reset までの時間と、その provider の plan を出す。plan を読めないときは plan だけを出さず、ほかの表示は続ける。
- R-003: provider のいずれかの窓の使用率が 80% を超えると、その provider の欄に警告を出す。
- R-004: provider の hook の警告は、その provider の欄に出す。
- R-005: hook の状態の記録を読めなかったことと、hook の警告の購読の失敗は、provider の欄とは別に、フッターに一つの警告としてまとめて出す。
- R-006: provider の欄は、使用率か hook の警告のどちらかがある provider にだけ出す。
- R-007: managed settings が statusLine を定めていない環境で、Releash が起動した Claude の Session で、Claude が statusLine に渡す使用率と reset が、フッターの Claude の欄に反映される。
- R-008: 利用者が Claude の statusLine を設定している場合、Releash が起動した Claude の Session でも、その statusLine が今までどおり表示される。
- R-009: Claude の plan は、`~/.claude.json` の `oauthAccount` から読んだ値を出す。
- R-010: Releash が起動した Codex の Session で、rollout の最新の `token_count` の使用率、reset、plan が、フッターの Codex の欄に反映される。
- R-011: 使用率は、provider ごとに最後に受け取った値を出す。サーバを再起動した後は、次に値を受け取るまで使用率を出さない。
- R-012: フッターの右に、サーバの版、互換、serving status、起動からの時間を出す。
- R-013: CLI の `status --json` は、protocol が同じで版が違うとき、サーバとクライアントのどちらが古いかを返す。
- R-014: フッターは、サーバが古いときに「再起動で更新できる」ことを出し、クライアントが古いときにクライアントが古いことを出す。
- R-015: フッターの「サーバを停止」でサーバが止まる。止まった後、画面はつなぎ直しをせず、接続失敗の画面に理由「サーバは停止しています」を出す。
- R-016: フッターの「サーバを再起動」は、同梱した CLI の `releash server restart` でサーバを再起動する。再起動の後、画面は新しいサーバに接続し直す。
- R-017: 回帰の E2E で、フッターに provider の使用率とサーバの状態が表示されることを確かめる。

# Assumptions

なし
