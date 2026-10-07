# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1903 （[06] サーバを `releashd` に改名し、単独で起動できるようにする）
- 補助資料: マイルストーン #100（https://github.com/siro33950/releash/milestone/100 ）、#1902（[05] CLI を薄いクライアント crate `releash` にする。`docs/specs/issues-1902/` が正）、#1904（[07] 画面をサーバの利用者にする）
- 本文の「今の作り」の file:line は main `4533af2f` 時点のもの。この文書は main `6647cbfa`（#1902 の後）で読み直した事実に基づく。
- 本文が `releash-backend` を固定で書いている箇所として挙げる `package.json`、`.github/scripts/workflows-test.mjs`、`docs/guide/cli.md`、`tests/helpers/*.mjs`、`wdio.performance.conf.ts`、`infrastructure/platform/cli_install.rs` には、main `6647cbfa` で `releash-backend` の記述は無い（#1902 で変わった）。
- 本文の方針「居なければ隣の `releashd` を detached で起動する処理を、クライアント側の共有 crate に置く」は、この ISSUE では作らない。この ISSUE には使い手が無く（CLI の `server start` は #1905、画面の起動時の利用は #1904）、使い手の無いコードを置くことになるため。#1902 で `Compatibility` を最初の使い手の ISSUE で作ったのと同じ扱いで、#1904 の本文（「クライアント側の共有 crate で…居なければ隣の `releashd` を起動して」）とは矛盾しない。
- 本文の方針「排他は store の writer lock だけにする。2 つ目は理由を出して exit 1」は、起動の形に依らず main `6647cbfa` で満たされている（`src-tauri/tests/daemon_smoke.rs:319-335`）。
- 対応プラットフォームは macOS（AGENTS.md「リリース」）。
- マイルストーン #100 の条件: どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。
- 判断の経緯: 要求の整理での質問への回答は、利用者の中央管理セッション releash-7a から受けた。

# Outcome

- 対象者: Releash のサーバを画面なしで動かしたい人（launchd・systemd などのサービス管理から、または手で起動する人）、CLI を使う人と agent、Releash の開発者。
- 現在の問題: サーバは `releash-backend` という名前で、画面が子プロセスとして起動する内部の引数（`--internal-daemon`）でしか起動できない。引数なしでは usage を出して終わり、画面が無いとサーバを動かせない。サーバと CLI で data dir の決め方が違う。
- 変更後の状態: サーバは `releashd` という名前の実行ファイルで、引数なしで起動するとそのまま foreground のサーバとして動き続ける。data dir は CLI と同じ規則で決まり、単独で起動したサーバに CLI が接続して操作できる。画面がサーバを子プロセスとして起動する今の経路は変わらずに動く。

# Current Behavior

main `6647cbfa` で読んで確かめた挙動。パスは `src-tauri/` 起点。

