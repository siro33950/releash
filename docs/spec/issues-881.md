## 要求

**種別**: 新機能
**ゴール**: Codex TypeScript SDK（`@openai/codex-sdk`）を使った Node.js ブリッジを作成し、AgentBackend として統合する。完了時に以下が成立していれば成功:
- NewSession 作成直後から初回メッセージ送信前まで、チャット入力 UI 内の Agent selector で「Codex」を選択し、GPT 系モデル（o3, GPT-5.4 等）でチャット・コード生成ができる
- Claude と同じブリッジパターン（stdin/stdout JSONL）で Codex SDK と通信する
- セッション継続（`resumeThread`）で前の対話コンテキストが維持される
- モデル変更（`setModel`）が反映される
- 中断（`interrupt`）でターンが停止する
- ブリッジプロセスのクラッシュおよび SDK エラー（認証失敗・レートリミット等）発生時にセッションがエラー状態に遷移し、エラーメッセージが表示され、次のメッセージ送信でブリッジが自動復旧する

**背景**: マルチエージェント対応（Milestone: マルチエージェント対応）の Phase 2。#880 で AgentBackend trait による抽象化レイヤーが導入されたが、現在 Claude のみが唯一のバックエンドである。Codex Backend を追加することで、GPT 系モデルの利用が可能になり、ユーザーがタスクに応じてエージェントを使い分けられるようになる。

### 対象ユーザー

Releash デスクトップアプリおよびリモートアクセス（モバイル）のユーザー

### 利用シーン

- デスクトップ: NewSession 作成直後のチャット入力 UI 内 Agent selector で「Codex」を選択し、GPT 系モデルでコード生成・チャットを行う
- リモート: WebSocket 経由で Codex バックエンドを選択し、モバイルからも利用可能

### スコープ

#### 前提対応: ブリッジ共通基盤の抽出（#880 追加対応）

現在 `backends/claude.rs` に密結合している以下を共通モジュールに抽出し、Claude・Codex 両方から利用可能にする:

| 項目 | 現状 | 対応 |
|------|------|------|
| **プロセス管理** | `AgentProcess`, `spawn_bridge_process()` 等が `claude.rs` 内 | 共通モジュールに抽出 |
| **状態管理** | `BridgeState`, `TurnPhase` が `claude.rs` 内 | 共通モジュールに抽出 |
| **ストリーミング蓄積** | `accumulate_sdk_message()` が `claude.rs` 内 | 共通モジュールに抽出 |
| **AgentsSection 設定** | `default: Option<String>` のみ | バックエンド固有設定（CLI パス等）を格納できる構造に拡張 |
| **イベントモデル** | Claude SDK 固有 | 各ブリッジが SDK 固有イベントを共通ブリッジプロトコルに変換して出力する |
| **モデル選択コマンド** | `set_agent_model` が Claude の `AgentProcessMap` に直接アクセス | セッションの `backend_id` に基づいて正しいバックエンドにディスパッチする共通化が必要 |

#### Node.js ブリッジ（`codex-sdk-bridge.mjs`）

- `@openai/codex-sdk` の `Codex` クラスを使ったスレッド管理
- Codex CLI 実行体はアプリに同梱しない。Claude ブリッジが `pathToClaudeCodeExecutable: "claude"` でユーザー環境の `claude` CLI を使うのと同様に、Codex ブリッジは `new Codex({ codexPathOverride: "codex" })` でユーザー環境の `codex` CLI を使う
- Rust からの stdin コマンドを受信し SDK API に変換:
  - `init` → `codex.startThread({ workingDirectory })` or `codex.resumeThread(threadId)`
  - `message` → `thread.runStreamed(prompt)` → イベントを stdout に転送
  - `interrupt` → 中断処理
  - `setModel` → モデル変更
  - `close` → セッション終了
- SDK のストリーミングイベントを Claude 互換の stdout JSON に変換:
  - `item.completed` (agentMessage) → `assistant` メッセージ
  - `item.agentMessage.delta` → `stream_event` (text_delta)
  - `turn.completed` → `turn_complete`

#### Rust 側（`backends/codex.rs`）

- `AgentBackend` trait 実装
- ブリッジプロセスの起動（`node codex-sdk-bridge.mjs`）
- 共通基盤（プロセス管理・状態遷移・ストリーミング蓄積）の利用
- Codex 固有の設定（model, approvalPolicy, sandbox）の管理

#### 設定（`config.rs`）

- `AgentsSection` の拡張:
  ```toml
  [agents]
  default = "claude"

  [agents.codex]
  model = "o3"                       # デフォルトモデル
  cli_path = "codex"                  # 任意: Codex CLI のパス（未指定時は PATH 上の codex）
  ```

