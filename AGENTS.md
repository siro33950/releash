# Releash

Releash は、ソフトウェア開発のための **programmable agentic workflow workbench**。

プロダクトの中心は workflow である。開発者が agentic workflow を定義し、実行し、観測し、承認し、却下し、再指示し、その判断に必要な作業状態を同じ場所で扱えることを目指す。

Releash は、特定の作業単位や特定の道具を主語にしない。コード、差分、terminal、テスト出力、review comment、agent session、workflow run、approval、実行履歴などを、workflow が扱う artifact として統合する。

## プロダクト方針

- Releash は programmable agentic workflow の workbench であり、小さな IDE クローンではない。
- 第一級の状態は workflow state である。
- human checkpoint を第一級に扱う。観測、承認、却下、指示修正、再開を自然にできるようにする。
- artifact は workflow の入力・出力・判断材料であり、プロダクトの主語ではない。
- remote / mobile を扱う場合は、workflow 判断点への監督・介入を中心にする。
- 実装は、実際の workflow action が軽くなる・信頼できるようになる薄い縦切りを優先する。

## 正本の所在

規約と語彙の詳細はこのファイルに複製しない。次を正とする。記述が食い違う場合は正本側に従う。

| 対象 | 正本 |
|---|---|
| Rust レイヤー規約（domain / usecase / gateway / infrastructure / controller / test） | `docs/architecture/` |
| ドメイン横断のユビキタス言語、状態所有、使用禁止語 | `docs/glossary/DOMAIN.md` |
| workflow 定義構文（YAML / Lua / Diagnostic） | `docs/glossary/WORKFLOW.md` |
| spec | `docs/specs/<name>/{requirements,behavior,design}.md` |

変更内容に応じて、着手前に次を確認する。

- Rust を変更する場合は、対象レイヤーの規約と `docs/architecture/TEST.md` を読む。
- ドメインの型・規則・状態所有を変更する場合は、`docs/glossary/DOMAIN.md` を読む。
- workflow 定義構文を変更する場合は、`docs/glossary/WORKFLOW.md` と対象 spec を読む。
- 振る舞いを変更する場合は、対象 spec の requirements / behavior / design を確認する。

## アーキテクチャ原則

### サーバがロジックを所有する

- **全てのアプリケーションロジックはサーバ（daemon）に置く。例外なし。**
- client は、画面（frontend）と Tauri のシェルである。Tauri のシェルは Rust で書かれていても client であり、ロジックを置かない。
- client に許すのは、表示とレイアウト制御、ユーザー入力の受付とフォーム状態管理、サーバの呼び出しと購読（接続、受け取り、つなぎ直し）、受け取ったデータの表示用フォーマット（日付表示形式の変換等）だけ。
- サーバとの通信（呼び出し、購読、受け取り、つなぎ直し）は画面側の React（`src/lib/client.ts`）が直接行う。Tauri のシェルは通信を中継しない。Tauri のシェルが持つのは、ウィンドウ、daemon の起動と監視、更新、接続先の受け渡しなど desktop 固有の操作だけ。
- サーバに置くのは、ビジネスロジック全般、データ変換・加工・計算、バリデーション、外部リソースアクセス（ファイル、Git、ネットワーク等）。
- 新しい振る舞いはサーバの usecase / query service の背後に実装し、client からは proto で定義した呼び出しと購読で使う。
- workflow、session、artifact、terminal、review、persistence のロジックを client に追加しない。
- 依頼された振る舞いの実装・修正に必要な既存ロジックはサーバへ移す。依頼に直接関係しない既存ロジックは、所在と問題を報告し、移設は別途合意する。

### 状態の所有者を明確にする

- workflow runtime、workflow artifact、agent session state、review comment、terminal state、persistence の所有者を明確にする。
- full-retention 設計を避ける。summary、page、id-based operation、delta で足りる場合に、session body、artifact、stream、workflow state 全体を clone / store / recompute / resend しない。
- read model は、Tauri、local API、将来の daemon / native client が同じ backend-owned state を読める形にする。
- frontend state は UI に必要な状態の mirror に留め、domain behavior の source of truth にしない。

## 技術スタック

- **フロントエンド**: React 19 + TypeScript + TailwindCSS 4 + shadcn/ui (Radix)。差分表示は Shiki（Web Worker）、terminal は xterm.js + WebGL
- **バックエンド**: Rust (Tauri 2) + tokio
- **永続化**: SQLite（rusqlite bundled）
- **workflow 定義の評価**: mlua (Lua 5.4)
- **観測**: OpenTelemetry (OTLP)
- **ビルド**: Vite + Biome

## 構成で押さえる点

