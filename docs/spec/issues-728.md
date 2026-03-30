## 要求

**種別**: バグ修正
**現在の挙動**: AgentChatパネルでAIの回答中に表示されるURLをクリックすると、ReleashのWebView内でそのURLが開いてしまう
**期待する挙動**: URLをクリックするとOSのデフォルトブラウザ（Chrome, Safari等）で開く
**再現手順**:
1. AgentChatパネルでAIに質問する
2. AIの回答にURLが含まれている
3. そのURLをクリックする
4. ReleashのWebView内でページが表示されてしまう（期待: デフォルトブラウザで開く）
**背景**: ReleashはTauriベースのデスクトップアプリであり、WebView内でURLが開くとナビゲーションが困難になる。外部URLはOSのデフォルトブラウザで開くのが自然なUX。

## 振る舞い定義

```gherkin
Feature: AgentChatパネルの外部リンク処理
  AgentChatパネルでAIの回答に含まれるURLをクリックした際に、
  OSのデフォルトブラウザで開く

  Rule: 外部URLはOSのデフォルトブラウザで開く
    Scenario: AIの回答に含まれるURLをクリックする
      Given AgentChatパネルにURLを含むAIの回答が表示されている
      When ユーザーがURL（リンク）をクリックする
      Then OSのデフォルトブラウザでそのURLが開かれる

    Scenario: AIの回答に含まれるURLをクリックしてもWebView内で遷移しない
      Given AgentChatパネルにURLを含むAIの回答が表示されている
      When ユーザーがURL（リンク）をクリックする
      Then Releashアプリ内のWebViewでページ遷移が発生しない
```

## 実装仕様

**対応方針**: 外部URLをOSのデフォルトブラウザで開くために、`StreamMessage.tsx` の `react-markdown` に `components` プロップでカスタム `<a>` コンポーネントを追加し、`@tauri-apps/plugin-opener` の `openUrl()` を呼び出す。

**対象コンポーネント**:
- `src/components/panels/AgentChatPanel/StreamMessage.tsx`: `<Markdown>` に `components={{ a: ... }}` を追加し、クリック時に `e.preventDefault()` + `openUrl(href)` を実行

**検討した代替案**:
- Tauriのナビゲーションイベントでグローバルに制御する案: WebView全体のリンク遷移をフックする方法。影響範囲が広すぎ、意図しない副作用のリスクがあるため却下
- `markdownConfig.ts` にrehypeプラグインとして追加する案: マークダウン設定を共通化できるが、`openUrl` はTauri API依存であり、マークダウン設定レイヤーに混ぜるのは責務の分離に反するため却下

**リスク**:
- `openUrl()` の失敗時（URLが不正等）: try-catchで例外を握りつぶし、フォールバック不要（ブラウザが開かないだけ）

**影響するテスト**:
- `StreamMessage.test.tsx`（新規または既存）: カスタムリンクコンポーネントがレンダリングされること、クリック時に `openUrl` が呼ばれることを検証