#### モデル選択の共通化

現在 `set_agent_model` Tauri コマンドは Claude の `AgentProcessMap` に直接アクセスしており、バックエンド非依存ではない。以下の対応が必要:

- `set_agent_model`: セッションの `backend_id` を参照し、対応するバックエンドのプロセスに `setModel` コマンドをディスパッチする
- `available_models`: 各バックエンドのブリッジから `supported_models` イベントで受信したモデル一覧を、バックエンドごとに管理する
- ModelSelector UI: `get_session.availableModels` を唯一のモデル一覧 source として、アクティブセッションの Agent 選択値に応じたモデルリストを表示する

#### フロントエンド

- Agent selector に「Codex」選択肢を追加（BackendRegistry 経由で自動表示のため、追加コード不要の想定）。Agent selector は上部タブバーではなくチャット入力 UI 内に表示し、未送信セッションでのみ有効化する
- Codex 選択時は Claude 用の `ModeSelector` を使わず、既存 selector と同じ外観の Codex 専用ドロップダウンを表示する。トリガーには `Read only` / `Workspace` / `Full access` を表示し、メニュー内で `Sandbox` と `Approval`（`Ask` / `Never`）を選択できるようにする。ただし `Approval` を選べるのは `Workspace` の時だけで、`Read only` と `Full access` は `Never` 固定にする。Codex UI では `Plan` / `Bypass` という表示名は使わない
- Codex ストリーミングレスポンスの表示対応（ブリッジが Claude 互換フォーマットに変換するため、大部分は既存コードで動作する想定）
- ModelSelector: `get_session.availableModels` に応じたモデル一覧を表示（backend list API にはモデル一覧責務を持たせない）

### 設計決定事項

| 項目 | 決定 |
|------|------|
| 通信方式 | Claude と同じ Node.js ブリッジパターン（stdin/stdout JSONL） |
| ツール実行 | Codex 側で完結。`@openai/codex-sdk@0.128.0` の公開 `ThreadEvent` に対話的な承認要求イベントはないため、Codex では Releash の承認ダイアログに接続せず、permission mode を SDK の `approvalPolicy` / `sandboxMode` にマッピングする |
| Codex Permission UI | Claude の `Code / Ask / Plan / Bypass` とは表示名を分けつつ、既存 selector と同じドロップダウン外観にする。Codex は `Read only` / `Workspace` / `Full access` を選択肢として表示し、メニュー内で `Sandbox` と `Approval`（`Ask` / `Never`）を選べるようにする。`Approval` 選択は `Workspace` 時のみ有効。`Plan` / `Bypass` 表示名は使わない |
| セッション管理 | `codex.startThread()` / `codex.resumeThread(threadId)` |
| ストリーミング | `thread.runStreamed(prompt)` の async generator からイベント受信 |
| イベント変換 | ブリッジ側で Claude 互換フォーマットに変換し、Rust 側の処理を共通化 |
| 可用性 | `@openai/codex-sdk` をプロジェクト依存として常にインストール。Claude の `@anthropic-ai/claude-agent-sdk` と同じパターンで常に `available: true` |
| Codex CLI | アプリには同梱しない。Claude と同様に外部 CLI 前提とし、既定では PATH 上の `codex` を `codexPathOverride: "codex"` で SDK に渡す。将来的に `[agents.codex].cli_path` で明示パスを指定できる |
| 認証 | Codex SDK の標準動作（環境変数 `OPENAI_API_KEY` 等、CLI 認証の流用）に委ねる。APIキー管理・設定UIはスコープ外 |

### ブリッジプロトコル

Claude ブリッジと同じ stdin/stdout JSONL プロトコルに準拠する。

#### Rust → Bridge (stdin)

| コマンド | 用途 | パラメータ |
|---------|------|-----------|
| `init` | セッション開始 | `cwd`, `sessionId?`（再開時）, `permissionMode?`, `model?`, `systemPrompt?` |
| `message` | メッセージ送信 | `prompt`, `images?` |
| `interrupt` | 中断 | — |
| `setModel` | モデル変更 | `modelId` |
| `close` | 終了 | — |

> **備考**: `sessionId` は共通プロトコルのパラメータ名。Codex ブリッジ内部では `threadId` にマッピングして `resumeThread()` に渡す。

#### Bridge → Rust (stdout)