ディレクトリの内訳はコードを見る。コードからは読み取りにくい点だけ挙げる。

- **workflow 定義はリポジトリ直下の `workflows/`** に置く。`*.yml` と `facets/{instructions,policies,knowledge}/*.md`。builtin は `adaptor/gateway/workflow/builtin.rs` が `include_str!` でコンパイル時に取り込むため、定義を追加するときは builtin.rs 側の登録も要る。
- **実行ファイルは2つある**。`releash`（`src/main.rs`、Tauri のシェル）は daemon を子プロセスとして起動・監視する。`releash-backend`（`src/bin/backend.rs`）は `--internal-daemon` で daemon、それ以外で CLI（`cli/`、`releash workflow|review|hook`）として動く。
- **daemon の入口は2つあり、同じ usecase を共有する**。画面用は Connect の ClientService（契約は `proto/client.proto`、入口は `adaptor/controller/api/client*.rs`、コマンドごとの処理は `adaptor/controller/client/`）。CLI / hook 用は HTTP local API（`adaptor/controller/api/` の `workflow.rs` / `provider_lifecycle.rs`）。Tauri コマンド（`adaptor/controller/command/`）は desktop 固有の操作だけを扱う。
- **daemon は 127.0.0.1 のみに bind する**。discovery file に port と token を書き出す。画面へ渡す client token は master token と分離する。
- **永続化は event store**。`domain/local_event/` と `adaptor/gateway/local_event_store/`。事実を追記し、読み側で projection を導出する。full-recompute 経路を増やさない。

## ビルド・テスト・Lint

CI と同じコマンドを使う。PR・main push の検証は `.github/workflows/ci.yml`、main の重い検証・計測は日次・手動起動の `.github/workflows/nightly.yml` を参照。

- 調査・提案のみでファイルを変更しない場合、ビルドやテストは実行しない。
- 文書のみの変更では、差分・参照先・指示の整合性を確認する。アプリケーションのビルドやテストは実行しない。
- 実装変更では、変更に関係する検証と必須チェックを行う。テスト追加の必須／柔軟は `docs/architecture/TEST.md` に従う。
- 必須チェックが成功した後は、新たな変更・失敗・未解決の懸念がない限り再実行しない。
- 実行できないチェックは理由と未検証範囲を報告し、成功扱いにしない。
- lint が失敗した場合は、今回の変更に起因する問題を対象ファイルに限定して修正する。既存の問題は別途報告し、`pnpm lint:fix` による一括修正で範囲外の変更を混ぜない。

PR 層（プロジェクトルート）:

```bash
pnpm lint
pnpm test
pnpm build
pnpm test:integration
node --test .github/scripts/workflows-test.mjs
```

PR 層（`src-tauri/`。CI では `CARGO_PROFILE_DEV_DEBUG="0"`）:

```bash
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo deny --locked check
cargo clippy --locked --no-default-features --bin releash-backend -- -D warnings
cargo test --locked
cargo test --locked --no-default-features --lib
cargo test --locked --no-default-features --test daemon_smoke
```

品質ゲート（プロジェクトルート。clippy と biome を横断で走らせる）:

```bash
qlty check --no-progress --all
```

nightly 層（プロジェクトルート。daemon の自己検証は release ビルド）:

```bash
pnpm test:performance:daemon
pnpm exec vitest run --coverage
```

nightly 層（`src-tauri/`）:

```bash
cargo test --locked --no-default-features --features performance --lib
cargo test --locked --features performance --test desktop_cli_install
```

Rust coverage は `llvm-tools-preview` と `cargo-llvm-cov` が必要。Linux で強制終了する子プロセスの profile を保持し、短い RPC deadline を使うテストの負荷干渉を避けるため、coverage 計測だけに環境変数を適用する（プロジェクトルート）:

```bash
(
  export RUSTFLAGS="-C llvm-args=-runtime-counter-relocation"
  export LLVM_PROFILE_FILE_NAME="releash-%m%c.profraw"
  export RUST_TEST_THREADS="2"
  python3 .github/scripts/coverage.test.py
  cd src-tauri
  cargo llvm-cov --locked --codecov --output-path rust-codecov.json
)
```

## テスト方針

Rust テストの配置、命名、レイヤー別の必須／柔軟、モック方針は `docs/architecture/TEST.md` を正とする。

フロントエンド:

- テストは対象ファイルの隣に `*.test.tsx` / `*.test.ts` として置く。
- `@tauri-apps/api` の `core` / `event`、xterm、`plugin-dialog` / `plugin-updater` / `plugin-process` は `src/test/setup.ts` で mock 済み。個別ファイルで重ねて mock しない。
- `react-resizable-panels` は jsdom で動作しないため、使うテストごとに `vi.mock` する。
- 外部プロセスはテストで実行しない。
- utility は入出力と edge case、hook は状態遷移と副作用、component は user interaction と conditional rendering をテストする。

