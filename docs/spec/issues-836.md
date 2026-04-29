## 要求

**種別**: バグ修正
**現在の挙動**: `@` でファイル名を指定する際、ファイル名に日本語・全角スペース・中黒（・）など正規表現 `\w` に一致しない文字が含まれていると、ファイル指定が外れる（パースされずファイルコンテキストがAIエージェントに送信されない）
**期待する挙動**: 日本語を含むファイル名でも `@` 参照が正しく機能し、ファイルコンテキストがAIエージェントに送信される
**再現手順**:
1. 日本語名を含むファイルを用意する（例: `docs/decisions/PJT-1691/Gitフロー　・デプロイサイクル見直し.md`）
2. AgentChatで `@` を入力し、当該ファイルを選択する
3. 行番号指定（`:L50`）も付与してメッセージを送信する
4. ファイルコンテキストがAIエージェントに送信されない
**背景**: `file_mention.rs` の正規表現 `[\w./_\-\[\]]+` の `\w` がASCII範囲（`[a-zA-Z0-9_]`）のみに対応しており、日本語やUnicodeのワード文字をカバーしていない

## 振る舞い定義

```gherkin
Feature: 非ASCII文字を含むファイルの@メンション参照
  ファイル名に日本語・全角記号等の非ASCII文字を含むファイルを
  @メンション構文で参照し、ファイルコンテキストをAIエージェントに送信できる

  Rule: ファイル選択結果の構造化データ伝達
    Scenario: ポップアップで選択したファイルパスが構造化データとしてAgentに渡される
      Given ユーザーが@メンションで "docs/Gitフロー　・デプロイサイクル見直し.md" を選択した
      When メッセージを送信する
      Then 選択されたファイルパスが構造化データとしてRust側のAgent処理に渡される
      And Rust側でファイル内容が読み取られる
      And ファイルの内容がuser messageのコンテキストとしてAgentプロンプトに含まれる

    Scenario: 行番号指定付きのファイル選択が構造化データとして渡される
      Given ユーザーが@メンションで "docs/Gitフロー.md" を選択し ":L50" を付与した
      When メッセージを送信する
      Then ファイルパスと行番号50が構造化データとしてRust側に渡される
      And 該当行の内容がuser messageのコンテキストとしてAgentプロンプトに含まれる

    Scenario: 行範囲指定付きのファイル選択が構造化データとして渡される
      Given ユーザーが@メンションで "docs/Gitフロー.md" を選択し ":L10-L20" を付与した
      When メッセージを送信する
      Then ファイルパスと行範囲10-20が構造化データとしてRust側に渡される
      And 該当行範囲の内容がuser messageのコンテキストとしてAgentプロンプトに含まれる

  Rule: 表示用メンションバッジの分離
    Scenario: 日本語ファイル名がメンションバッジとして表示される
      Given 送信済みメッセージに "docs/Gitフロー.md" のメンションが含まれている
      When メッセージを表示する
      Then メンション部分がバッジとして視覚的に区別して表示される
```

## 実装仕様

**対応方針**: Agent送信時のファイルコンテキスト構築を、テキスト埋め込み→正規表現パース方式から構造化データ直接伝達方式に変更する。表示用パースの正規表現はUnicode対応に修正する。

**対象コンポーネント**:

### Rust側

- **`src-tauri/src/file_mention.rs`**:
  - `MentionReference` 構造体を追加（`file_path: String, start_line: Option<u32>, end_line: Option<u32>`）
  - `resolve_from_references()` 新設: 構造化データから直接ファイル読み込み・`<file_context>` ブロック構築
  - `resolve_mentions_or_fallback()` は `resolve_from_references` のフォールバック付きラッパーとして残す（エラー時にログ出力し元のcontentを返す）
  - `MENTION_RE` の文字クラスをASCII空白限定に修正（`parse_display_mentions` の表示用パースで全角スペースを含むパスを切り捨てないため）
  - `parse_mentions` / `resolve_mentions_internal` は削除（呼び出し元がなくなる）

- **`src-tauri/src/agent_sdk.rs`**:
  - `send_agent_message` コマンドに `mentions: Option<Vec<MentionReference>>` パラメータ追加
  - `PendingMessage` に `mentions` フィールド追加
  - `resolve_mentions_or_fallback` を経由して `resolve_from_references` を呼び出す（フォールバック付きで後方互換性を維持）

- **`src-tauri/src/diff_comment_sender.rs`**:
  - `format_comments_for_agent` → `build_mentions_from_comments`: `DiffComment[]` を `MentionReference[]` に変換
  - コメントテキストは別途渡す
  - `SendDiffCommentsResult` に `mentions: Vec<MentionReference>` フィールド追加

### フロントエンド側

- **`src/types/session.ts`**: `MentionReference` 型追加
- **`MessageInput.tsx`**: 選択ファイルを `MentionReference[]` stateに保持し `onSend` に渡す
- **`useAgentChat.ts`** / **`useSessionStore.ts`**: `mentions` 引数を中継
- **`MainLayout.tsx`**: `sendAgentMessageRef` の型を拡張（`mentions` 引数追加）
- **`ReviewPanel.tsx`**: コメント送信時に `mentions` + コメントテキストを渡す

### 表示

- `content` テキストには `@filepath` がそのまま残る（変更なし）
- `parse_display_mentions` の正規表現をUnicode対応に修正 → 日本語ファイル名もバッジ表示される

**検討した代替案**:
- 正規表現の文字クラスをUnicode対応に拡張するのみの案: Agent送信側もテキストパースに依存し続ける。全角スペース等でメンション区間の終端判定が困難。OSS（Continue.dev, Zed, Void）全件が構造化データ方式を採用。却下

**影響するテスト**:
- `file_mention.rs`: `resolve_from_references` の新規テスト、`parse_display_mentions` のUnicode対応テスト
- `diff_comment_sender.rs`: `build_mentions_from_comments` のテスト
- `MessageInput.test.tsx`: メンション選択時に構造化データ保持のテスト
- `useDiffComments.test.ts`: コメント送信時のメンションデータ伝達テスト