| イベント | 用途 | 備考 |
|---------|------|------|
| `session_ready` | 初期化完了 | `session_id`（threadId） |
| `supported_models` | モデル一覧 | `models[]` |
| `stream_event` | テキストストリーミング | Claude 互換 `content_block_delta` 形式に変換 |
| `assistant` | アシスタントメッセージ | tool_use ブロック含む |
| `turn_complete` | ターン完了 | `exit_code` |
| `error` | エラー | `message` |

### 制約

- ブリッジが Claude 互換フォーマットに変換するため、Rust 側のストリーミング処理は可能な限り既存の共通基盤を再利用する
- Codex SDK のツール実行は Codex 側で完結する。対話的な承認要求・プラン承認は SDK がイベントを公開するまで Codex 側の対象外とし、Claude 側の既存共通フローで扱う
- `@openai/codex-sdk` はプロジェクト依存として常にインストールされる（Claude の `@anthropic-ai/claude-agent-sdk` と同じパターン）
- Codex CLI binary/vendor は Tauri resources に含めない。実行時はユーザー環境の `codex` CLI を使用し、見つからない場合は Codex backend の起動エラーとしてユーザーに表示する
- 認証は Codex SDK の標準動作（環境変数 `OPENAI_API_KEY` 等、CLI 認証の流用）に委ねる。APIキーの管理・設定UIはスコープ外

### 生成物配置ポリシー

- 生成物はすべて `src-tauri/generated/` 配下に配置し、コミットしない
- Remote app の Vite build output は `src-tauri/generated/remote/` に出力する
- Node.js bridge bundle は `src-tauri/generated/bridges/` に出力する
- `src-tauri/resources/` はソースまたは手書き静的リソースのみを置き、remote build output や `*.bundled.mjs` を置かない
- `.gitignore` では `src-tauri/generated/` を丸ごと ignore し、`src-tauri/resources/` 配下の生成物を隠す ignore ルールは置かない

### 影響範囲

- `src-tauri/src/backends/claude.rs` — 共通部分の抽出（AgentProcess, BridgeState, TurnPhase, accumulate_sdk_message 等）
- `src-tauri/src/backends/mod.rs` — 共通モジュール追加、Codex バックエンド登録
- `src-tauri/src/backends/codex.rs` — 新規: Codex AgentBackend 実装
- `src-tauri/src/backends/bridge_common.rs`（仮） — 新規: ブリッジ共通基盤
- `src-tauri/resources/codex-sdk-bridge.mjs` — 新規: Codex SDK ブリッジ
- `src-tauri/generated/bridges/` — bridge bundle 生成先（ignore対象）
- `src-tauri/generated/remote/` — Remote app build output 生成先（ignore対象）
- `vite.config.remote.ts` — Remote app の生成先設定
- `scripts/build-bridge.mjs` — bridge bundle の生成先設定
- `src-tauri/tauri.conf.json` — bundle resources の参照先設定
- `src-tauri/src/config.rs` — AgentsSection 拡張
- `src-tauri/Cargo.toml` — 依存追加（必要に応じて）
- `package.json` — `@openai/codex-sdk` 依存追加

### 依存関係

- #880 AgentBackend 抽象化レイヤー（Phase 1 完了済み）
- 後続: #882 マルチエージェント協調

### 検証方法

- NewSession 作成直後にチャット入力 UI 内 Agent selector で Codex を選択して初回メッセージを送信し、レスポンスがストリーミング表示される
- セッション継続（`resumeThread`）で前の対話コンテキストが維持される
- Codex セッションの ModelSelector に Codex 固有のモデル一覧（o3, GPT-5.4 等）が表示される
- モデル変更（`setModel`）が Codex セッション内で反映される
- Claude セッションに切り替えた場合、ModelSelector が Claude のモデル一覧に切り替わる
- 中断（`interrupt`）でターンが停止する
- ブリッジプロセスのクラッシュおよび SDK エラー時にセッションがエラー状態に遷移し、エラーメッセージが表示される
- エラー状態のセッションで新しいメッセージを送信し、ブリッジが自動復旧してメッセージが正常に処理される

### 参考資料

