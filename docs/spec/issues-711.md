## 要求

**種別**: 新機能
**ゴール**: AgentChatの入力欄にモデル選択ドロップダウンを追加し、使用するAIモデルをチャット中に切り替えられるようにする
**背景**: コード生成・レビュー・質問応答など用途によって最適なモデルが異なるため、ユーザーがその場でモデルを選択できる必要がある
**制約**:
- モデルリストはAgent SDK の `initializationResult().models` から動的に取得する
- モデル切り替えはAgent SDK の `setModel()` メソッドで実現する
- 参考UI: Cursorのモデル選択ドロップダウン（Auto, GPT, Claude, Gemini等の切り替え）

## 振る舞い定義

```gherkin
Feature: AgentChat モデル選択
  AgentChatの入力欄でAIモデルを切り替え、用途に応じて最適なモデルを使えるようにする

  Rule: モデルリストはAgent SDKから動的に取得される
    Scenario: AgentChatの初期化時にモデルリストを取得する
      Given AgentChatセッションが開始される
      When Agent SDKとの接続が確立される
      Then 利用可能なモデルのリストがAgent SDKから取得される

  Rule: ユーザーはモデルを選択できる
    Scenario: モデルを切り替える
      Given モデルリストが取得済みである
      When ユーザーがモデル選択ドロップダウンからモデルを選ぶ
      Then 選択したモデルが現在のモデルとして設定される

    Scenario: モデル未選択時はSDKのデフォルトが使われる
      Given モデルリストが取得済みである
      And ユーザーがモデルを明示的に選択していない
      When ユーザーがメッセージを送信する
      Then モデル未指定でAgent SDKにリクエストが送られ、SDK側のデフォルトモデルが使用される

  Rule: モデル選択はセッションに紐づいて保持される
    Scenario: セッション中のモデル選択が維持される
      Given ユーザーがモデルを選択済みである
      When ユーザーが同一セッション内で複数のメッセージを送信する
      Then すべてのメッセージが選択済みのモデルで送信される

    Scenario: セッション移動後に戻ってもモデル選択が保持される
      Given ユーザーがセッションAでモデルを選択済みである
      And ユーザーが別のセッションBに移動する
      When ユーザーがセッションAに戻る
      Then セッションAで選択していたモデルが復元される

    Scenario: ワークスペース移動後に戻ってもモデル選択が保持される
      Given ユーザーがワークスペースXのセッションでモデルを選択済みである
      And ユーザーが別のワークスペースYに移動する
      When ユーザーがワークスペースXのセッションに戻る
      Then 選択していたモデルが復元される

    Scenario: 新しいセッションではデフォルトから開始する
      Given 既存のセッションでモデルが選択されている
      When ユーザーが新しいセッションを作成する
      Then モデル選択はデフォルト（SDK任せ）で開始される

  Rule: 選択されたモデルはメッセージ送信時に使用される
    Scenario: モデルを指定してメッセージを送信する
      Given ユーザーがモデルを選択済みである
      When ユーザーがメッセージを送信する
      Then 選択されたモデルでAgent SDKにリクエストが送られる

  Rule: 現在のモデル選択状態がUIに表示される
    Scenario: 選択中のモデルが入力欄に表示される
      Given モデルリストが取得済みである
      And ユーザーがモデルを選択済みである
      When ユーザーが入力欄を見る
      Then 現在選択されているモデル名がドロップダウンに表示される

    Scenario: モデル未選択時の表示
      Given モデルリストが取得済みである
      And ユーザーがモデルを明示的に選択していない
      When ユーザーが入力欄を見る
      Then デフォルトであることを示す表示がドロップダウンに表示される
```

## 実装仕様

