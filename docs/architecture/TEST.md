# テスト 規約

## 種類

種類は検証に関わる仕組みと操作の仕方で決める。置き場所・実行手段・CI は種類ごとに分け、置き場所から種類が決まるようにする。

| 種類 | 確かめること |
|---|---|
| 単体 | 単一の仕組みで完結すること |
| 統合 | 2つ以上の仕組み（サーバ/DB、フロント/サーバ、サーバ/OS）の関連 |
| 振る舞い | 本物のサーバ・DB・OS を含む構成に対し、ユーザと同様の操作をして確かめる妥当性 |

## 書かないテスト

- 削除済み機能が存在しないことを確かめるテスト
- 定数の値を確かめるテスト
- 単体テストで証明できることを確かめる統合テスト
- 統合テストで証明できることを確かめる振る舞いテスト
- 下位の層の単体テストで証明できることを確かめる上位の層の単体テスト
- 同じ入力区分を確かめる2本目以降の単体テスト（区分の境界値を確かめるものは除く）
- 主要パターン以外の振る舞いテスト
- パフォーマンステスト

## 配置と実行

| 種類 | 置き場所 | 実行 | CI |
|---|---|---|---|
| 単体（サーバ） | `src-tauri/src/` の `<impl>_test.rs` | `cargo test --lib --bins -p releash-backend`、`cargo test --doc -p releash-backend` | PR 層の単体ジョブ |
| 単体（シェル） | `src-tauri/releash-desktop/src/` の `<impl>_test.rs` | `cargo test --lib --bins -p releash-desktop`、`cargo test --doc -p releash-desktop` | PR 層の単体ジョブ |
| 単体（共有） | `src-tauri/releash-client/src/` の `<impl>_test.rs` | `cargo test --lib -p releash-client`、`cargo test --doc -p releash-client` | PR 層の単体ジョブ |
| 単体（CLI） | `src-tauri/releash/src/` の `<impl>_test.rs` | `cargo test --lib --bins -p releash`、`cargo test --doc -p releash` | PR 層の単体ジョブ |
| 単体（フロント） | `src/` の `*.test.ts(x)` | `pnpm test` | PR 層の単体ジョブ |
| 統合（サーバ） | `src-tauri/tests/` | `cargo test --test '*' -p releash-backend` | PR 層の統合ジョブ |
| 統合（シェル） | `src-tauri/releash-desktop/tests/` | `cargo test --test '*' -p releash-desktop` | PR 層の統合ジョブ |
| 統合（CLI） | `src-tauri/releash/tests/` | `cargo test --test '*' -p releash` | PR 層の統合ジョブ |
| 統合（フロント） | `tests/integration/` | `pnpm test:integration` | PR 層の統合ジョブ |
| 振る舞い | `tests/behavior/` | `pnpm test:behavior` | nightly 層 |

手動で実行するテストはコミットしない。

統合テストの前に、`src-tauri/` で次を実行する。サーバは `cargo build --locked -p releash --bin releash`、シェルと CLI は `cargo build --locked -p releash-backend --bin releash-backend -p releash --bin releash`。テストから cargo は呼ばない。

Rust の単体テストは、実装と同じディレクトリに `<impl>_test.rs` を置き、`<impl>.rs` の末尾で `#[path]` を指定して取り込む。ファイル名は `<impl>_test.rs`、テストモジュール名は `<impl>_tests` とする。

例: `terminal_surface_registry.rs` と同じディレクトリの `terminal_surface_registry_test.rs` を取り込む。

```rust
#[cfg(test)]
#[path = "terminal_surface_registry_test.rs"]
mod terminal_surface_registry_tests;
```

## 命名規則

- テスト関数: `test_{業務機能}_{条件と期待結果}`。業務機能は日本語で書き、テスト失敗時に意図が伝わる名前にする（例: `test_ブランチ作成_空文字エラー`、`test_差分Hunk計算_変更行のみがhunkになる`）
- テストモジュール: `{implementation_name}_tests`

## レイヤー別の必須／柔軟

| レイヤー | テスト | 理由 |
|---|---|---|
| `domain/` | **必須** | ビジネスロジックの中核 |
| `usecase/` | **必須** | 業務手順の正しさを担保 |
| `adaptor/gateway/` | **必須** | 外部システムとの境界、モデル変換の検証 |
| `adaptor/controller/command/` | 柔軟 | Tauri 依存で書きにくい場合は省略可 |
| `adaptor/controller/api/` | 柔軟 | Connect 依存で書きにくい場合は省略可 |
| `adaptor/presenter/` | 柔軟 | 表示整形のみ、必要に応じて |
| `infrastructure/` | 柔軟 | 外部世界の都合をそのまま扱う層。判断も変換も持たないため、統合テストで検証 |
| `common/` | 柔軟 | 横断的関心事の包み |

「柔軟」のレイヤーも、テストを書ける範囲では書く。書きにくいから書かない判断は許容するが、書きやすくする工夫（インターフェース抽出等）も検討する。

## テスト構造

Given / When / Then をコメントで区切り、前提・操作・検証を分ける。

## モック方針

- **domain の trait（Repository、ドメインサービス）と usecase の trait（QueryService、購読の配信の口）**: `mockall` でモック生成可、または手書きの fake 実装
- **Tauri API**: テストでは呼ばない設計を優先。やむを得ない場合は薄いラッパー化してテスト側で差し替え
- **git2**: 実 git リポジトリを `tempdir` 上に作り、統合テストとして書く
- **外部 HTTP API**: 偽サーバを立てる
- **長時間プロセス（PTY, Provider CLI）**: 単体テストでは呼ばず、Terminal Surfaceのbyte I/O、AgentSession lifecycle、Provider lifecycle signalを個別に単体テストし、実processは別途手動・統合テストで検証する。Provider conversationをReleashのMessage modelへ変換するtestは作らない

## テストヘルパー

ドメインごとに `test_helpers.rs` をテストファイルと同じディレクトリに置ける。

## CI

CI と同じコマンドをローカルでも使う。詳細はプロジェクト root の `AGENTS.md` を参照。