## コーディング規約

### フロントエンド

- インデントは tab。import 整理を含め Biome に従う。Biome の対象は `src/**`、`*.json`、`vite.config.ts`（`tests/` は対象外）。
- UI component は shadcn/ui と Radix UI をベースにする。styling は TailwindCSS。
- React component は interface-oriented に保つ。domain decision を hook、reducer、view helper に埋め込まない。

### Rust

- async 処理は tokio を使う。
- module ごとに専用 error type を使う。
- 層ごとの責務、依存方向、命名は `docs/architecture/` に従う。

## コミット・PR

- Conventional Commits。`type(scope): 日本語要約 (#PR番号)` の形にする。
- type は `feat` / `fix` / `docs` / `refactor` / `chore` / `perf`。
- main へ直接 push しない。PR 経由で入れる。
- 版を上げるコミットは `release: vX.Y.Z`。このコミットの merge 自体ではリリースしない。

## リリース

対応プラットフォームは macOS。

1. `Nightly` は毎日（UTC 18:23 / JST 03:23）と `workflow_dispatch` で起動する。main の HEAD を対象とし、日次は直近の公開済み nightly のタグが指す commit と同じならスキップする。手動起動は常に実行する。
2. PR 層の検証一式と `performance` がすべて成功したら、tauri-action で署名・公証済みの macOS universal ビルドを作り、prerelease を公開する。`coverage` は関門に含めない。署名・公証、updater の署名、telemetry の値は 1Password から取得する。
3. nightly のタグは `v{X.Y.Z}-nightly.{YYYYMMDD}.{N}`（UTC のビルド日、日ごとに 1 から採番）。リポジトリとアプリの版は `X.Y.Z` のまま。nightly の Release は直近 14 件を残す。nightly は GitHub Release から手動で取得する。
4. `Stable` を `workflow_dispatch` で起動し、`nightly` に公開済み nightly のタグを指定する。その commit からビルド・署名・公証をやり直し、`vX.Y.Z` を stable の latest Release として公開する。`vX.Y.Z` タグは 1Password の `releash-stable-release`（Contents / Workflows write の fine-grained PAT）で作る。`GITHUB_TOKEN` は workflow ファイルがブランチ先端と異なる commit にタグを作れないため。`latest.json` により既存の Tauri updater で更新できる。
5. stable 公開後、main の版の patch を 1 つ上げ、`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` を揃える PR を作る。別の上げ幅が必要なら、`Bump Version` を `workflow_dispatch`（patch / minor / major）で実行して版更新 PR を作る。

## セキュリティ

- 依存の advisory とライセンスは `cargo deny`（`src-tauri/deny.toml` の allow list）で検査する。新しいライセンスの依存を足すときは allow list への追記が要る。
- CodeQL が javascript-typescript を PR と週次で解析する。
- Tauri capability は `src-tauri/capabilities/`。`startup-pre-admission` は permissions を空にし、main window は Rust の startup authority が Ready に達した後にだけ作る。permission を追加するときは対象 window を確認する。
- local API の master token を renderer JS へ渡さない。terminal 用は別 token を使う。
- Lua の評価環境は外部 I/O を持たず、メモリ量と命令数に上限がある。この上限を緩めない。
- command テンプレートの `{{ }}` は shell quoting を行わない。信頼できない値を shell syntax へ直接連結しない。

## 落とし穴

- `git2` の `UnbornBranch`: `repo.head()` が `ErrorCode::UnbornBranch` を返す場合の分岐が要る。
- `git apply --cached`: パッチのベースはステージング状態にする。HEAD ベースだとコンテキストが一致しない。
- worktree をリポジトリルート内に作らない。Biome が nested config で失敗する。

## レビュー観点

Releash を変更するときは、次を確認する。

- workflow action の定義、実行、観測、承認、却下、再指示のどれかが軽くなるか。
- 新しいロジックは client ではなくサーバに置かれているか。
- 変更した state の source of truth は明確か。
- ドメインの規則（判断・計算・分類・検証・遷移）を domain が所有しているか。状態を持つ概念は集約が、持たない概念は値オブジェクトとドメインサービスが表現しているか。同じ概念が二つの場所で表現されていないか。domain の型と規則は実行経路にあるか。
- full-retention / full-recompute 経路を増やしていないか。
- 同じ backend-owned state を Tauri、local API、将来の client surface で再利用できるか。
- artifact が workflow の判断材料として扱われているか。
- `docs/glossary/DOMAIN.md` の使用禁止語を持ち込んでいないか。
