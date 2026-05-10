## 要求

**種別**: リファクタリング + 新機能
**ゴール**: 現在 `agent_sdk.rs` に密結合している Claude Agent SDK Bridge の通信ロジックを `AgentBackend` trait で抽象化し、複数のエージェントバックエンドを切り替え可能にする基盤を構築する。Phase 1 完了時に以下が成立していれば成功:
- 既存の Claude Agent SDK 連携が trait 経由で動作し、現行と同等の機能を維持する
- バックエンド一覧が UI（デスクトップ・リモート）に表示され、選択を切り替えられる
- config.toml でデフォルトバックエンドを指定できる
- ChatSession に使用バックエンドが記録される
- 新しいバックエンドの追加が trait 実装のみで可能な構造になっている

**背景**: 現在のエージェント実行レイヤーは Claude Agent SDK（Node.js Bridge プロセス経由）に固定されている。マルチエージェント対応（Milestone: マルチエージェント対応）の Phase 1 として、抽象化レイヤーを導入し、Phase 2 で Codex Backend、Phase 3 でマルチエージェント協調を実現するための土台を作る。

### 対象ユーザー

Releash デスクトップアプリおよびリモートアクセス（モバイル）のユーザー

### 利用シーン

- デスクトップ: チャットパネルヘッダーでバックエンドを選択し、セッション開始時に指定したバックエンドでエージェントを実行する
- リモート: WebSocket 経由でバックエンド一覧を取得し、選択・切り替えを行う

### スコープ

#### Rust 側

- `AgentBackend` trait 定義（start_session, send_message, interrupt, respond_permission, available_models, close_session）
- 共通型定義（`SessionConfig`, `SessionHandle`, `AgentMessage`, `AgentEvent`, `ToolPermissionRequest` 等）
- `agent_sdk.rs` → `backends/claude.rs` リファクタリング（trait 実装としてアダプタ化）
- `AgentBackendRegistry`: バックエンド登録・取得・一覧
- config.toml `[agents]` セクション（バックエンド設定、デフォルト選択）
- ChatSession に `backend_id` フィールドを追加し、セッション作成時に記録・永続化
- Tauri コマンド: バックエンド一覧取得、セッション開始時のバックエンド指定
- WebSocket プロトコル拡張: バックエンド一覧取得・選択メッセージの追加

#### フロントエンド側（デスクトップ）

- チャットパネルヘッダーにバックエンド選択 UI
- `useAgentChat` のバックエンド対応（セッション開始時にバックエンド指定）
- バックエンドごとのモデルリスト取得

#### フロントエンド側（リモート）

- WebSocket 経由でバックエンド一覧取得・選択が可能な UI

### AgentBackend trait（案）

```rust
#[async_trait]
pub trait AgentBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle>;
    async fn send_message(&self, session: &SessionHandle, message: AgentMessage) -> Result<EventStream>;
    async fn interrupt(&self, session: &SessionHandle) -> Result<()>;
    async fn respond_permission(&self, session: &SessionHandle, response: PermissionResponse) -> Result<()>;
    async fn available_models(&self) -> Result<Vec<ModelInfo>>;
    async fn close_session(&self, session: &SessionHandle) -> Result<()>;
}
```

### 設計決定事項

| 項目 | 決定 |
|------|------|
| 抽象化方式 | Rust trait `AgentBackend` |
| Claude アダプタ | 既存 `agent_sdk.rs` の Bridge 通信をアダプタとして切り出し |
| レジストリ | バックエンド名 → アダプタインスタンスのマップ管理 |
| 設定 | config.toml の `[agents]` セクションで利用可能バックエンドを定義 |
| UI | チャットパネルヘッダーにバックエンド選択ドロップダウン |
| セッション永続化 | ChatSession に `backend_id` フィールドを追加 |
| リモート対応 | WebSocket プロトコル拡張でバックエンド一覧・選択を提供 |
| エラー時挙動 | バックエンドが利用不可の場合はエラー表示のみ（自動フォールバックなし） |
| trait非対応の既存コマンド | `scan_slash_commands`, `prepare_image_attachment`, `set_agent_model` 等のバックエンド固有コマンドは `backends/claude.rs` に配置し、Claudeバックエンド固有のTauriコマンドとして維持する。共通化が必要になった場合は後続Phaseで対応 |

### 制約

- Phase 1 では Claude バックエンドのみ実装する（Codex は Phase 2: #881）
- 既存の動作を破壊しないこと（リファクタリングであり機能退行は不可）

### 影響範囲

