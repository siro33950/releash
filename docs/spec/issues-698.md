## 要求

**種別**: 新機能
**ゴール**: AgentChatのメッセージ入力エリアで、画像をドラッグ&ドロップまたはクリップボードからペースト（Ctrl+V / Cmd+V）して、コンテキストとして送信できるようにする
**背景**: Cursor、Claude Code等の主要AIコーディングツールが採用しているパターンであり、スクリーンショットやUI画像を直接共有してAIとの対話に活用するUXを実現する
**制約**:
- 対応画像形式: Claude Codeが認識できる形式に準拠
- サイズ上限: Claude Code側の制限に従う（Releash側では制限しない）
**送信方式**: テキスト+画像の同時送信、画像単体送信の両方に対応

## 振る舞い定義

```gherkin
Feature: AgentChat 画像添付
  AgentChatのメッセージ入力エリアで画像をドラッグ&ドロップまたはペーストして、
  コンテキストとしてAIに送信する。

  Rule: 画像はドラッグ&ドロップで添付できる
    Scenario: 画像ファイルをドロップして添付する
      Given メッセージ入力エリアが表示されている
      When ユーザーが画像ファイルをメッセージ入力エリアにドロップする
      Then 画像が添付リストに追加される

  Rule: 画像はクリップボードからペーストで添付できる
    Scenario: クリップボードの画像をペーストして添付する
      Given メッセージ入力エリアにフォーカスがある
      When ユーザーがクリップボードから画像をペーストする
      Then 画像が添付リストに追加される

  Rule: 添付された画像はプレビュー表示される
    Scenario: 添付済み画像のプレビューを表示する
      Given 画像が添付リストに追加されている
      When ユーザーがメッセージ入力エリアを見る
      Then 添付画像のサムネイルプレビューが表示される

  Rule: 添付された画像は削除できる
    Scenario: 添付済み画像を削除する
      Given 画像が添付リストに追加されている
      When ユーザーが添付画像を削除する
      Then 画像が添付リストから除去される

  Rule: テキストと画像を同時に送信できる
    Scenario: テキストと画像を一緒に送信する
      Given メッセージ入力エリアにテキストが入力されている
      And 画像が添付リストに追加されている
      When ユーザーがメッセージを送信する
      Then テキストと画像がAIに送信される
      And 添付リストがクリアされる

  Rule: 画像のみでも送信できる
    Scenario: 画像だけを送信する
      Given メッセージ入力エリアにテキストが入力されていない
      And 画像が添付リストに追加されている
      When ユーザーがメッセージを送信する
      Then 画像がAIに送信される
      And 添付リストがクリアされる

  Rule: 複数画像を添付できる
    Scenario: 複数の画像を添付する
      Given 画像が添付リストに追加されている
      When ユーザーが別の画像を追加する
      Then 両方の画像が添付リストに含まれる

  Rule: 送信済み画像はチャット履歴に表示される
    Scenario: 送信したメッセージに画像が表示される
      Given ユーザーが画像付きメッセージを送信した
      When ユーザーがチャット履歴を見る
      Then 送信メッセージ内に画像が表示される

  Rule: 非対応形式のファイルは添付できない
    Scenario: 非画像ファイルをドロップする
      Given メッセージ入力エリアが表示されている
      When ユーザーが画像以外のファイルをドロップする
      Then ファイルは添付されない
```

## 実装仕様

**対応方針**: 振る舞い定義（D&D・ペースト・プレビュー・削除・送信・チャット履歴表示）を実現するために、Rust（agent_sdk / session）に画像処理ロジックを集約し、フロントエンド（MessageInput / StreamMessage）はUI表示とinvoke呼び出しのみ行う。Bridgeは `content` 配列に画像ブロックを追加する拡張を行う。

**対象コンポーネント**:
- `src-tauri/src/agent_sdk.rs`: `prepare_image_attachment` コマンド新設（バイナリ受取→バリデーション→Base64エンコード→返却）、`send_agent_message` 拡張（画像データを受取→セッション保存→Bridgeへ送信）
- `src-tauri/src/session/mod.rs`: `MessagePart` に `Image` バリアントを追加、Humanメッセージを `parts` 付きで保存
- `src-tauri/resources/claude-sdk-bridge.mjs`: `toUserMessage(text, images)` に拡張し、`content` 配列に `{ type: "image", source: { type: "base64", ... } }` を含める
- `src/components/panels/AgentChatPanel/MessageInput.tsx`: D&D/ペーストイベント受付→`invoke("prepare_image_attachment")` 呼出→プレビューUI表示→削除UI
- `src/components/panels/AgentChatPanel/StreamMessage.tsx`: Humanメッセージのpartsに画像バリアントがある場合 `<img>` で表示
- `src/hooks/useSessionStore.ts`: `sendAgentMessage` のシグネチャ拡張（テキスト+画像配列をinvokeに渡す）
- `src/types/session.ts`: `MessagePart` に `image` バリアントの型定義を追加

**ロジック配置（Rust-first原則）**:
- バリデーション（画像形式判定、非対応形式の拒否）: Rust
- Base64エンコード: Rust
- MIMEタイプ判定: Rust
- UI状態管理（添付リスト）: フロントエンド（UIの責務）
- プレビューURL組立て（`data:${mime};base64,...`）: フロントエンド（表示フォーマットの責務）
- D&D/ペーストイベントの受付: フロントエンド（入力受付の責務）

**検討した代替案**:
- `@tauri-apps/plugin-clipboard-manager` 導入: Webview標準の `ClipboardEvent` で画像取得可能なため新規プラグイン導入のコストに見合わない → 却下
- 画像を一時ファイルとして保存→パスで渡す: メモリ上で完結する方がシンプル。Claude APIもBase64を期待している → 却下

**リスク**:
- 大きな画像のinvoke通信コスト: Uint8ArrayをRustに渡す際のシリアライゼーション。Claude Code SDK側のサイズ制限に委ねるため、Releash側では制限しない（要求の制約通り）

**影響するテスト**:
- Rust（cargo test）: `prepare_image_attachment` のバリデーション・エンコードテスト、`MessagePart::Image` のserdeテスト
- フロントエンド（Vitest）: MessageInput のD&D/ペースト/プレビュー/削除UIテスト、StreamMessage の画像パート表示テスト
