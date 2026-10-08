# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1854 （[10] ディレクトリ構成を見直す）
- 補助資料: マイルストーン #100（https://github.com/siro33950/releash/milestone/100 ）、`docs/specs/issues-1902/design.md` の固定ルート（移動の障害にならない 5 条件）
- この文書は main `f605b1a9`（#1907 の後）で読んだ事実に基づく。
- 配置は 2026-10-09 に利用者が決めた。リポジトリ直下に、サーバの `server/` とクライアントの `clients/` を置く。クライアントは `clients/cli/`（CLI `releash`）、`clients/desktop/`（Tauri シェルと React の画面）、`clients/macos/`（#78 で作る Swift のクライアント）の 3 つである。
- `clients/desktop/` は将来破棄する。破棄は `clients/desktop/` を消すだけで終わる形にする。それまでの間、desktop は `clients/cli` の crate に lib として依存し、`client-api.json` の読み取りやサーバの起動・停止などの部品を使う。
- 以後のクライアントは対応 OS の名前で増やす。macOS と iOS でコードは共有しない。
- `releash-sdk` は無くす。サーバとクライアントの間の契約は `proto/` に置き、proto の生成物は crate ごとに自分で生成する（TS は既に `src/generated` で自前に生成している）。
- `docs/specs/` にある closed の ISSUE の spec は記録なので書き換えない。
- サーバは `.app` の更新をまたいで動き続ける。そのため、新しい CLI と画面は、古いサーバが書いた `client-api.json` を読めなければならない。
- マイルストーン #100 の条件: どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。
- 判断の経緯: 要求の整理での質問への回答は、利用者の中央管理セッション releash-81 から受けた。

# Outcome

- 対象者: Releash の開発者と、Releash.app・CLI の利用者。
- 現在の問題:
  - リポジトリの配置が、サーバと複数のクライアントという今の実態に合っていない。サーバ・Tauri シェル・CLI・共有 crate が `src-tauri/` の下にまとまり、React の画面は直下にある。
  - desktop を破棄するとき、直下の node の設定や `src-tauri/` の中を個別に片付ける必要がある。
  - `client-api.json` は、同じ識別子を `ServerInfo` の `daemon_id` と違う名前（`instance_id`）で書いている。
  - client とサーバの間の期限・再試行の値の置き場所が、規約の文書に書かれていない。
- 変更後の状態:
  - 直下を見れば、サーバ（`server/`）とクライアント（`clients/` の下）の区分が分かる。desktop の破棄は `clients/desktop/` を消せば済む。
  - Releash.app・CLI・hook は今と同じに動く。
  - `client-api.json` の識別子のキーは `daemon_id` で、古いサーバが書いたファイルも読める。

# Current Behavior

main `f605b1a9` で読んで確かめた挙動。

- 配置
  - `src-tauri/` が Cargo workspace の root で、root package はサーバ `releashd`（`src-tauri/src/`、`src-tauri/tests/`、`src-tauri/build.rs`、`src-tauri/deny.toml`）である。member は `releash-desktop`、`releash-sdk`、`releash`（`src-tauri/Cargo.toml:1-3`）。
  - React の画面と node の設定は直下にある（`src/`、`index.html`、`public/`、`package.json`、`pnpm-lock.yaml`、`pnpm-workspace.yaml`、`vite.config.ts`、`tsconfig.json`、`tsconfig.node.json`、`biome.json`、`playwright.config.ts`）。desktop が使う scripts も直下にある（`scripts/build-desktop-backend.mjs`、`scripts/generate-client-protocol.mjs`、`scripts/generate-icons.sh`）。
  - `releash-desktop` は、サーバの crate に依存し（`src-tauri/releash-desktop/Cargo.toml` の `releashd = { path = ".." }`）、公開入口 `releashd::desktop_api` を使う。
  - 直下の `tests/` には、`fixtures/terminal-surface-checkpoint-v1.json` と `helpers/desktop-daemon.mjs`、`helpers/client-recovery.mjs` がある。見本はサーバの単体テスト（`src-tauri/src/infrastructure/terminal/terminal_emulator_test.rs:84`、上位への相対 path）と React の単体テスト（`src/lib/terminalSurfaceStream.test.ts:3`）が共有する。`desktop-daemon.mjs` はシェルの統合テスト（`src-tauri/releash-desktop/tests/desktop_daemon.rs:83`、`:306`）が、`client-recovery.mjs` はサーバの統合テスト（`src-tauri/tests/client_api/mod.rs:225`）が実行する。Playwright の `tests/integration/`・`tests/behavior/` はまだ無い。