- [Codex SDK ドキュメント](https://developers.openai.com/codex/sdk)
- [Codex SDK TypeScript リポジトリ](https://github.com/openai/codex/tree/main/sdk/typescript)
- [Codex CLI Going Native (codex-rs)](https://github.com/openai/codex/discussions/1174)
- 既存の Claude ブリッジ実装: `src-tauri/resources/claude-sdk-bridge.mjs`, `src-tauri/src/backends/claude.rs`

## 振る舞い定義

```gherkin
Feature: Codex Backend
  Codex TypeScript SDK を使った AgentBackend として、
  GPT 系モデルによるチャット・コード生成を可能にする。

  # ── バックエンド可用性 ──

  Rule: Codex バックエンドはプロジェクト依存として常に利用可能である

    Scenario: アプリ起動時に Codex バックエンドが登録される
      When アプリが起動する
      Then Codex バックエンドが利用可能として登録される

  Rule: 利用可能なバックエンドがチャット入力 UI 内の Agent selector に表示される

    Scenario: 複数のバックエンドが利用可能な場合に選択肢が表示される
      Given Claude と Codex の両方が利用可能である
      When NewSession 作成直後のチャット入力 UI を表示する
      Then Agent selector に Claude と Codex の選択肢が表示される

  Rule: セッションのバックエンドは初回メッセージ送信時に固定される

    Scenario: NewSession 作成直後は Agent を変更できる
      Given 新しい未送信セッションが作成されている
      When チャット入力 UI の Agent selector で Codex を選択する
      Then セッションの backend_id が Codex に更新される
      And Codex のモデル一覧が ModelSelector に表示される
      And Agent process は起動されない

    Scenario: 初回メッセージ送信時に現在の Agent 選択値で固定される
      Given NewSession の Agent selector で Codex が選択されている
      When 最初のメッセージを送信する
      Then Codex バックエンドで Agent が起動する
      And Agent selector は変更できなくなる

    Scenario: 既存セッションではバックエンドを変更できない
      Given Codex バックエンドで作成されたセッションが存在する
      When そのセッションを表示する
      Then Agent selector はそのセッションのバックエンドを変更できない

  # ── チャット・コード生成 ──

  Rule: Codex バックエンドでメッセージを送信するとターンが実行される

    Scenario: Codex バックエンドでメッセージを送信する
      Given Codex バックエンドが選択されたセッションが存在する
      When メッセージを送信する
      Then Codex SDK 経由でターンが開始される
      And ターン完了時にレスポンスが確定する

  Rule: Codex のストリーミングレスポンスがリアルタイムに表示される

    Scenario: ストリーミング中のレスポンスが表示される
      Given Codex バックエンドでターンが実行中である
      When ストリーミングイベントが受信される
      Then レスポンスがリアルタイムに表示更新される

  # ── セッション継続 ──

  Rule: 既存セッションを再開すると前のスレッドが復元される

    Scenario: 対話履歴のあるセッションを再開する
      Given 過去に Codex バックエンドで対話したセッションが存在する
      When そのセッションを再開する
      Then 前のスレッドのコンテキストが維持された状態でセッションが開始される

    Scenario: スレッド復元に失敗した場合に新しいスレッドで開始される
      Given 復元対象のスレッドが無効になっている
      When セッションを再開する
      Then 新しいスレッドとしてセッションが開始される
      And 無効なスレッド ID がクリアされる

  # ── モデル選択 ──

  Rule: モデルを変更すると以降のターンに反映される

    Scenario: Codex セッション内でモデルを変更する
      Given Codex バックエンドのセッションが存在する
      When モデルを変更する
      Then 以降のターンが新しいモデルで実行される

  Rule: アクティブセッションのバックエンドに応じたモデル一覧が表示される

    Scenario: Codex セッションがアクティブな場合に Codex のモデル一覧が表示される
      Given Codex バックエンドのセッションがアクティブである
      When ModelSelector を表示する
      Then Codex 対応のモデル一覧が表示される

    Scenario: 別バックエンドのセッションに切り替えるとモデル一覧が切り替わる
      Given Codex セッションで Codex のモデル一覧が表示されている
      When Claude バックエンドのセッションに切り替える
      Then ModelSelector が Claude のモデル一覧に更新される

  # ── ツール実行承認（バックエンド共通） ──

  Rule: ツール実行時に承認要求がフロントエンドに転送される

    Scenario Outline: SDK がツール実行の承認を求める
      Given <backend> バックエンドでターンが実行中である
      When SDK がツール実行の承認を要求する
      Then 承認ダイアログが表示される

      Examples:
        | backend |
        | Claude  |

    Scenario: ツール実行を許可する
      Given 承認ダイアログが表示されている
      When ユーザーが許可する
      Then ツールが実行される
      And 実行結果がストリーミング表示される

    Scenario: ツール実行を拒否する
      Given 承認ダイアログが表示されている
      When ユーザーが拒否する
      Then ツールがスキップされる

  # ── プランモード（バックエンド共通） ──

  Rule: 対話的なプランモードは承認イベントを公開するバックエンドで動作する

    Scenario Outline: SDK がプランモードに移行する
      Given <backend> バックエンドのセッションが存在する
      When SDK がプランモードを開始する
      Then プランモードUIが表示される

      Examples:
        | backend |
        | Claude  |

    Scenario: プランの承認後にプランモードが終了する
      Given セッションでプランモードが表示されている
      When ユーザーがプランを承認する
      Then プランモードが終了する
      And 承認されたプランに基づいて実行が開始される

  # ── パーミッションモード（バックエンド共通） ──

  Rule: バックエンドに応じたパーミッションUIでモードを切り替えられる

    Scenario: Claude では ModeSelector でパーミッションモードを変更する
      Given Claude バックエンドのセッションが存在する
      When ModeSelector でパーミッションモードを変更する
      Then 以降のツール実行に新しいモードが適用される

    Scenario: Codex では専用の permission dropdown が表示される
      Given Codex バックエンドのセッションが存在する
      When チャット入力 UI を表示する
      Then Claude 用の ModeSelector は表示されない
      And Codex 用の permission dropdown が表示される
      And Codex dropdown には Read only, Workspace, Full access, Sandbox, Ask, Never が表示される

    Scenario: Codex の Read only は編集不能な計画・調査用として動作する
      Given Codex バックエンドのセッションが存在する
      When Codex Permission UI で Read only を選択する
      Then Codex Bridge には sandboxMode read-only と approvalPolicy never が適用される

    Scenario: Codex の Workspace では Approval を選択できる
      Given Codex バックエンドのセッションが存在する
      When Codex Permission UI で Workspace と Ask を選択する
      Then Codex Bridge には sandboxMode workspace-write と approvalPolicy on-request が適用される
      When Codex Permission UI で Workspace と Never を選択する
      Then Codex Bridge には sandboxMode workspace-write と approvalPolicy never が適用される

  Rule: SDK 側のパーミッションモード変更イベントがあるバックエンドではフロントエンドに反映される

    Scenario Outline: SDK がパーミッションモードを変更した場合にUIが追随する
      Given <backend> バックエンドのセッションが存在する
      When SDK がパーミッションモードを変更する
      Then ModeSelector の表示が新しいモードに更新される

      Examples:
        | backend |
        | Claude  |

    Scenario: プランモード終了時にパーミッションモードが復元される
      Given SDK がプランモードに遷移している
      When プランモードが終了する
      Then パーミッションモードがプランモード前の状態に復元される
      And ModeSelector の表示が復元されたモードに更新される

  # ── ツール使用表示（バックエンド共通） ──

  Rule: どのバックエンドでもツール使用がアクティビティログに表示される

    Scenario Outline: ツール実行がアクティビティログに表示される
      Given <backend> バックエンドでターンが実行中である
      When SDK がツールを使用する
      Then ツール使用と結果がアクティビティログに表示される

      Examples:
        | backend |
        | Claude  |
        | Codex   |

  # ── ターン中断 ──

  Rule: 実行中のターンを中断できる

    Scenario: ストリーミング中にターンを中断する
      Given Codex バックエンドでターンがストリーミング中である
      When 中断を実行する
      Then 現在のターンが停止する
      And セッションが待機状態に戻る

  # ── エラーハンドリング ──

  Rule: ブリッジプロセスのクラッシュ時にセッションがエラー状態に遷移する

    Scenario: ブリッジプロセスがクラッシュする
      Given Codex バックエンドのセッションが存在する
      When ブリッジプロセスがクラッシュする
      Then セッションがエラー状態に遷移する

    Scenario: ストリーミング中にブリッジプロセスがクラッシュする
      Given Codex バックエンドでターンがストリーミング中である
      And 部分レスポンスが蓄積されている
      When ブリッジプロセスがクラッシュする
      Then 蓄積済みの部分レスポンスが表示され続ける
      And セッションがエラー状態に遷移する

  Rule: エラー状態のセッションでエラーが表示される

    Scenario: ブリッジクラッシュ時にエラーメッセージが表示される
      Given セッションがエラー状態である
      When チャットパネルを表示する
      Then エラーメッセージが表示される

  Rule: エラー状態から次のメッセージ送信でブリッジが自動復旧する

    Scenario: エラー状態のセッションで新しいメッセージを送信する
      Given Codex セッションがエラー状態である
      When 新しいメッセージを送信する
      Then ブリッジプロセスが再起動される
      And 既存の sessionId でセッションが再開される
      And メッセージが正常に処理される

    Scenario: エラー復旧時にスレッド復元に失敗した場合
      Given Codex セッションがエラー状態である
      And 復元対象のスレッドが無効になっている
      When 新しいメッセージを送信する
      Then ブリッジプロセスが再起動される
      And 新しいスレッドとしてセッションが開始される
      And メッセージが正常に処理される

  Rule: SDK からのエラーイベント受信時にセッションがエラー状態に遷移する

    Scenario: SDK が API エラーを返す
      Given Codex バックエンドのセッションが存在する
      When ブリッジが SDK のエラーイベントを受信する
      Then セッションがエラー状態に遷移する
      And エラーメッセージが表示される
```

## アーキテクチャ概要

### 責務配置

- **`backends/bridge_common`（新規）**: ブリッジ共通基盤。以下を担う: プロセス起動・stdin/stdout 接続・プロセス監視・PID 管理・孤児プロセスクリーンアップ、ストリーミング蓄積（`accumulate_sdk_message()`）、ツール承認フローの共通処理（`permission_request` の受信→Tauri イベント emit→`permission_response` の stdin 書き込み）、パーミッションモード管理（`setMode` コマンド発行）、プランモードの状態遷移 / バックエンド固有の SDK プロトコルやイベント解釈を持たない
- **`backends/codex.rs`（新規）**: Codex バックエンドの AgentBackend trait 実装。Codex 固有の設定管理（model, approvalPolicy, 外部 `codex` CLI パス）を担う / ブリッジプロセスの低レベル管理・ツール承認・ストリーミング蓄積は `bridge_common` に委譲する
- **`backends/claude.rs`（変更）**: Claude バックエンド。現在密結合しているプロセス管理・状態遷移・ストリーミング蓄積・ツール承認フロー・パーミッションモード管理の共通部分を `bridge_common` に抽出する / 抽出後は Claude SDK 固有の処理（`query()` API の呼び出しオプション等）のみ保持する
- **`backends/mod.rs`（変更）**: AgentBackend trait 定義、AgentBackendRegistry、`build_registry()` での Codex バックエンド登録追加 / バックエンド固有の実装詳細を持たない
- **`resources/codex-sdk-bridge.mjs`（新規）**: Node.js ブリッジ。Codex SDK（`@openai/codex-sdk`）との通信を担い、stdin/stdout JSONL プロトコルで Rust と通信する。SDK 初期化時は Claude と同じ外部 CLI 前提で `codexPathOverride` に `codex`（または設定された CLI パス）を渡す。Codex SDK のストリーミングイベント・ツール承認要求・プランモードイベントを共通ブリッジプロトコルに変換して出力する / ブリッジはステートレスな変換層であり、セッション状態管理はRust側に委ねる
- **`config.rs`（変更）**: `AgentsSection` を拡張してバックエンド固有設定（Codex のデフォルトモデル等）を格納する / バリデーションロジックは持たない（レジストリ構築時に処理）
- **フロントエンド（`AgentChatPanel/`, `hooks/`）**: チャット入力 UI 内 Agent selector・ModelSelector・PermissionDialog・ActivityLog の表示。Claude では既存 `ModeSelector` を表示し、Codex では専用 `CodexPermissionControl` を表示する。バックエンド一覧は `list_agent_backends` コマンドから取得し、BackendRegistry の登録内容をそのまま反映する

### データ/通信フロー

- **バックエンド一覧取得**: UI → `list_agent_backends` invoke → `AgentBackendRegistry.list()` → `BackendInfo[]` 返却 → チャット入力 UI 内 Agent selector に表示
- **未送信セッションの Agent 選択値更新**: UI → `set_session_backend` invoke → Rust がセッションの `messages` が空かつ `agent_session_id` がないことを検証 → `backend_id` を保存し `selected_model` をクリア → 同じ session の stale `AgentProcess` が存在する場合は破棄 → `get_session` 相当レスポンスで変更後 backend の `availableModels` を返す
- **未送信セッションの起動抑止**: `createNewSession` / `init_agent_sessions` / restore は、`messages` が空かつ `agent_session_id` がない session では Bridge/backend process を起動しない。初回送信時だけ保存済み `backend_id` を読み、対応する backend の Bridge を spawn する
- **セッション開始**: UI → `start_agent_session` invoke → `bridge_common::spawn_bridge_process()` → Node.js ブリッジ起動 → stdin に `init` コマンド（Codex では CLI パスも渡す）→ Bridge が `new Codex({ codexPathOverride: cliPath })` で SDK 初期化 → stdout に `session_ready` → Rust が `AgentProcess` の `BridgeState` を `Ready` に遷移 → `agent-session-state-changed` Tauri イベント → UI 更新
- **メッセージ送信**: UI → `send_agent_message` invoke → Rust が stdin に `message` コマンド → Bridge が SDK の `runStreamed()` / `promptGenerator` に転送 → stdout にストリーミングイベント（`stream_event`, `assistant`, `turn_complete`） → Rust が `accumulate_sdk_message()` で蓄積 → `agent-streaming-updated` Tauri イベント → UI がリアルタイム更新
- **ツール承認**: 承認イベントを公開する Bridge では、SDK がツール実行 → Bridge の承認コールバック → パーミッションモードに応じて自動許可 or `permission_request` を stdout に出力 → Rust が `agent-sdk-message` Tauri イベントで転送 → フロントエンドの PermissionDialog が表示 → ユーザーが許可/拒否 → `respond_agent_permission` invoke → Rust が stdin に `permission_response` → Bridge が pending Promise を resolve → SDK がツールを実行 or スキップ。Codex SDK 0.128.0 は公開 `ThreadEvent` に承認要求を含まないため、Codex Bridge は `approvalPolicy` / `sandboxMode` の指定に留める
- **プランモード**: 承認イベントを公開する Bridge では、SDK が `EnterPlanMode` / `ExitPlanMode` ツールを使用 → 承認コールバック経由で `permission_request`（`tool_name: "EnterPlanMode"` / `"ExitPlanMode"`）が送信される → フロントエンドの PermissionDialog がプラン承認UIを表示 → ユーザーが承認/拒否 → 通常のツール承認フローと同じ経路で応答。Codex は SDK が該当イベントを公開するまで対象外
- **パーミッションモード変更（UI起点）**: UI → `set_agent_permission_mode` invoke → Rust が stdin に `setMode` コマンド → Bridge が SDK の `setPermissionMode()` を呼び出し → 以降のツール承認にモードが適用される
- **Codex パーミッション変換**: UI では `Read only` → 互換 `permissionMode: "plan"`、`Workspace + Ask` → `"default"`、`Workspace + Never` → `"acceptEdits"`、`Full access` → `"bypassPermissions"` として保存/送信する。Codex Bridge では `"plan"` を `sandboxMode: "read-only"` + `approvalPolicy: "never"`、`"default"` を `workspace-write` + `on-request`、`"acceptEdits"` を `workspace-write` + `never`、`"bypassPermissions"` を `danger-full-access` + `never` に変換する。Codex Bridge では deprecated な `on-failure` は使わない
- **パーミッションモード変更（SDK起点）**: SDK がモード変更（例: プランモード開始で `"plan"` に遷移）→ Bridge が `system` メッセージ（`permissionMode` フィールド）を stdout に出力 → Rust が `AgentProcess.current_permission_mode` を更新 → `agent-permission-mode-changed` Tauri イベント → フロントエンドの ModeSelector が追随。プランモード終了時（`permissionMode: "default"`）は永続化済みのモードに復元し、Bridge にも `setMode` で反映する
- **モデル変更**: UI → `set_agent_model` invoke → Rust が stdin に `setModel` コマンド（process 未起動時は SessionStore 保存のみ）→ Bridge が SDK の `setModel()` を呼び出し → 以降のターンに反映
- **中断**: UI → `interrupt_agent_query` invoke → Rust が stdin に `interrupt` コマンド → Bridge が `AbortController.abort()` → SDK がターンを停止 → stdout に `turn_complete` → 通常のターン完了フロー
- **モデル一覧取得/受信**: `get_session.availableModels` が UI の唯一の source。process 未起動時は Rust が `AgentBackendRegistry.available_models()` または backend 別 cache から返し、Bridge 起動後に SDK が `supported_models` を stdout に出力した場合は `AgentProcess.available_models` と backend 別 cache を更新する
- **エラー復旧（ブリッジクラッシュ）**: ブリッジプロセスが予期せず終了 → Rust が stdout EOF を検出 → `BridgeState::Crashed` に遷移 → `agent-session-state-changed` イベント → 次の `send_agent_message` 呼び出し時に `spawn_bridge_process()` でブリッジを再起動
- **エラー復旧（SDK エラー）**: SDK が API エラー（認証失敗・レートリミット等）を返す → Bridge が `error` イベントを stdout に出力後 `exit(1)` で終了 → Rust が stdout EOF を検出 → `BridgeState::Crashed` に遷移（ブリッジクラッシュと同じ復旧パス）

### 状態Owner

- **`AgentProcessMap`（`HashMap<String, AgentProcess>`）**: Rust（Tauri state）。セッション ID → ブリッジプロセスのマッピング。プロセスの stdin ハンドル、`BridgeState`、`TurnPhase`、ストリーミング中のパーツ蓄積、利用可能モデル、選択中モデル、パーミッションモードを保持する
- **`BridgeState`（`Initializing` / `Ready` / `Streaming` / `Crashed`）**: Rust（`AgentProcess` 内）。ブリッジプロセスのライフサイクル状態
- **`TurnPhase`（`Idle` / `Streaming` / `WaitingPermission`）**: Rust（`AgentProcess` 内）。SDK のターンライフサイクル。`WaitingPermission` はツール承認待ち（プランモード承認含む）を示す。フロントエンドに Tauri イベントで通知される
- **`AgentBackendRegistry`**: Rust（Tauri state、`Arc<AgentBackendRegistry>`）。登録済みバックエンドの一覧と可用性。アプリ起動時に構築、以後 immutable
- **セッションの `backend_id`**: Rust（`SessionStore` → JSON ファイル）。session の Agent 選択値であり、process 起動済みを意味しない。NewSession 作成直後から初回メッセージ送信前までは変更可能で、初回メッセージ送信後に固定される。未送信 session の Agent 変更時に既存 `AgentProcess` が残っている場合は stale とみなし、破棄して次回送信時に保存済み `backend_id` から起動し直す
- **`availableModels` / `selectedModel`**: フロントエンド（`agentChatReducer` の state）。Rust から `agent-models-updated` イベントで受信し、`get_session` レスポンスからも復元する。表示専用
- **`backends` / `selectedBackendId`**: フロントエンド（`agentChatReducer` の state）。`list_agent_backends` の結果と active session の Agent 選択値を表示するために保持する。active session がない場合のみ、新規セッション作成時の初期選択に使用する
- **`permissionMode`**: Rust（`AgentProcess.current_permission_mode`）が権威。フロントエンドは Claude では `ModeSelector`、Codex では `CodexPermissionControl` を表示・変更UIとして使う。変更時は `set_agent_permission_mode` invoke → Rust → Bridge へ反映する。保存値は互換維持のため既存 `permissionMode` を使う

### 境界

- **Rust ↔ Bridge（プロセス境界）**: stdin/stdout JSONL の共通ブリッジプロトコル。各ブリッジ（Claude/Codex）はそれぞれの SDK 固有イベントを共通プロトコルに変換して出力する。共通プロトコルは以下を含む: テキスト/thinking ストリーミング（`stream_event`）、ツール使用/結果（`assistant`/`user` メッセージ内の `tool_use`/`tool_result`）、ツール承認要求/応答（`permission_request`/`permission_response`）、プランモード（`EnterPlanMode`/`ExitPlanMode` ツールとしての `permission_request`）、パーミッションモード設定（`setMode`）、ターン完了（`turn_complete`）
- **Rust ↔ フロントエンド（Tauri 境界）**: Tauri コマンド（invoke）でリクエスト、Tauri イベント（emit）でプッシュ通知。フロントエンドはバックエンド固有の知識を持たず、`BackendInfo.id` / `BackendInfo.name` でのみバックエンドを識別する。PermissionDialog・ModeSelector・ActivityLog はバックエンド非依存に動作する
- **共通基盤 ↔ バックエンド固有コード（モジュール境界）**: `bridge_common` はプロセスライフサイクル管理・JSONL パース・ストリーミング蓄積・ツール承認フロー・パーミッションモード管理の共通処理を提供する。`claude.rs` / `codex.rs` はバックエンド固有の設定（ブリッジスクリプトパス、起動引数、可用性検出）と AgentBackend trait 実装のみを持つ
- **AgentBackendRegistry ↔ Tauri コマンド（dispatch 境界）**: 現在の Tauri コマンド（`set_agent_model`, `set_agent_permission_mode`, `respond_agent_permission` 等）は `AgentProcessMap` に直接アクセスしている。Codex 追加にあたり、セッションの `backend_id` に基づいて正しいバックエンドのプロセスにディスパッチする共通化が必要

### 実装に委ねること

- `bridge_common` の内部関数分割（helper 関数名、private struct の設計）
- `accumulate_sdk_message()` を共通化する際の関数シグネチャ
- `codex-sdk-bridge.mjs` 内のイベント変換ロジックの詳細実装（Codex SDK の `canUseTool` 相当の承認コールバック実装含む）
- Codex SDK のプランモード API と Claude の `EnterPlanMode`/`ExitPlanMode` 間のマッピング詳細
- `AgentsSection` のバックエンド固有設定のフィールド名
- フロントエンドでの状態 dispatch アクション名やペイロード形状の詳細
- テストのファイル配置・テストケースの具体的構成
- 共通化の際にどこまで claude.rs から抽出するかの粒度（段階的抽出も可）
