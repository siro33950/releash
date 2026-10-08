# Design

パスは、断りのない限りリポジトリ直下を起点とする。行番号は main `f605b1a9` 時点。

## 変える部分

- サーバの配置: `src-tauri/src/`、`src-tauri/tests/`、`src-tauri/build.rs` を `server/` へ移し、package `releashd` の `Cargo.toml` を `server/Cargo.toml` にする。根拠: R-001、R-002、Scope「`server/`（サーバ `releashd` の crate）」。ルート: 固定するルートの配置に従う。
- Cargo workspace の root: `src-tauri/Cargo.toml` の `[workspace]`・`[workspace.package]`、`src-tauri/Cargo.lock`、`src-tauri/deny.toml` を直下へ移す。target は直下の `target/` になる。根拠: Scope「Cargo workspace の root を直下へ移す」。ルート: workspace の root は package を持たない（virtual manifest）。member は `server`、`clients/cli`、`clients/desktop`。
- CLI の配置: `src-tauri/releash/` を `clients/cli/` へ移す。package 名 `releash` と bin 名 `releash` は変えない。根拠: R-002。ルート: 委任。
- desktop の配置: `src-tauri/releash-desktop/` の中身（Rust の crate、`tauri.conf*.json`、`capabilities/`、`icons/`、`launchagents/`）と、React の画面と node の設定（`src/`、`index.html`、`public/`、`package.json`、`pnpm-lock.yaml`、`pnpm-workspace.yaml`、`vite.config.ts`、`tsconfig.json`、`tsconfig.node.json`、`biome.json`、`playwright.config.ts`）を `clients/desktop/` へ移す。desktop だけが使う `scripts/build-desktop-backend.mjs`、`scripts/generate-client-protocol.mjs`、`scripts/generate-icons.sh` は `clients/desktop/scripts/` へ移す。根拠: R-001、Context「破棄は `clients/desktop/` を消すだけで終わる形にする」。ルート: Rust の crate と node の設定が同じディレクトリに並ぶ配置か、Rust の crate を `clients/desktop/` の下の別ディレクトリに置く配置かは委任。`tauri.conf.json` の `frontendDist`・`beforeDevCommand`・`beforeBuildCommand`（`src-tauri/releash-desktop/tauri.conf.json:7-10`、`tauri.conf.bundle.json:3`）は新しい配置に合わせる。
- `releash-sdk` の解体: `src-tauri/releash-sdk/` を消す。proto の Rust の生成（`releash-sdk/build.rs`）は、サーバと `clients/cli` がそれぞれ自分の `build.rs` で行う。サーバの presenter の変換（`releash-sdk/src/wire/values.rs`）と service option の読み取り（`releash-sdk/src/descriptor.rs`）のうちサーバが使うものはサーバへ、client 側の部品（`discovery.rs`、`compatibility.rs`、`daemon.rs`、`data_dir.rs`）と client が使う service option の読み取りは `clients/cli` へ移す。desktop は `clients/cli` の crate に lib として依存し、`releash-sdk` から使っていたものを `clients/cli` から使う。根拠: Scope「`releash-sdk` を無くす」。ルート: desktop は CLI の実行ファイルを呼ばず、lib として依存する。`clients/cli` の lib の公開範囲は委任。
- `client-api.json` の形: `client-api.json` の内容を proto の message で定義し、サーバと `clients/cli` はそれぞれの生成型で読み書きする。識別子の field は `daemon_id` とし、旧キーの `instance_id` は読み取りの互換のための deprecated な field として残す。サーバは `instance_id` を書かない。読む側は `daemon_id` が無く `instance_id` があれば、`instance_id` を識別子として使う。JSON のキーは proto の field 名（snake_case）で書く。根拠: R-003、R-004、B-005、B-006。ルート: message の名前と置き場所（`client.proto` か別ファイルか）は委任。JSON の読み書きは protojson で行う。
- data dir の定数: 既定の data dir 名（`com.releash.app`）、development の接尾辞（`.dev`）、env の名前（`RELEASH_DATA_DIR`）を proto の option に置き、サーバと `clients/cli` はそこから読む。`--data-dir`・env・既定値の順に選ぶ手順は、サーバと `clients/cli` がそれぞれ持つ。根拠: R-005、B-007、B-008。ルート: option の種類と名前は委任。
- サーバの `client-api.json` の書き込みと data dir の解決: サーバが `releash-sdk` から使っていた data dir の解決（`src-tauri/src/lib.rs:36`、`src/adaptor/controller/background_worker.rs:4`、`src/infrastructure/platform/path_aliases.rs:12`、`src/desktop_api.rs:55`）、`client-api.json` の構造体とプロセスの開始時刻の取得（`src/infrastructure/local_api/discovery.rs:8`、`src/adaptor/gateway/local_api.rs:4`、`:20`、`src/desktop_api.rs:78-79`）を、サーバ自身の実装と proto の生成型に置き換える。根拠: R-003、R-005。ルート: 委任。
- テストの見本と helper: `tests/fixtures/terminal-surface-checkpoint-v1.json` は、サーバ（`src-tauri/src/infrastructure/terminal/terminal_emulator_test.rs:84`）と `clients/desktop`（`src/lib/terminalSurfaceStream.test.ts:3`）がそれぞれ自分の crate・ディレクトリの中に持つ。`tests/helpers/client-recovery.mjs` はサーバの統合テストの側へ、`tests/helpers/desktop-daemon.mjs` は desktop の統合テストの側へ移す。直下の `tests/` を消す。根拠: Scope「checkpoint の見本はサーバと `clients/desktop` がそれぞれ持つ」。ルート: テストから上位の相対 path（`../../../../tests/…`）で見本を読まない。
- CI と配布: `.github/workflows/ci.yml`・`nightly.yml`・`stable.yml` の `working-directory` と `rust-cache` の `workspaces`、`bump-version.yml`、`.github/dependabot.yml`、`.github/scripts/test-placement.mjs` を新しい配置に合わせる。`sdk-lint`・`sdk-unit`・`sdk` のジョブ（`ci.yml:541-596`）は消す。frontend 系のジョブは `clients/desktop/` で実行する。根拠: R-006。ルート: 委任。
- リポジトリ全体の設定: `codecov.yml`、`.qlty/qlty.toml`、`.gitignore`、`.ast-grep/rules/`、`workflows/facets/knowledge/implement-task.md` の `src-tauri` を新しい配置に合わせる。これらの設定は直下に残す。根拠: Scope「`src-tauri` と `releash-sdk` を書いているもの…を新しい配置に追従させる」。ルート: 委任。
- 規約とガイド: `AGENTS.md`（`src-tauri` を書いている 9 か所、PR 層のコマンド、Cargo workspace の説明、`releash-sdk` の記述）、`docs/architecture/README.md:3`、`docs/architecture/TEST.md` の置き場所の表とコマンドを新しい配置で書く。TEST.md の「単体（SDK）」の行は消し、「統合（フロント）」「振る舞い」の行は desktop の行にして置き場所を `clients/desktop/` の下にする。`AGENTS.md`、`docs/architecture/`、`docs/glossary/`、`docs/guide/` に書かれたパスがすべて実在する状態にする。根拠: Scope。ルート: 委任。
- 期限の規則: `AGENTS.md:49-51` の「### 通信の原則」に 1 項目足す。内容は「client とサーバの間のやり取りの期限・再試行の値は、proto の service option に置き、両側がそこから読む。目的は、同じやり取りの両側が同じ期限で動くこと。片側だけの待ち（シェルの中だけの待ちなど）は対象外」。根拠: Scope「`AGENTS.md` の『通信の原則』に、期限・再試行の値の置き場所の規則を足す」。ルート: 置き場所は `AGENTS.md` の「通信の原則」。言い回しは整えてよいが、上の趣旨を変えない。
- 「local API」: `AGENTS.md:57` と `:236` の「local API」を、今の入口の呼び方に直す。根拠: Scope。ルート: 委任。
- 消した env の残り: `src-tauri/tests/daemon_smoke.rs:200-202` と `:285` の `RELEASH_DAEMON_PARENT_PIPE` の設定と削除を消す。根拠: Scope。ルート: 委任。