**対応方針**: モデル選択機能を実現するために、ブリッジスクリプト・Rustバックエンド・フロントエンドの3層に変更を加える。全ての状態管理・判断ロジックはRust側に集約し、フロントエンドは表示とユーザー操作の中継のみを担う。参考実装: [zed-industries/claude-agent-acp](https://github.com/zed-industries/claude-agent-acp)。

**対象コンポーネント**:

### 1. ブリッジスクリプト (`src-tauri/resources/claude-sdk-bridge.mjs`)
- SDK初期化時に `initializationResult.models` からモデルリストを取得し `{ type: "supported_models", models: [...] }` で emit（Zed実装と同じく `initializationResult()` 経由。`supportedCommands()` と同タイミング）
- `setModel` コマンドハンドラーを追加（`setMode` と同パターン: stdinからJSON受信 → `currentQuery.setModel(modelId)` 呼び出し）

### 2. Rustバックエンド（状態管理・判断の中心）
- `src-tauri/src/agent_sdk.rs`:
  - `AgentProcess` に `available_models: Vec<ModelInfo>` と `selected_model: Option<String>` を追加
  - stdoutパーサーに `supported_models` メッセージのハンドリングを追加 → `agent-models-updated` イベントでフロントエンドに push
  - `set_agent_model` Tauriコマンドを追加: Bridge へ `setModel` コマンド送信 + `AgentProcess.selected_model` 更新 + DB永続化 + `agent-models-updated` イベント emit を一括実行
- `src-tauri/src/session/mod.rs`:
  - `ChatSession` に `selected_model: Option<String>` フィールドを追加（`#[serde(skip_serializing_if = "Option::is_none", default)]`）
  - 既存JSONの後方互換は `Option` + `default` で確保
- `src-tauri/src/lib.rs`: 新コマンド `set_agent_model` を登録

### 3. フロントエンド（UIインターフェースのみ）
- `src/types/session.ts`: `ModelInfo` 型を追加（`{ value: string, displayName: string }`）
- `src/components/panels/AgentChatPanel/ModelSelector.tsx` (新規): `ModeSelector` と同じ shadcn/ui `DropdownMenu` パターン。props: `models`, `currentModelId`, `onModelChange`, `disabled`。デフォルト表示: "Auto"
- `src/components/panels/AgentChatPanel/MessageInput.tsx`: `ModelSelector` を `ModeSelector` と送信ボタンの間に配置
- `src/hooks/useAgentSdkListeners.ts`: `agent-models-updated` イベントのリスナーを追加。受け取った値をそのまま状態に反映（判断ロジックなし）
- `src/hooks/useAgentChat.ts`: `setModel` 関数を追加（`invoke("set_agent_model", ...)` を呼ぶだけ。状態更新はイベント経由でRustから受け取る）
- `src/components/panels/AgentChatPanel/AgentChatPanel.tsx`: props接続

**検討した代替案**:
- フロントエンドReducerで `availableModels` / `sessionModels` を管理する案: Zed実装ではバックエンドが単一の真実の源であり、UIは表示のみ。Rust側に集約する方がセッション復元・DB永続化との整合性が取りやすく、フロントエンドの責務を最小化できる。却下。
- `query()` の `model` オプションで毎ターン指定する案: SDKに `setModel()` が専用APIとして存在し、Zed実装でも `setModel()` を使用。セッション途中の切替に対応でき、Bridgeの `promptGenerator` 改修が不要。却下。

**リスク**:
- Agent SDKのバージョンアップで `initializationResult` / `setModel()` のAPIが変更される可能性 → 型定義をSDKの `ModelInfo` に準拠させることで追従しやすくする
- 既存セッションのJSONに `selected_model` がない → `Option<String>` + `#[serde(default)]` で後方互換を確保

**影響するテスト**:
- フロントエンド:
  - `ModelSelector.test.tsx`: ドロップダウンの開閉、モデル選択時のコールバック、デフォルト表示
  - `MessageInput.test.tsx`: ModelSelectorの存在確認
- Rust:
  - `session/mod.rs`: `ChatSession` のシリアライズ/デシリアライズテスト（`selected_model` あり/なし）
