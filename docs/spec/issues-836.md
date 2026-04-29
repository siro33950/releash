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

**対応方針**: Agent送信時のファイルコンテキスト構築を、テキスト埋め込み→正規表現パース方式から構造化データ直接伝達方式に変更する。表示用バッジ生成も構造化データ（mentions配列）をソースオブトゥルースとし、`parse_display_mentions` Tauriコマンドによるテキストパースは廃止する。

**対象コンポーネント**:

### Rust側

- **`src-tauri/src/session/mod.rs`**:
  - `ChatMessage` に `mentions: Option<Vec<crate::file_mention::MentionReference>>` フィールド追加（永続化対応）
  - `add_message_internal` に `mentions` パラメータ追加
  - humanメッセージ送信時にフロントエンドから渡されたmentionsをそのまま保存

- **`src-tauri/src/file_mention.rs`**:
  - `MentionReference` 構造体（`file_path: String, start_line: Option<u32>, end_line: Option<u32>`）
  - `resolve_from_references()`: 構造化データから直接ファイル読み込み・`<file_context>` ブロック構築
  - `resolve_mentions_or_fallback()`: `resolve_from_references` のフォールバック付きラッパー
  - 削除: `parse_display_mentions` コマンド、`DisplayPart` enum、`MENTION_RE` 正規表現、`is_valid_mention_position` 関数（テキストパース方式の全廃）

- **`src-tauri/src/agent_sdk.rs`**:
  - `send_agent_message` コマンドに `mentions: Option<Vec<MentionReference>>` パラメータ追加
  - `PendingMessage` に `mentions` フィールド追加
  - `resolve_mentions_or_fallback` を経由して `resolve_from_references` を呼び出す
  - humanメッセージ保存時にmentionsをChatMessageに含める

- **`src-tauri/src/lib.rs`**:
  - `file_mention::parse_display_mentions` のコマンド登録を削除

- **`src-tauri/src/diff_comment_sender.rs`**:
  - `format_comments_for_agent` → `build_mentions_from_comments`: `DiffComment[]` を `MentionReference[]` に変換
  - コメントテキストは別途渡す
  - `SendDiffCommentsResult` に `mentions: Vec<MentionReference>` フィールド追加

### フロントエンド側

- **`src/types/session.ts`**: `ChatMessage` / `LegacyChatMessage` に `mentions?: MentionReference[]` フィールド追加
- **`src/hooks/useSessionStore.ts`**: `convertLegacyMessage` で `mentions` を引き継ぎ
- **`MessageInput.tsx`**: 選択ファイルを `MentionReference[]` stateに保持し `onSend` に渡す。`syncMentionsWithText` をテキスト出現順にイテレーションするよう修正（C2対応）
- **`useAgentChat.ts`** / **`useSessionStore.ts`**: `mentions` 引数を中継
- **`MainLayout.tsx`**: `sendAgentMessageRef` の型を拡張（`mentions` 引数追加）
- **`ReviewPanel.tsx`**: コメント送信時に `mentions` + コメントテキストを渡す

### 表示

- `content` テキストには `@filepath` がそのまま残る（変更なし）
- **`StreamMessage.tsx`**: `invoke("parse_display_mentions")` を完全削除。`buildDisplayParts()` 関数で `mentions` prop から同期的にバッジ生成（表示用フォーマットの範疇でフロントエンドに配置）
- **`AgentChatPanel.tsx`**: `<StreamMessage>` に `mentions={msg.mentions}` を渡す

**検討した代替案**:
- 正規表現の文字クラスをUnicode対応に拡張するのみの案: Agent送信側もテキストパースに依存し続ける。全角スペース等でメンション区間の終端判定が困難。OSS（Continue.dev, Zed, Void）全件が構造化データ方式を採用。却下
- `format_comment_text` でファイルパスをエスケープする案: テキストパースの複雑化を避けるため却下。VS Code / Cursor / Continue.dev 等のOSSが構造化データSoT方式を採用しており、業界標準に合わせる

**`syncMentionsWithText` の順序修正（C2）**:
- 変更前: `refs` 配列の順でイテレーションし、テキスト中の出現順と食い違う可能性があった
- 変更後: テキスト中の `@filePath` 出現順で正規表現マッチをイテレーションし、各マッチの `filePath` に対応する `refs` をカウンターで消費する方式に修正

**影響するテスト**:
- `file_mention.rs`: `parse_display_mentions_*` テスト9件を削除、`resolve_from_references` テストは既存のまま維持
- `session/mod.rs` / `session/store.rs`: `ChatMessage` 手動構築箇所に `mentions: None` 追加
- `StreamMessage.test.tsx`: `parse_display_mentions` モック削除、`mentions` propベースのバッジ表示テストに書き換え
- `MessageInput.test.tsx`: メンション選択時に構造化データ保持のテスト