## 固定するルート

- 配置: サーバは `server/`、CLI は `clients/cli/`、desktop は `clients/desktop/`。Cargo workspace の root と `Cargo.lock`・`deny.toml` は直下。`proto/` は動かさない。`clients/macos/` はこの ISSUE では作らない。
- node 関連は直下に残さない。`clients/desktop/` を消せば desktop の破棄が終わる。リポジトリ全体の検査の設定（`.github/scripts/`、qlty、ast-grep、buf、codecov）は直下に残し、中のパスだけ直す。CI の frontend 系ジョブ、`tauri.conf` の `beforeDevCommand`・`beforeBuildCommand`、`AGENTS.md` の PR 層の node のコマンドは `clients/desktop/` で実行する形にする。proto の TS の生成（`generate:protocol`）の入力は `../../proto` になる。
- `releash-sdk` は名前を変えて残さない。サーバとクライアントの間の契約（`client-api.json` の形、data dir の定数、期限・再試行の値）は proto に置き、各 crate が自分で生成した型で読む。
- desktop は `clients/cli` の crate に lib として依存する。
- `docs/specs/issues-1902/design.md:40-45` の 5 条件を守る。crate どうしはファイルの path で互いの中を参照しない。実行時に場所をリポジトリの配置で決めない。テストで実行ファイルを見つける方法は今の方法に揃える。テストに上位の相対 path を書かない。proto の読み方は各 `build.rs` の 1 行で済む形にする。

## 変えないもの

- desktop がサーバの crate の公開入口 `desktop_api` を使う形。
- シェルの中だけの待ち（`desktop_restart.rs` の 10 秒、`single_instance.rs` の 1 秒）。片側だけの待ちで、期限の規則の対象外であるため。
- `docs/specs/` にある closed の ISSUE の spec。記録であるため。

## 未確定・リスク

- なし