- `src-tauri/src/agent_sdk.rs` → `src-tauri/src/backends/claude.rs` へ移動・リファクタリング
- `src-tauri/src/agent_status.rs` — trait 抽象化への適応
- `src-tauri/src/session/mod.rs` — ChatSession に `backend_id` 追加
- `src-tauri/src/config.rs` — `[agents]` セクション追加
- `src-tauri/src/protocol/agent.rs` — 共通型定義の見直し
- `src-tauri/src/ws_server/` — バックエンド関連 WebSocket メッセージ追加
- `src/hooks/useAgentChat.ts` — バックエンド指定対応
- `src/components/panels/AgentChatPanel` — バックエンド選択 UI
- `src/remote/` — リモート側バックエンド選択 UI

### 依存関係

- なし（Phase 1: 最初に着手）
- 後続: #881 Codex Backend、#882 マルチエージェント協調

## 振る舞い定義

```gherkin
Feature: AgentBackend 抽象化レイヤー
  エージェントバックエンドを切り替え可能にし、
  複数のAIエージェントサービスを統一的に利用できる基盤を提供する

  Background:
    Given Claudeバックエンドがレジストリに登録されている

  Rule: バックエンド一覧の取得
    レジストリに登録されたバックエンドは一覧として取得できる

    Scenario: 登録済みバックエンドの一覧を取得する
      When バックエンド一覧を取得する
      Then Claudeバックエンドが一覧に含まれている

  Rule: デフォルトバックエンドの決定
    config.tomlの設定に基づきデフォルトバックエンドが決定される

    Scenario: config.tomlでデフォルトバックエンドが指定されている
      Given config.tomlのデフォルトバックエンドが"claude"に設定されている
      When バックエンドを指定せずにセッションを作成する
      Then セッションのバックエンドは"claude"である

    Scenario: デフォルトバックエンド未指定時はconfig.toml記載順で最初の利用可能バックエンドを使用する
      Given config.tomlにデフォルトバックエンドが設定されていない
      When バックエンドを指定せずにセッションを作成する
      Then セッションのバックエンドはconfig.toml [agents] セクションの記載順で最初の利用可能なバックエンドである

    Scenario: config.toml記載順で最初のバックエンドが初期化に失敗している場合は次の利用可能なバックエンドが使用される
      Given config.tomlにデフォルトバックエンドが設定されていない
      And config.toml [agents] セクションの最初のバックエンドがレジストリ初期化に失敗している
      When バックエンドを指定せずにセッションを作成する
      Then セッションのバックエンドは記載順で次に利用可能なバックエンドである

    Scenario: config.tomlのデフォルトバックエンドがレジストリに存在しない場合はエラーを返す
      Given config.tomlのデフォルトバックエンドが"nonexistent"に設定されている
      And "nonexistent"バックエンドがレジストリに登録されていない
      When バックエンドを指定せずにセッションを作成する
      Then エラーが返される
      And 他のバックエンドへの自動フォールバックは行われない

    Scenario: config.toml [agents]セクションにバックエンドが1つも定義されていない場合はエラーを返す
      Given config.toml [agents]セクションにバックエンドが定義されていない
      When バックエンドを指定せずにセッションを作成する
      Then エラーが返される

  Rule: セッション作成時のバックエンド指定と記録
    セッション開始時にバックエンドを指定でき、選択はセッションに永続化される

    Scenario: バックエンドを指定してセッションを開始する
      When "claude"バックエンドを指定してセッションを開始する
      Then セッションは"claude"バックエンドで作成される
      And セッションのbackend_idに"claude"が記録される

    Scenario: セッション復元時に記録されたバックエンドが使用される
      Given backend_idが"claude"のセッションが保存されている
      When そのセッションを復元する
      Then "claude"バックエンドでエージェントプロセスが起動する

    Scenario: backend_idを持たない既存セッションを復元する
      Given backend_idが記録されていないセッションが保存されている
      When そのセッションを復元する
      Then デフォルトバックエンドでエージェントプロセスが起動する
      And セッションのbackend_idにデフォルトバックエンドのIDが記録される

  Rule: バックエンド利用不可時の挙動
    利用不可のバックエンドが選択された場合、エラーを返しフォールバックしない

    Scenario: 利用不可のバックエンドでセッション開始を試みる
      Given "codex"バックエンドがレジストリに登録されていない
      When "codex"バックエンドを指定してセッションを開始する
      Then エラーが返される
      And 他のバックエンドへの自動フォールバックは行われない

    Scenario: セッション復元時に記録されたバックエンドが利用不可になっている
      Given backend_idが"codex"のセッションが保存されている
      And "codex"バックエンドがレジストリに登録されていない
      When そのセッションを復元する
      Then エラーが返される
      And 他のバックエンドへの自動フォールバックは行われない

  Rule: バックエンド実行時エラーの挙動
    レジストリに登録済みだが実行時にセッション開始が失敗した場合、エラーを返す

    Scenario: バックエンドのセッション開始が実行時に失敗する
      Given Claudeバックエンドがレジストリに登録されている
      And Bridgeプロセスの起動に失敗する状態である
      When "claude"バックエンドを指定してセッションを開始する
      Then セッション開始エラーが返される
      And エラーにはバックエンド名と失敗原因が含まれる
      And 他のバックエンドへの自動フォールバックは行われない

  Rule: バックエンドごとのモデル一覧取得
    各バックエンドが提供する利用可能モデルを取得できる

    Scenario: バックエンドの利用可能モデル一覧を取得する
      Given Claudeバックエンドがレジストリに登録されている
      When Claudeバックエンドのモデル一覧を取得する
      Then Claudeバックエンドが提供するモデルが一覧に含まれる

    Scenario: バックエンドのモデル一覧取得に失敗した場合はエラーが返される
      Given Claudeバックエンドがレジストリに登録されている
      And モデル一覧の取得が失敗する状態である
      When Claudeバックエンドのモデル一覧を取得する
      Then エラーが返される

  Rule: バックエンド選択UIの表示
    利用可能なバックエンドがチャットパネルヘッダーに表示される

    Scenario: バックエンド一覧がチャットパネルに表示される
      Given 利用可能なバックエンドが存在する
      When チャットパネルを表示する
      Then ヘッダーにバックエンド選択UIが表示される
      And 利用可能なバックエンドが選択肢として列挙される

    Scenario: デフォルトバックエンドが初期選択される
      Given config.tomlのデフォルトバックエンドが"claude"に設定されている
      When チャットパネルを表示する
      Then バックエンド選択UIで"claude"が選択されている

    Scenario: セッション実行中にバックエンドを切り替えても現行セッションは影響を受けない
      Given Claudeバックエンドでセッションがストリーミング中である
      When バックエンド選択UIで別のバックエンドを選択する
      Then 現行セッションはClaudeバックエンドで継続する
      And 次に新規セッションを作成する際に選択されたバックエンドが使用される

  Rule: リモートアクセスでのバックエンド操作
    WebSocket経由でバックエンドの一覧取得と選択が可能

    Scenario: WebSocket経由でバックエンド一覧を取得する
      Given リモートクライアントがWebSocketで接続している
      When バックエンド一覧リクエストを送信する
      Then 利用可能なバックエンド一覧がレスポンスとして返される

    Scenario: WebSocket経由でバックエンドを指定してセッションを開始する
      Given リモートクライアントがWebSocketで接続している
      When "claude"バックエンドを指定してセッション開始リクエストを送信する
      Then セッションは"claude"バックエンドで作成される
      And セッションのbackend_idに"claude"が記録される

  Rule: 既存Claude連携の機能維持
    リファクタリング後もClaude Agent SDK経由の全操作がtrait実装を通じて動作する

    Scenario: Claudeバックエンドでメッセージを送受信する
      Given Claudeバックエンドでセッションが開始されている
      When メッセージを送信する
      Then エージェントからストリーミングレスポンスを受信する

    Scenario: Claudeバックエンドでターンを中断する
      Given Claudeバックエンドがストリーミング中である
      When 中断を実行する
      Then ストリーミングが停止する

    Scenario: Claudeバックエンドでツール実行許可に応答する
      Given Claudeバックエンドがツール実行許可を要求している
      When 許可応答を送信する
      Then エージェントがツール実行を継続する

    Scenario: Claudeバックエンドのセッションを終了する
      Given Claudeバックエンドでセッションが開始されている
      When セッションを終了する
      Then セッション状態がClosedに遷移する
      And バックエンドのリソースが解放される
```

