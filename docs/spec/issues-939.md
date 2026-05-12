## 要求

**種別**: 新機能
**ゴール**: ワークフローYAMLの各ステップにModel/Permissionフィールドを追加し、ステップごとに異なるモデル・権限モードで実行できるようにする。Model指定により対応するバックエンドが自動的に選択される
**背景**: 現在のワークフローは全ステップがワークフロー起動時のセッション設定（Backend/Model/PermissionMode）で統一的に動作する。しかし、タスク特性に応じた使い分け（例: 計画ステップでは高性能モデル、実装ステップでは高速モデル、レビューステップではファイル編集権限を制限）が求められる

### 成功基準

- ワークフローYAMLの各ステップに `model`（モデルID）、`permission`（PermissionMode）フィールドをオプションで指定できる
- model値から `available_models()` を使って対応するバックエンドが自動的に解決される
- ワークフローエンジンがステップ実行時に、解決されたバックエンド・モデルでエージェントセッションを起動し、指定のPermissionModeで動作する
- 未指定時は親セッションの設定（ワークフロー起動時のBackend/Model/PermissionMode）をそのまま継承する

### 設計メモ

- Model指定で実行バックエンドを自動解決する。各バックエンドの `available_models()` からmodel値を逆引きし、対応するバックエンドを特定する
- 各ステップに指定可能なフィールドは `model`（モデルID）と `permission`（PermissionMode）の2つ。いずれもオプション
- PermissionModeは既存の3モード（`acceptEdits` / `bypassPermissions` / `plan`）から選択
- 未指定時は親セッションの設定を継承する

## 振る舞い定義

```gherkin
Feature: ステップ別Model/Permission指定
  ワークフローYAMLの各ステップにModel・Permissionをオプション指定し、
  ステップごとに異なるモデル・権限モードで実行できる。
  Model値から対応するバックエンドが自動的に解決される。

  Background:
    Given ワークフローセッションが "claude" バックエンド、"opus-4" モデル、"acceptEdits" 権限モードで起動されている

  Rule: ステップに指定された設定でセッションを起動する

    Scenario: Model・Permissionが指定されたステップを実行する
      Given ステップに model: "codex-mini"、permission: "bypassPermissions" が指定されている
      When そのステップが実行される
      Then "codex-mini" から対応するバックエンド "codex" が自動解決される
      And ステップセッションは "codex" バックエンド、"codex-mini" モデルで起動される
      And ステップセッションは "bypassPermissions" 権限モードで動作する

    Scenario: Modelのみ指定されたステップを実行する
      Given ステップに model: "haiku" のみ指定されている
      When そのステップが実行される
      Then "haiku" から対応するバックエンド "claude" が自動解決される
      And ステップセッションは "claude" バックエンド、"haiku" モデルで起動される
      And ステップセッションは親セッションの "acceptEdits" 権限モードを継承する

    Scenario: Permissionのみ指定されたステップを実行する
      Given ステップに permission: "plan" のみ指定されている
      When そのステップが実行される
      Then ステップセッションは親セッションの "claude" バックエンドを継承する
      And ステップセッションは親セッションの "opus-4" モデルを継承する
      And ステップセッションは "plan" 権限モードで動作する

  Rule: 未指定フィールドは親セッションの設定を継承する

    Scenario: 全フィールドが未指定のステップを実行する
      Given ステップに model・permission がいずれも指定されていない
      When そのステップが実行される
      Then ステップセッションは親セッションの "claude" バックエンドを継承する
      And ステップセッションは親セッションの "opus-4" モデルを継承する
      And ステップセッションは親セッションの "acceptEdits" 権限モードを継承する

  Rule: 並列ステップでもModel/Permissionを個別に指定できる

    Scenario: 並列ブロック内の各ステップに異なる設定を指定する
      Given 並列ブロック内にステップAとステップBがある
      And ステップAに model: "opus-4"、permission: "plan" が指定されている
      And ステップBに model: "codex-mini"、permission: "bypassPermissions" が指定されている
      When 並列ブロックが実行される
      Then ステップAは "opus-4" から解決されたバックエンド "claude" で、"plan" 権限モードで起動される
      And ステップBは "codex-mini" から解決されたバックエンド "codex" で、"bypassPermissions" 権限モードで起動される

  Rule: 無効な設定値はYAMLロード時にバリデーションエラーとする

    Scenario: どのバックエンドにも存在しないModelが指定されたワークフローをロードする
      Given ステップに model: "unknown-model" が指定されている
      When ワークフローYAMLをロードする
      Then バリデーションエラー "unknown model: unknown-model" が返される
      And ワークフローは実行されない

    Scenario: 無効なPermissionが指定されたワークフローをロードする
      Given ステップに permission: "invalid-mode" が指定されている
      When ワークフローYAMLをロードする
      Then バリデーションエラー "invalid permission mode: invalid-mode" が返される
      And ワークフローは実行されない
```

