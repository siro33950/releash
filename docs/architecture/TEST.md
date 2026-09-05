# テスト 規約

## 配置

実装と同じディレクトリに `<impl>_test.rs` を置き、`<impl>.rs` の末尾で `#[path]` を指定して取り込む。ファイル名は `<impl>_test.rs`、テストモジュール名は `<impl>_tests` とする。

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
| `adaptor/controller/api/` | 柔軟 | HTTP 依存で書きにくい場合は省略可 |
| `adaptor/presenter/` | 柔軟 | 表示整形のみ、必要に応じて |
| `infrastructure/` | 柔軟 | 外部世界の都合をそのまま扱う層。判断も変換も持たないため、統合テストで検証 |
| `other/` | 柔軟 | 横断的関心事 |

「柔軟」のレイヤーも、テストを書ける範囲では書く。書きにくいから書かない判断は許容するが、書きやすくする工夫（インターフェース抽出等）も検討する。

## テスト構造

Given / When / Then をコメントで区切り、前提・操作・検証を分ける。

## モック方針

- **ドメイン層の Repository / Gateway trait**: `mockall` でモック生成可、または手書きの fake 実装
- **Tauri API**: テストでは呼ばない設計を優先。やむを得ない場合は薄いラッパー化してテスト側で差し替え
- **git2**: 実 git リポジトリを `tempdir` 上に作って統合テスト寄りに書く
- **外部 HTTP API**: 偽サーバを立てる
- **長時間プロセス（PTY, Provider CLI）**: 単体テストでは呼ばず、Terminal Surfaceのbyte I/O、AgentSession lifecycle、Provider lifecycle signalを個別に単体テストし、実processは別途手動・統合テストで検証する。Provider conversationをReleashのMessage modelへ変換するtestは作らない

## テストヘルパー

ドメインごとに `test_helpers.rs` をテストファイルと同じディレクトリに置ける。

## 統合テスト

`src-tauri/tests/` に配置する。

- 複数レイヤーをまたぐシナリオ
- 実 git リポジトリ操作
- 実 local API 通信

## CI

CI と同じコマンドをローカルでも使う。詳細はプロジェクト root の `AGENTS.md` を参照。