- サーバの crate は root package `releash-backend`（`Cargo.toml:9`）、lib は `releash_lib`（`:22`）、bin は `releash-backend`（`:25`、`src/bin/backend.rs`）。Tauri シェルは `releash-backend` に依存し（`releash-desktop/Cargo.toml:12,39,40,47`）、`releash_lib::desktop_api` を使う。
- `releash-backend` の引数は `--internal-daemon [DATA_DIR]` と `--internal-background-worker` だけ。それ以外（引数なしを含む）は `usage: releash-backend --internal-daemon [DATA_DIR]` を stderr に出して exit 2（`src/bin/backend.rs:1-15`）。
- `--internal-daemon` で DATA_DIR を省くと、data dir は既定だけで決まり `RELEASH_DATA_DIR` を見ない（`src/lib.rs:37-40`、`src/infrastructure/platform/app_data_dir.rs:5-7`）。CLI は `--data-dir` ＞ `RELEASH_DATA_DIR` ＞ 既定 で決める（`releash/src/lib.rs:54-71`）。既定の値は両者とも `releash-sdk/src/data_dir.rs:35-37`。
- 画面は自分の隣の `releash-backend` を `--internal-daemon <data_dir>`、env `RELEASH_DAEMON_LAUNCH_ID`・`RELEASH_DAEMON_PARENT_PIPE=1` で起動する（`releash-desktop/src/desktop.rs:69`、`releash-desktop/src/adaptor/gateway/daemon_supervision.rs:163-173`）。stdin の EOF での終了は env `RELEASH_DAEMON_PARENT_PIPE` があるときだけ働く（`src/infrastructure/process/parent_lifetime.rs:10-13`）。停止の完了は stdout の `releash-shutdown-complete`（`src/adaptor/controller/daemon.rs:94`）。
- サーバは background worker を、自分の隣（cargo の `deps/` の中で動くときは 2 つ上）の `releash-backend` を `--internal-background-worker` で起動して使う（`src/infrastructure/process/background_worker.rs:25-41`）。
- 同じ data dir で 2 つ目を起動すると、`Local data is currently in use…（<correlation id>）` を stderr に出して exit 1（`tests/daemon_smoke.rs:319-335`）。
- `.app` への同梱: `scripts/build-desktop-backend.mjs:7,15,19` が `releash-backend` と `releash` を作って `binaries/` に置き、`releash-desktop/tauri.conf.bundle.json:7-10` の `externalBin` に両方を載せる。
- `releash-backend` を固定で書いている箇所（`git grep -c releash-backend`、`docs/specs/` を除く）: `AGENTS.md`、`.github/workflows/ci.yml`、`docs/architecture/TEST.md`、`scripts/build-desktop-backend.mjs`、`Cargo.toml`・`Cargo.lock`、`releash-desktop/Cargo.toml`、`releash-desktop/tauri.conf.bundle.json`、`releash-desktop/src/desktop.rs`、`src/bin/backend.rs`、`src/infrastructure/process/background_worker.rs`、`tests/daemon_smoke.rs`、`tests/workflow_control_plane_acceptance_test.rs`、`tests/provider_lifecycle_acceptance_test.rs`、`tests/agent_session_tui_acceptance.rs`、`releash-desktop/tests/desktop_daemon.rs`、`releash-desktop/tests/support/mod.rs`、`releash/tests/support/diagnostics.rs`。
- CLI の統合テストはサーバを `--internal-daemon <dir>` と env `RELEASH_DAEMON_LAUNCH_ID` で起動している（`releash/tests/support/diagnostics.rs:40-54`）。

# Scope / Non-goals

## Scope

- サーバの crate・lib・実行ファイルの名前を `releashd` にし、`releash-backend` を固定で書いている箇所（ビルド、同梱、CI、テスト、文書）をすべて追従させる。
- `releashd` を引数なしで起動したときに foreground のサーバとして動くようにする。
- data dir の決め方（`--data-dir` ＞ `RELEASH_DATA_DIR` ＞ 既定）をサーバと CLI で同じにする。

## Non-goals

- 「居なければ隣の `releashd` を detached で起動する」処理（使い手の ISSUE #1904・#1905 で作る。Context を参照）。
- 画面が子プロセスとして起動する経路の削除（`--internal-daemon`、stdin の EOF での終了、stdout の完了マーカー、`RELEASH_DAEMON_LAUNCH_ID`・`RELEASH_DAEMON_PARENT_PIPE`）と、画面の監督の削除（#1904）。
- CLI のサーバの状態確認・起動・停止・再起動（#1905）。CLI の配置の規則（#1907）。
- ディレクトリ構成の見直し（root package を `src-tauri/` 直下に置いたままにする。#1854）。
- launchd の plist・systemd の unit の提供。この ISSUE は、それらから起動できる形（引数なしの foreground のサーバ）までを扱う。
- 多重起動の排他の仕組みの変更（store の writer lock のまま）。

# Requirements

- R-001: サーバの crate と実行ファイルの名前は `releashd` である。ビルド、`.app` への同梱、CI、テスト、文書に `releash-backend` の名前は残らない。
- R-002: `releashd` を引数なしで起動すると、foreground のサーバとして動き、明示的に止めるまで動き続ける。
- R-003: `releashd` の data dir は、`--data-dir` の指定 ＞ `RELEASH_DATA_DIR` ＞ 既定 の順で決まり、CLI（`releash`）と同じ規則である。
- R-004: 同じ data dir でサーバが既に動いているとき、2 つ目の `releashd` は理由を stderr に出して終了コード 1 で終わる。
- R-005: 単独で起動した `releashd` に、CLI が接続して操作できる。
- R-006: Tauri アプリは今と同じに動く（サーバを子プロセスとして起動して接続し、再起動・更新・Quit で今と同じにサーバを止める）。

# Assumptions

- なし
