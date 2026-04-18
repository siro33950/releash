## 要求

**種別**: バグ修正
**現在の挙動**: AgentChatパネルのヘッダー領域でドラッグ・ダブルクリックしても反応がなく、ウィンドウの移動・最大化/復元ができない
**期待する挙動**: AgentChatパネルのヘッダー領域でドラッグするとウィンドウが移動し、ダブルクリックするとウィンドウが最大化/復元される（他の画面と同じ挙動）
**再現手順**:
1. AgentChatパネルを開く
2. ヘッダー領域をドラッグする → ウィンドウが移動しない
3. ヘッダー領域をダブルクリックする → ウィンドウが最大化/復元されない
**背景**: Tauriのウィンドウ操作（ドラッグ移動・ダブルクリック最大化）はヘッダー領域に `data-tauri-drag-region` 属性を付与することで実現されるが、AgentChatのヘッダーにこの属性が欠落しているか、イベントが阻害されている可能性がある

## 振る舞い定義

```gherkin
Feature: AgentChatパネルのウィンドウ操作
  AgentChatパネルのヘッダー領域からウィンドウの移動・最大化/復元ができる

  Rule: ヘッダー領域のドラッグでウィンドウが移動する
    Scenario: ヘッダーの空き領域をドラッグするとウィンドウが移動する
      Given AgentChatパネルが表示されている
      When ヘッダーの空き領域をドラッグする
      Then ウィンドウが移動する

    Scenario: セッションタブをドラッグしてもウィンドウは移動しない
      Given AgentChatパネルに複数のセッションタブがある
      When セッションタブをドラッグする
      Then タブの並べ替えが行われる
      And ウィンドウは移動しない

  Rule: ヘッダー領域のダブルクリックでウィンドウが最大化/復元される
    Scenario: ヘッダーの空き領域をダブルクリックするとウィンドウが最大化される
      Given AgentChatパネルが表示されている
      And ウィンドウが通常サイズである
      When ヘッダーの空き領域をダブルクリックする
      Then ウィンドウが最大化される

    Scenario: 最大化状態でヘッダーの空き領域をダブルクリックするとウィンドウが復元される
      Given AgentChatパネルが表示されている
      And ウィンドウが最大化されている
      When ヘッダーの空き領域をダブルクリックする
      Then ウィンドウが通常サイズに復元される
```

## 実装仕様

**対応方針**: 振る舞い定義を実現するために、AgentChatPanel のヘッダーコンテナに対して `data-tauri-drag-region` 属性の付与で対応する。ViewToolbar・RightPanelHeader の既存パターンを踏襲する。

**対象コンポーネント**:
- `src/components/panels/AgentChatPanel/AgentChatPanel.tsx`: ヘッダーのコンテナ div に `data-tauri-drag-region` 属性を追加。TabsList とボタン群の間に `data-tauri-drag-region` 付きのフレックス空き領域（`<div data-tauri-drag-region className="flex-1" />`）を挿入し、ドラッグ可能な空き領域を確保する。

**検討した代替案**:
- ヘッダーコンテナのみに属性付与（空き領域 div なし）: TabsList・ボタンがヘッダー全幅を占有するため、ドラッグ可能な空き領域が確保できない可能性があるため却下

**影響するテスト**:
- `AgentChatPanel.test.tsx`: `data-tauri-drag-region` 属性の存在を確認するテストを追加（ViewToolbar.test.tsx・RightPanelHeader.test.tsx の既存パターンに準拠）