- `releash-sdk` の中身と使い手
  - proto から生成した型と Connect の定義（`src-tauri/releash-sdk/src/lib.rs`、`build.rs`）、サーバの presenter の変換（`src/wire/values.rs`、1782 行）、service option の読み取り（`src/descriptor.rs`）、`client-api.json` の読み取りと同一性の確認（`src/discovery.rs`）、互換判定（`src/compatibility.rs`）、サーバの起動・停止・稼働確認（`src/daemon.rs`）、data dir の解決（`src/data_dir.rs`）を持つ。
  - サーバも、生成型と service option のほかに、data dir の解決（`src-tauri/src/lib.rs:36`、`src/adaptor/controller/background_worker.rs:4`、`src/infrastructure/platform/path_aliases.rs:12`）と `client-api.json` の構造体とプロセスの開始時刻の取得（`src/infrastructure/local_api/discovery.rs:8`、`src/adaptor/gateway/local_api.rs:4`）を `releash-sdk` から使う。
- `client-api.json`
  - サーバは識別子をキー `instance_id` で書く（`src-tauri/releash-sdk/src/discovery.rs:7-13` の構造体を serde で書き出す）。
  - CLI と画面は同じ構造体で読み、`instance_id` を `ServerInfo` の `daemon_id` と照合する（`src-tauri/releash-sdk/src/discovery.rs:102-111`）。
- data dir の解決は `--data-dir`、`RELEASH_DATA_DIR`、ビルド種別の既定値（release は `com.releash.app`、debug は `com.releash.app.dev`）の順である（`src-tauri/releash-sdk/src/data_dir.rs:26-48`）。
- 期限・再試行の値は `proto/client_options.proto:39-46` の ServiceOptions にあり、画面・CLI・シェルはそこから読む。`AGENTS.md:49-51` の「通信の原則」は、配信と購読の 1 項目だけで、値の置き場所を書いていない。
- `AGENTS.md:57` と `:236` に「local API」の語が残っている。HTTP の入口は #1902 で削除済みである。
- `src-tauri/tests/daemon_smoke.rs:200-202` と `:285` が、#1904 で消した env `RELEASH_DAEMON_PARENT_PIPE` を設定・削除している。
- CI（`.github/workflows/ci.yml`）には `sdk-lint`・`sdk-unit`・`sdk` のジョブがある（`:541-596`）。

# Scope / Non-goals

## Scope

- 配置を変える。`server/`（サーバ `releashd` の crate）、`clients/cli/`（CLI `releash` の crate）、`clients/desktop/`（Tauri シェルの crate、React の画面、node の設定、desktop だけが使う scripts）を作り、Cargo workspace の root を直下へ移す。`src-tauri/` と直下の `tests/` を無くす。
- `releash-sdk` を無くす。中身は使い手の側へ移す。client 側の部品（`client-api.json` の読み取りと同一性の確認、互換判定、サーバの起動・停止・稼働確認、data dir の解決）は `clients/cli` へ移す。
- `client-api.json` の形と、サーバとクライアントが共に使う data dir の定数を proto に定義する。
- `client-api.json` の識別子のキーを `daemon_id` にする。
- checkpoint の見本はサーバと `clients/desktop` がそれぞれ持つ。helper の `.mjs` は実行するテストの側へ移す。
- `src-tauri` と `releash-sdk` を書いているもの（CI、scripts、設定、`AGENTS.md`、`docs/` の規約・ガイド、`workflows/`）を新しい配置に追従させる。`docs/architecture/TEST.md` の置き場所の表を新しい配置で書き、「統合（フロント）」「振る舞い」の行を desktop の行にする。
- `AGENTS.md` の「通信の原則」に、期限・再試行の値の置き場所の規則を足す。
- `AGENTS.md` の「local API」を今の入口の呼び方に直す。
- `daemon_smoke.rs` の `RELEASH_DAEMON_PARENT_PIPE` の行を消す。

## Non-goals

- Swift のクライアント（`clients/macos/` の中身）。#78 で作る。
- `proto/` の移動。
- `docs/specs/` にある closed の ISSUE の spec の書き換え。
- desktop がサーバの crate の公開入口 `desktop_api` を使う形の変更。
- シェルの中だけの待ち（`desktop_restart.rs` の 10 秒、`single_instance.rs` の 1 秒）を proto へ移すこと。
- main の ruleset の必須チェックの変更と、マイルストーン #78 の説明文・#1204 の本文の `macos/` の書き換え。merge 前に利用者が行う。

# Requirements

- R-001: 新しい配置からビルドした Releash.app は、今と同じに動く。画面はサーバを見つけ、居なければ起動して接続する。
- R-002: 新しい配置からビルドした CLI `releash` と hook は、今と同じに動く。
- R-003: サーバは `client-api.json` に識別子をキー `daemon_id` で書き、`instance_id` を書かない。
- R-004: CLI と画面は、`client-api.json` の `daemon_id` でサーバの同一性を確かめる。`daemon_id` が無く `instance_id` だけがあるファイル（古いサーバが書いたもの）も同じ識別子として読み、そのサーバに接続する。
- R-005: data dir は、`--data-dir`、`RELEASH_DATA_DIR`、ビルド種別の既定値（release は `com.releash.app`、development は `com.releash.app.dev`）の順で決まる。サーバ・CLI・画面は同じ条件で同じ data dir を選ぶ。
- R-006: CI の検証一式は新しい配置で動き、すべて成功する。

# Assumptions

- なし