## アーキテクチャ概要

### 責務配置

- **backends/ (新規モジュール `src-tauri/src/backends/`)**: AgentBackend trait 定義、AgentBackendRegistry（登録・取得・一覧・デフォルト解決）、各バックエンド実装の格納場所 / Tauri コマンドの定義やセッション永続化は担当しない
- **backends/claude.rs (agent_sdk.rs からリファクタリング)**: Claude Agent SDK Bridge プロセスの起動・stdin/stdout 通信・イベント変換を AgentBackend trait 実装として提供 / バックエンド選択やレジストリ管理は担当しない
- **session/ (既存拡張)**: ChatSession に backend_id を記録・永続化する。セッション作成・復元時に backend_id を受け渡す / バックエンドの起動・通信は担当しない
- **config.rs (既存拡張)**: `[agents]` セクションの定義（利用可能バックエンド一覧、デフォルト指定） / バックエンドインスタンスのライフサイクル管理は担当しない
- **agent_status.rs (既存・最小変更)**: trait 抽象化後もセッション状態の集約・Tauri イベント/WebSocket 通知を継続。TurnPhase 等の型参照元が変わる可能性がある / バックエンド固有の状態管理は担当しない
- **protocol/agent.rs (既存拡張)**: バックエンド一覧・選択に関する共通型の追加 / trait 定義そのものは backends/ に配置
- **ws_server/ (既存拡張)**: バックエンド一覧取得・選択の WebSocket メッセージのルーティング・ハンドリングを追加 / バックエンドの実装やライフサイクルは担当しない
- **useAgentChat (既存拡張)**: バックエンド指定でのセッション開始、選択状態の reducer 管理、バックエンド一覧の取得・保持 / バックエンド通信ロジックは担当しない
- **AgentChatPanel (既存拡張)**: チャットパネルヘッダーにバックエンド選択 UI を表示 / 選択ロジックは useAgentChat に委譲
- **remote/ (既存拡張)**: WebSocket 経由でバックエンド一覧取得・選択 UI を提供 / バックエンド通信は担当しない

