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

## アーキテクチャ原則

### Rust がロジックを所有する

- **全てのアプリケーションロジックは Rust に置く。例外なし。**
- frontend に許すのは、表示とレイアウト制御、ユーザー入力の受付とフォーム状態管理、Tauri command（`invoke`）の呼び出し、受け取ったデータの表示用フォーマット（日付表示形式の変換等）だけ。
- Rust 側に置くのは、ビジネスロジック全般、データ変換・加工・計算、バリデーション、外部リソースアクセス（ファイル、Git、ネットワーク等）。
- 新しい振る舞いは Rust の usecase / query service / Tauri command の背後に実装し、frontend からは `invoke` で呼ぶ。
- workflow、session、artifact、terminal、review、persistence のロジックを React hook や UI component に追加しない。
- 触った frontend code に既存ロジックが残っている場合は、タスクのスコープ内で可能な範囲で Rust 境界へ移す。

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
- **入口は3つあり、同じ usecase を共有する**。Tauri command（`adaptor/controller/command/`）、loopback HTTP local API（`adaptor/controller/api/` と `infrastructure/local_api/`）、CLI（`cli/`、`releash workflow|review|hook`）。`main.rs` が引数の有無で CLI と GUI を分岐する。
- **local API は 127.0.0.1 のみに bind する**。discovery file に port と token を書き出す。terminal stream だけが WebSocket で、renderer へ渡す token は master token と分離する。
- **永続化は event store**。`domain/local_event/` と `adaptor/gateway/local_event_store/`。事実を追記し、読み側で projection を導出する。full-recompute 経路を増やさない。

## ビルド・テスト・Lint

CI と同じコマンドを使う。`.github/workflows/ci.yml` を参照。

プロジェクトルート:

```bash
pnpm lint              # 落ちたら pnpm lint:fix
pnpm test
pnpm build
pnpm test:integration
```

`src-tauri/`:

```bash
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo deny --locked check
cargo build --locked
cargo test --locked --test agent_tui_harness
cargo test --locked
```

品質ゲート（プロジェクトルート。clippy と biome を横断で走らせる）:

```bash
qlty check --no-progress --all
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
- リリースコミットは `release: vX.Y.Z`。

## リリース

対応プラットフォームは macOS。

1. `Bump Version` を workflow_dispatch で実行する（patch / minor / major）。version 更新 PR ができる。
2. main へ merge すると `Auto Tag` が `package.json` の version 変更を検知して `vX.Y.Z` タグを作る。
3. タグ push で `Release` が tauri-action により macOS ビルドと GitHub Release 作成を行う。署名鍵は 1Password から取得する。

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
- `docs/specs` は `.gitignore` に入っているが、既存のファイルは tracked のまま残っている。新規に作った spec だけが ignore される。

## レビュー観点

Releash を変更するときは、次を確認する。

- workflow action の定義、実行、観測、承認、却下、再指示のどれかが軽くなるか。
- 新しいロジックは frontend ではなく Rust に置かれているか。
- 変更した state の source of truth は明確か。
- ドメインの規則（判断・計算・分類・検証・遷移）を domain が所有しているか。状態を持つ概念は集約が、持たない概念は値オブジェクトとドメインサービスが表現しているか。同じ概念が二つの場所で表現されていないか。domain の型と規則は実行経路にあるか。
- full-retention / full-recompute 経路を増やしていないか。
- 同じ backend-owned state を Tauri、local API、将来の client surface で再利用できるか。
- artifact が workflow の判断材料として扱われているか。
- `docs/glossary/DOMAIN.md` の使用禁止語を持ち込んでいないか。
