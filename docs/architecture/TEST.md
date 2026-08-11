# テスト 規約

## 配置

実装と同じディレクトリに `*_test.rs` を配置する。

```
src/domain/repository/entities/
├── branch.rs
└── branch_test.rs
```

```rust
// branch_test.rs
#[cfg(test)]
mod branch_tests {
    use super::*;  // branch.rs を参照

    #[test]
    fn test_ブランチ判定_アップストリーム有りでトラッキング扱い() {
        // ...
    }
}
```

`branch.rs` 側で `mod branch_test;` のような宣言は不要（テストは `#[cfg(test)]` で `mod` 宣言する形式の場合）か、あるいは `branch.rs` 末尾に `#[cfg(test)] mod tests;` で test ファイルを取り込む形式にする。プロジェクトのコンパイル方針に合わせる。

## 命名規則

### テスト関数

```
test_{業務機能}_{条件と期待結果}
```

業務機能は日本語で、テスト失敗時に意図が伝わる名前にする。

```rust
#[test]
fn test_ブランチ作成_空文字エラー() { ... }

#[test]
fn test_差分Hunk計算_変更行のみがhunkになる() { ... }
```

### テストモジュール

```
{implementation_name}_tests
```

```rust
#[cfg(test)]
mod branch_repository_impl_tests {
    use super::*;
    // ...
}
```

## レイヤー別の必須／柔軟

| レイヤー | テスト | 理由 |
|---|---|---|
| `domain/` | **必須** | ビジネスロジックの中核 |
| `usecase/` | **必須** | 業務手順の正しさを担保 |
| `adaptor/gateway/` | **必須** | 外部システムとの境界、モデル変換の検証 |
| `adaptor/controller/command/` | 柔軟 | Tauri 依存で書きにくい場合は省略可 |
| `adaptor/controller/handler/` | 柔軟 | WebSocket 依存で書きにくい場合は省略可 |
| `adaptor/presenter/` | 柔軟 | 表示整形のみ、必要に応じて |
| `infrastructure/` | 柔軟 | 外部世界の都合をそのまま扱う層。判断も変換も持たないため、統合テストで検証 |
| `other/` | 柔軟 | 横断的関心事 |

「柔軟」のレイヤーも、テストを書ける範囲では書く。書きにくいから書かない判断は許容するが、書きやすくする工夫（インターフェース抽出等）も検討する。

## テスト構造（Given-When-Then）

```rust
#[tokio::test]
async fn test_ブランチ作成_既存名でエラー() {
    // Given
    let repo = create_test_repo().await;
    create_branch(&repo, "feature/foo").await.unwrap();

    // When
    let result = create_branch(&repo, "feature/foo").await;

    // Then
    assert!(matches!(result, Err(DomainError::AlreadyExists)));
}
```

## モック方針

- **ドメイン層の Repository / Gateway trait**: `mockall` でモック生成可、または手書きの fake 実装
- **Tauri API**: テストでは呼ばない設計を優先。やむを得ない場合は薄いラッパー化してテスト側で差し替え
- **git2**: 実 git リポジトリを `tempdir` 上に作って統合テスト寄りに書く
- **外部 HTTP API**: `wiremock` 等で偽サーバを立てる
- **長時間プロセス（PTY, Provider CLI）**: 単体テストでは呼ばず、Terminal Surfaceのbyte I/O、AgentSession lifecycle、Provider lifecycle signalを個別に単体テストし、実processは別途手動・統合テストで検証する。Provider conversationをReleashのMessage modelへ変換するtestは作らない

## テストヘルパー

ドメインごとに `test_helpers.rs` をテストファイルと同じディレクトリに配置できる：

```rust
// src/adaptor/gateway/repository/test_helpers.rs
pub async fn create_test_repo() -> tempfile::TempDir { /* ... */ }
pub fn create_test_branch() -> Branch { /* ... */ }
```

## 統合テスト

`src-tauri/tests/` に配置。

- 複数レイヤーをまたぐシナリオ
- 実 git リポジトリ操作
- 実 WebSocket 通信

## CI

CI と同じコマンドをローカルでも使う。詳細はプロジェクト root の `CLAUDE.md` を参照。