### データ/通信フロー

- **バックエンド一覧取得（デスクトップ）**: AgentChatPanel → `invoke("list_agent_backends")` → AgentBackendRegistry.list() → バックエンド情報一覧を返却
- **バックエンド一覧取得（リモート）**: RemoteUI → WebSocket(バックエンド一覧リクエスト) → ws_server/routing → AgentBackendRegistry.list() → WebSocket レスポンス
- **セッション開始（デスクトップ・バックエンド指定）**: UI → `invoke("start_agent_session", { backend_id })` → Registry.get(backend_id) → backend.start_session() → SessionStore.save(session + backend_id) → AgentStatusCenter.update → Tauri emit
- **セッション開始（リモート・バックエンド指定）**: RemoteUI → WebSocket(セッション開始リクエスト + backend_id) → ws_server/routing → Registry.get(backend_id) → backend.start_session() → SessionStore.save(session + backend_id) → AgentStatusCenter.update → WebSocket レスポンス
- **メッセージ送信**: UI → `invoke("send_agent_message")` → セッションから backend_id 取得 → Registry.get(backend_id) → backend.send_message() → EventStream → Tauri emit → UI
- **セッション復元**: UI → `invoke("restore_session")` → SessionStore.get → backend_id 取得 → Registry.get(backend_id) → backend.start_session()
- **デフォルトバックエンド解決**: セッション作成時に backend_id 未指定 → AppConfig.[agents].default 参照 → 未設定なら config.toml `[agents]` セクションの記載順で最初の利用可能なバックエンド

### 状態Owner

- **バックエンド登録情報（どのバックエンドが利用可能か）**: AgentBackendRegistry (`Arc`, AppState として Tauri に登録)
- **バックエンド設定（config.toml `[agents]`）**: AppConfig (`Arc<AppConfig>`, AppState)
- **セッションの backend_id（どのバックエンドで作成されたか）**: SessionStore（ファイル永続化 + メモリキャッシュ）
- **UI 上の選択バックエンド**: useAgentChat reducer 内の状態（フロントエンド）
- **各バックエンドのプロセス状態（Bridge プロセス等）**: 各 AgentBackend 実装内部（現在の AgentProcessMap に相当）
- **セッション集約状態（Running/Done/Error/Waiting）**: AgentStatusCenter (`Arc`, AppState)

### 境界

- **AgentBackend trait**: バックエンドに要求する操作の契約。全バックエンドがこの trait を満たす。trait に含まれない操作（バックエンド固有のオプション等）は各実装内部に閉じる
- **Tauri コマンド層**: フロントエンドが呼べる操作の境界。フロントエンドは backend_id を文字列で指定するだけで、バックエンドの内部実装を知らない
- **WebSocket メッセージ**: リモートクライアントが呼べる操作の境界。デスクトップ向け Tauri コマンドと同等の操作を WebSocket メッセージで提供する
- **config.toml `[agents]`**: 利用可能バックエンドの宣言的定義。レジストリの初期化はこの設定に基づく
- **フロントエンド ↔ Rust**: フロントエンドは backend_id・バックエンド表示情報（名前等）のみ扱い、通信プロトコルやプロセス管理の詳細に依存しない

### 実装に委ねること

- SessionConfig, SessionHandle, AgentMessage, AgentEvent 等の共通型の具体的なフィールド構成
- AgentBackendRegistry の内部データ構造（HashMap, Vec, IndexMap 等）
- Tauri コマンドの正確な関数名・引数名
- WebSocket メッセージの具体的な JSON フィールド名・構造
- バックエンド選択 UI の具体的なコンポーネント分割（ドロップダウン実装方法等）
- agent_sdk.rs から claude.rs へのリファクタリング時の関数分割・ヘルパー関数名
- テストケースの具体的な配置・ケース名・モック構成
- config.toml `[agents]` セクションのバックエンド記載順を保持するための構造選択（toml クレートの Table は BTreeMap のためキー順序がアルファベット順になる。記載順に依存するフォールバックロジックがあるため、toml_edit やカスタムパーサ等で挿入順を保持する方法の選択が必要）
- AgentProcess の内部フィールドの再編成