## アーキテクチャ概要

### 責務配置

- **workflow::schema（`schema.rs`）**: ステップ定義の型を担当する。`Step`/`ParallelStep`にオプショナルな`model`/`permission`フィールドを持つ / ステップ設定の解決（継承ロジック）は担当しない
- **workflow::validation（`validation.rs`）**: ワークフローYAMLのバリデーションを担当する。`model`値が全バックエンドの`available_models()`のいずれかに存在するか、`permission`値が有効なPermissionMode（`acceptEdits`/`bypassPermissions`/`plan`）かを検証する / ステップ設定の解決（継承ロジック）は担当しない
- **workflow::engine（`engine.rs`）**: ステップ実行時の設定解決を担当する。ステップの`model`/`permission`と親セッションの設定をマージし、`model`値から`available_models()`を走査して対応するバックエンドを特定し、解決済みの`backend_id`/`model`/`permission_mode`でセッション起動・ターン送信を行う / ステップ定義の型や構文解析は担当しない
- **session（`session/mod.rs`）**: セッション永続化を担当する。`ChatSession`に`backend_id`/`selected_model`/`permission_mode`を保持し、セッション生成時にこれらを設定する / 設定のマージや継承ロジックは担当しない
- **backends::bridge_common（`bridge_common.rs`）**: ブリッジプロセス起動を担当する。`ChatSession`の`backend_id`/`selected_model`を読み取り、対応するブリッジプロセスを起動する。ターン開始時に`permission_mode`をSDKに送信する / 設定の解決やフォールバックロジックは担当しない
- **backends（`mod.rs`、`AgentBackendRegistry`）**: バックエンドの登録と`available_models()`の提供を担当する。validation層・engine層からの問い合わせに対し、モデル一覧とバックエンド解決を提供する / ステップ設定のマージは担当しない

### データ/通信フロー

- **逐次ステップ実行**: engine `start_step_session` → ステップ定義から`model`/`permission`を読み取り → 未指定フィールドは親セッション（`ChatSession`）の値で補完 → `model`値から`available_models()`を走査して対応する`backend_id`を特定 → `create_session_internal`で解決済みの`backend_id`/`selected_model`/`permission_mode`を持つステップセッションを生成 → `start_agent_session_internal`で対応バックエンドのブリッジプロセスを起動 → `start_agent_turn_internal`で解決済み`permission_mode`とプロンプトを送信
- **並列ステップ実行**: engine `start_parallel_children` → 各子ステップの`model`/`permission`を個別に読み取り → 逐次ステップと同じ解決ロジックで各子セッションを生成・起動

### 状態Owner

- **ステップ定義の`model`/`permission`値（YAML由来）**: `workflow::schema::Step`/`ParallelStep` — YAMLデシリアライズ時に設定、ワークフロー実行中は不変
- **親セッションの設定（`backend_id`/`selected_model`/`permission_mode`）**: `session::ChatSession`（ワークフロー起動時のセッション） — ワークフロー実行中は不変、フォールバック元として参照される
- **解決済みステップ設定**: Owner無し（一時的な値） — `start_step_session`/`start_parallel_children`内で都度計算し、ステップセッション生成パラメータとして消費される
- **ステップセッションの設定**: `session::ChatSession`（ステップ用セッション） — 生成時に解決済みの値が設定され、セッション存続中は不変

### 境界

- schema層は「何が指定されたか」を保持するのみで、「未指定時にどうするか」は知らない。継承ロジックはengine層が担当する
- validation層（`validation.rs`）は`model`値が全バックエンドの`available_models()`に存在するか、`permission`値が有効かを検証する。モデル一覧へのアクセスが必要となる
- engine層はステップ実行時に`model`値から対応するバックエンドを解決し、設定の解決結果をsession生成パラメータとして渡すが、ブリッジプロセスの起動方法やSDK通信の詳細には関与しない
- session層は渡された値をそのまま保持・永続化する。値の妥当性はvalidation層で検証済みであることを前提とする
- bridge_common層はChatSessionから読み取った値でブリッジを起動する。値がどこから来たか（ステップ指定か親セッション継承か）は知らない

### 実装に委ねること

- 設定解決ロジックのヘルパー関数名・配置（engine.rs内のprivateメソッド or 別関数）
- `create_session_internal`のシグネチャ拡張方法（引数追加 or 構造体パラメータ化）
- validation.rsでの`AgentBackendRegistry`（モデル一覧）へのアクセス方法（引数追加 or 静的参照等）
- テストの具体的な配置とヘルパー関数
- ParallelStepの新フィールドの属性マクロ（`#[serde(default)]`等）の具体的記法
