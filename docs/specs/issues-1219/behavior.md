# Behavior

撤去（デッドコード削除・機能廃止）タスクのため、本書は「撤去後に外部から観測される振る舞い」を定義する。新機能の追加や挙動変更はなく、観測可能な差分は「自前 MCP 機能が消えること」と「それ以外が従来どおりであること（回帰なし）」に集約される。

実装経路（どのファイル・モジュールを削除するか）は振る舞いではないため本書には含めない。撤去対象の内訳は `requirements.md` を参照する。

## 仮定

- 「自前 MCP サーバ」とは、Releash がエージェントへ worktree / file tool を公開する MCP サーバ（facet 1）を指す。これが提供するツールは `mcp__releash__*` という名前空間を持つ。
- 「MCP 設定 UI」とは、デスクトップの設定モーダル内に存在する `MCP` タブ（facet 3）を指す。
- 「facet 4」とは、エージェント自身が利用する外部 MCP ツール（例: `mcp__notion__get_page`）の表示用分類を指し、本撤去の対象外である。
- 「従来どおり」「回帰なし」とは、撤去前に成立していた振る舞いが撤去後も同一であることを指す。
- ビルド・テスト・lint の緑は受け入れ条件（プロセス上の完了条件）であり、本書では「成果物が壊れていないこと」を表す観測点として `Rule: 撤去後も成果物は健全である` に含める。

## Feature: 自前 MCP ドメインの撤去

CLI への全面移行で用済みとなった Releash 自前 MCP 機能（facet 1/2/3）を撤去する。
撤去後、エンドユーザーから見て自前 MCP 機能は消滅し、それ以外の機能は一切変化しない。

### Background

```gherkin
Background:
  Given Releash デスクトップアプリが撤去後のコードからビルドされている
  And ユーザーがアプリを起動している
```

## Rule: 撤去後、自前 MCP サーバは起動せず mcp__releash__* tool を提供しない

```gherkin
Scenario: アプリ起動時に自前 MCP サーバが起動しない
  When アプリが起動を完了する
  Then 自前 MCP サーバは起動しない

Scenario: エージェントから mcp__releash__* tool が見えない
  Given アプリが起動を完了している
  When エージェントが利用可能なツール一覧を取得する
  Then `mcp__releash__*` 名前空間のツールは一覧に含まれない

Scenario: エージェントへの MCP 設定注入が行われない
  Given アプリが起動を完了している
  When エージェントセッションが開始される
  Then Releash 自前 MCP サーバへの接続設定はエージェントへ注入されない
```

## Rule: 撤去後、MCP 設定 UI は表示されない

```gherkin
Scenario: 設定モーダルに MCP タブが存在しない
  Given ユーザーが設定モーダルを開いている
  When ユーザーがタブ一覧を確認する
  Then `MCP` タブは表示されない

Scenario: 既存の他タブは従来どおり表示・操作できる
  Given ユーザーが設定モーダルを開いている
  When ユーザーが `Notion` タブを選択する
  Then Notion 設定の入力・保存が従来どおり行える
```

## Rule: facet 4 のツール表示分類は従来どおり機能する（撤去対象外）

```gherkin
Scenario Outline: エージェントが使う外部 MCP ツールが mcp として分類表示される
  Given エージェントセッションのアクティビティログを表示している
  When エージェントが <tool> を呼び出す
  Then そのツール活動は従来どおり `mcp` カテゴリとして分類・表示される

  Examples:
    | tool                    |
    | mcp__notion__get_page   |
    | mcp__server__some_tool  |
```

## Rule: Notion 設定および MCP 以外の全機能は回帰しない

```gherkin
Scenario: Notion 設定が従来どおり動作する
  When ユーザーが Notion 設定を編集して保存する
  Then 撤去前と同一の結果になる

Scenario: MCP 以外のデスクトップ機能が従来どおり動作する
  When ユーザーが Git 操作・エディタ・ターミナル・ソース管理・コメント等の既存機能を利用する
  Then いずれも撤去前と同一の振る舞いになり、回帰は生じない
```

## Rule: 撤去後も成果物は健全である

```gherkin
Scenario: Rust 成果物が緑である
  When `cargo clippy -- -D warnings` と `cargo test` を src-tauri/ で実行する
  Then どちらも警告・失敗なく成功する

Scenario: フロントエンド成果物が緑である
  When `pnpm lint` / `pnpm build` / `pnpm test` をプロジェクトルートで実行する
  Then いずれも警告・失敗なく成功する

Scenario: 撤去に伴う未使用参照が残らない
  When ビルドおよび lint を実行する
  Then 撤去ドメインのみが使用していた未使用インポート・参照・依存に起因する警告やエラーは発生しない
```

## Open Questions

なし。
