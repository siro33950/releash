## 要求

**種別**: 改善

**ゴール**:
- `ModelInfo.display_name` フィールドを廃止し、`value` に統一する（フロントエンドは `value` をそのまま表示する）
- 各バックエンド（claude / codex）の対応モデル一覧を `config.toml` で一元管理する（既存の `available_models.json` キャッシュは廃止する）
- アプリ起動時にバックグラウンドで各バックエンドCLI（`codex debug models` 等）からモデル一覧を取得し、`config.toml` を自動更新する
- ユーザー／ワークフローから指定されたモデルIDは `config.toml` に登録されているもののみ受け入れる（サイレントフォールバックしない）

**背景**:
- 現在モデル一覧は `claude_supported_models()` / `codex_supported_models()` にハードコードされており、CLI側のモデル追加・廃止への追従が手動コード変更を要する
- 並行して、起動済みプロセスから流れる `supported_models` イベントの結果が `bridge_common.rs` の `save_available_models()` で `available_models.json` に保存されており、永続化先が二重化している
- `ModelInfo.display_name` は現状 `value` と同値で、独立した表示名管理の必要性がない
- #939（ステップ別Model/Permission指定）でモデル一覧がバリデーションに使われるため、最新性が確保されないと有効なモデルが弾かれる

**制約**:
- モデル一覧の保存先は `config.toml` のみ（`available_models.json` / `available_models_{backend_id}.json` は廃止）
- 起動時のCLI取得はバックエンド単位で独立して非同期実行し、アプリ起動をブロックしない
- いずれかのバックエンドのCLI取得に失敗しても、他バックエンドの取得・更新は継続する（部分失敗を許容する）
- CLI取得に失敗した場合は当該バックエンドの `config.toml` の既存値をそのまま使い続ける（ログは出す）
- 初期状態（`config.toml` に未保存）では空の一覧で開始し、CLI取得完了後に埋める
- コード内のハードコードフォールバック値は持たない（テスト用モックを除く）
- CLI取得結果およびユーザー指定モデルIDは入力検証を行い、検証に失敗した値は `config.toml` を上書きしない／指定として受け入れない

**影響範囲**:
- `src-tauri/src/backends/mod.rs`（`ModelInfo` 構造体、`AgentBackendRegistry`、`collect_all_model_values` / `resolve_backend_for_model`）
- `src-tauri/src/backends/claude.rs` / `codex.rs`（ハードコードされた `supported_models` 関数廃止、CLI問い合わせロジック追加）
- `src-tauri/src/backends/bridge_common.rs`（`available_models_path` / `save_available_models` / `load_available_models` 廃止、`supported_models` メッセージ処理時の永続化を config.toml 経由へ統合、`set_agent_model` / `set_agent_model_internal` の入力検証を config 由来へ切替）
- `src-tauri/src/config.rs`（`AppConfig` にバックエンド別モデル一覧セクション追加）
- `src-tauri/src/protocol/agent.rs`（`AgentModelSetRequest` の検証経路）
- `src-tauri/src/ws_server/routing.rs` / `ws_server/handlers.rs`（`handle_agent_model_set_request` の検証経路）
- `src-tauri/src/lib.rs`（起動時バックグラウンド同期のトリガー）
- フロントエンド: `ModelSelector.tsx`、`MessageInput.tsx`、`session.ts`、`useSessionStore.ts`、`useAgentSdkListeners.ts`、`useAgentChat.ts`、`agentChatReducer.ts` 等の `ModelInfo.displayName` 参照箇所
  - 対象は `ModelInfo.displayName` のみ。`PermissionRequest.display_name`（権限ダイアログ表示名）は対象外
- Remote UI 経由のモデル指定（`src/remote/` 配下のモデル選択導線 → `AgentModelSetRequest` 送信経路）
- 関連テスト全般

## 振る舞い定義

```gherkin
Feature: バックエンド対応モデル一覧の動的管理
  ユーザーが指定可能なモデルは、各バックエンドの権威ある情報源から得られた
  最新の一覧を信頼できる単一の場所で管理し、ユーザーが常に有効なモデルだけを
  選択・指定できるようにする。

  Rule: モデル一覧はバックエンドごとに独立して最新化される
    各バックエンドのモデル一覧は、そのバックエンド固有の情報源から取得され、
    他バックエンドの取得結果や障害に影響されない。

    Scenario: 起動時に各バックエンドのモデル一覧が最新化される
      Given アプリには複数のバックエンドが登録されている
      When アプリが起動する
      Then 各バックエンドのモデル一覧が、それぞれの情報源から得られた最新の一覧で記録される

    Scenario: アプリ起動はモデル一覧取得の完了を待たない
      Given アプリ起動時に各バックエンドのモデル一覧の最新化が開始される
      And いずれかのバックエンドのモデル一覧取得に時間を要している
      When アプリの起動シーケンスが進行する
      Then アプリの起動は当該バックエンドのモデル一覧取得の完了を待たずに完了する
      And 当該バックエンドのモデル一覧は取得完了後に当該バックエンドの一覧へ反映される

    Scenario: あるバックエンドの取得失敗は他バックエンドの最新化を妨げない
      Given 一部のバックエンドではモデル一覧の取得に失敗する
      And 他のバックエンドではモデル一覧の取得に成功する
      When アプリが起動する
      Then 成功したバックエンドのモデル一覧は最新化される
      And 失敗したバックエンドのモデル一覧は直前の登録内容のまま維持される
      And 失敗したことが運用ログから追跡できる

    Scenario: 取得結果が信頼できない場合は登録内容を変更しない
      Given あるバックエンドの取得結果が要件を満たさない不正な内容である
      When アプリがその結果を反映しようとする
      Then 当該バックエンドのモデル一覧は直前の登録内容のまま維持される
      And 不正な内容を受け入れなかったことが運用ログから追跡できる

    Scenario: 取得結果に同一識別子の重複が含まれていても登録上は一意に保たれる
      Given あるバックエンドの取得結果に同一のモデル識別子が複数含まれる
      When アプリがその結果を反映する
      Then 当該バックエンドのモデル一覧には同一識別子が重複して現れない

    Scenario: 既に動いているバックエンドからの一覧通知も同じ規準で取り込まれる
      Given アプリが既に起動しており、あるバックエンドが稼働している
      When 当該バックエンドから新しいモデル一覧の通知が届く
      Then 当該バックエンドのモデル一覧は、起動時と同じ受け入れ規準を経て最新化される
      And ユーザーが見ているモデル選択候補も、その最新化された内容に追従する

    Scenario: 同一バックエンドへの更新が競合しても他バックエンドの一覧を失わない
      Given 同一バックエンドに対する複数の更新がほぼ同時に発生する
      When それぞれの更新が処理される
      Then 当該バックエンドのモデル一覧には、最後に受け入れられた有効な内容が反映される
      And 他バックエンドのモデル一覧はその影響を受けない

    Scenario: 既定モデルが現行の一覧に含まれない場合は未指定として扱う
      Given あるバックエンドに既定モデルが設定されている
      And その既定モデルが当該バックエンドの現行のモデル一覧に含まれない
      When 当該バックエンドの新しいセッションが開始される
      Then そのセッションの初期モデルは未指定として扱われる
      And 既定モデルがサイレントに別のモデルへ書き換えられることはない
      And 当該既定モデルが一覧外であることが運用ログから追跡できる

    Scenario: 既定モデルが現行の一覧に含まれる場合は初期モデルとして使われる
      Given あるバックエンドに既定モデルが設定されている
      And その既定モデルが当該バックエンドの現行のモデル一覧に含まれる
      When 当該バックエンドの新しいセッションが開始される
      Then そのセッションの初期モデルは当該既定モデルとなる

  Rule: モデル選択候補は登録済み一覧そのものを提示する
    ユーザーがモデルを選ぶ場面では、登録済みのモデル一覧がそのまま候補として
    提示される。コードに埋め込まれた候補や暗黙のフォールバック候補は提示しない。

    Scenario: 登録済みのモデルが選択候補として提示される
      Given あるバックエンドのモデル一覧が登録されている
      When ユーザーが当該バックエンドのモデル選択を開く
      Then 候補として、登録されているモデル識別子がそのまま提示される

    Scenario: モデル識別子は取得元の文字列がそのまま表示される
      Given あるバックエンドに、取得元から得られた識別子がそのまま登録されている
      When ユーザーがモデル選択候補を見る
      Then 候補欄には当該識別子が、加工や整形なしの文字列としてそのまま表示される

    Scenario: モデル識別子は安全な文字列として表示される
      Given 登録されているモデル識別子に、表示時に副作用を起こしうる文字が含まれる
      When ユーザーがモデル選択候補を見る
      Then 当該識別子は文字列としてのみ表示される
      And 当該識別子に含まれる内容が画面上で実行・解釈されることはない
      And メイン画面・リモート画面のいずれでも同様に振る舞う

    Scenario: モデル一覧が空でも選択UI自体は提示される
      Given あるバックエンドのモデル一覧が空である
      When ユーザーが当該バックエンドのモデル選択を開く
      Then モデル選択UI自体は表示される
      And 候補は0件として提示される
      And 暗黙のフォールバック候補や代替文言は含まれない

  Rule: モデル指定は登録済み一覧に照らして検証される
    ユーザーやワークフローが指定したモデルは、登録済みのモデル一覧と
    照らし合わせて検証される。検証を通らなかった指定は、サイレントに
    別のモデルへ置き換えられることはなく、明示的に拒否される。

    Scenario: 当該セッションのバックエンドに登録済みのモデル指定は受け入れる
      Given セッションのバックエンドが特定されている
      And 指定されたモデルが、当該バックエンドのモデル一覧に登録されている
      When ユーザーまたはワークフローがそのモデルを指定する
      Then 指定は受け入れられ、当該セッションの選択モデルとなる

    Scenario: 当該セッションのバックエンドに未登録のモデル指定は明示的に拒否する
      Given セッションのバックエンドが特定されている
      And 指定されたモデルが、当該バックエンドのモデル一覧に登録されていない
      When ユーザーまたはワークフローがそのモデルを指定する
      Then 指定は明示的なエラーで拒否される
      And 当該セッションの選択モデルは変更されない
      And サイレントに別のモデルへフォールバックすることはない

    Scenario: 他バックエンドのモデルを当該セッションに指定するとバックエンド不一致として拒否する
      Given セッションのバックエンドが特定されている
      And 指定されたモデルは別バックエンドのモデル一覧にのみ登録されている
      When ユーザーまたはワークフローがそのモデルを指定する
      Then 指定はバックエンド不一致を示すエラーで拒否される
      And 当該セッションの選択モデルは変更されない

    Scenario: モデル指定がバックエンドに固定されないワークフロー経路では識別子から所属バックエンドを特定して受け入れる
      Given ワークフローのステップではセッションのバックエンドが事前に固定されていない
      And 指定されたモデルが、ただ一つのバックエンドのモデル一覧にのみ登録されている
      When ワークフローが当該ステップを実行する
      Then 指定モデルから所属するバックエンドが一意に特定される
      And 当該バックエンドのセッションが、その指定モデルで開始される

    Scenario: ワークフロー経路で指定モデルが複数バックエンドに登録されている場合は明示的に拒否する
      Given ワークフローのステップではセッションのバックエンドが事前に固定されていない
      And 指定されたモデルが、複数のバックエンドのモデル一覧に同一識別子で登録されている
      When ワークフローが当該ステップを実行する
      Then 所属バックエンドを一意に特定できないことを示す明示的なエラーで拒否され、セッションは開始されない
      And いずれかのバックエンドへサイレントに割り当てられることはない

    Scenario: いずれのバックエンドにも登録されていないモデル指定は、ワークフロー経路でも明示的に拒否する
      Given ワークフローのステップではセッションのバックエンドが事前に固定されていない
      And 指定されたモデルは、いずれのバックエンドのモデル一覧にも登録されていない
      When ワークフローが当該ステップを実行する
      Then 指定は明示的なエラーで拒否され、セッションは開始されない
      And サイレントに別のモデルへフォールバックすることはない

    Scenario: モデル未指定（選択解除）は受け入れ、暗黙のフォールバックは行わない
      Given 当該セッションに選択モデルが設定されている
      When ユーザーまたはワークフローがモデル未指定を指示する
      Then 指示は受け入れられ、当該セッションの選択モデルは「未指定」状態に戻る
      And 以降のセッション実行で、暗黙のうちに既定モデルへフォールバックすることはない

    Scenario: 形式として無効なモデル指定は、登録判定に進まず拒否する
      Given ユーザーまたはワークフローが、識別子として成立しない値をモデルに指定する
      When その指定がモデル設定経路に到達する
      Then 指定は登録一覧との照合に進む前に、形式不正として拒否される
      And 指定された値は変更や正規化を受けない

    Scenario: 検証は指定経路によらず同一の基準で行われる
      Given ユーザーまたはワークフローが何らかの経路からモデルを指定する
      When 同一内容の指定が異なる指定経路から行われる
      Then モデル識別子の形式検証は、指定経路によらず同一のルールで行われる
      And 登録済み一覧との照合判定も、指定経路によらず同一のルールで行われる
      And 照合対象となる登録済み一覧は、当該経路が前提とする「セッションのバックエンドが固定されているか否か」のみに依存する（既存セッション経路は当該セッションの所属バックエンドの一覧、ワークフロー経路は全バックエンドの一覧から所属バックエンドを一意に特定する）
      And 経路間で受け入れ／拒否の判定基準が分岐することはない
```

## アーキテクチャ概要

### 責務配置

- **`src-tauri/src/config.rs`（`AgentsSection` / `ClaudeAgentSection` / `CodexAgentSection`）**
  - 担当する: 各バックエンドのモデル一覧（モデルID文字列のリスト）の永続化スキーマ定義、デフォルト値（空リスト）の提供、`config.toml` への書き出し、既存の `agents.<backend>.model`（起動時の初期モデル）の永続化スキーマ維持
  - 担当しない: モデル一覧の取得・検証・更新タイミングの決定、`agents.<backend>.model` を `agents.<backend>.models` に対して整合チェックする処理（整合チェックは起動シーケンス側で実施）

- **`src-tauri/src/backends/{claude,codex}.rs`（各 `AgentBackend` 実装）**
  - 担当する: 自バックエンドCLIへ問い合わせて生のモデル識別子リストを取得する処理（CLIコマンド呼び出し・出力パース・タイムアウト・エラーハンドリング）
  - 担当しない: 取得結果の永続化、ハードコードされたモデル一覧の保持、`available_models()` の表示用整形

- **`src-tauri/src/backends/mod.rs`（`AgentBackendRegistry` / `ModelInfo`）**
  - 担当する: 「設定ファイル上のモデル一覧」と「バックエンドCLI取得処理」の仲介、`available_models()` を config 由来で返す、起動時バックグラウンド同期処理のエントリポイント提供、モデル指定の検証（`resolve_backend_for_model` / `collect_all_model_values` を config.toml 由来へ切替）、モデルID入力検証ルールの提供、ワークフロー step 実行経路における所属バックエンド特定の一意性保証（同一モデルIDが複数バックエンドに登録されている場合は一意特定不能として明示エラーを返す／`model_id=None` は選択解除として扱い暗黙の既定モデルへフォールバックさせない）
  - 担当しない: モデル取得CLIの具体的な呼び出し、`config.toml` のスキーマ詳細

- **`src-tauri/src/backends/bridge_common.rs`**
  - 担当する: `supported_models` メッセージ受信時の検証＆ config.toml への保存呼び出し、`agent-models-updated` イベントは config.toml 由来の最新値で配信、`set_agent_model` / `set_agent_model_internal` における config 由来の登録済みモデル検証（当該セッションの `backend_id` に登録されたモデル一覧のみと突合し、別バックエンドのモデルは backend mismatch として拒否）、`model_id` が `None`（null）の場合の選択解除処理（暗黙の既定モデルへフォールバックさせない）
  - 担当しない: `available_models.json` 系ファイルへの読み書き（経路を廃止）

- **`src-tauri/src/ws_server/handlers.rs`（`handle_agent_model_set_request`）**
  - 担当する: `AgentModelSetRequest` を transport 層として受け付け、共通のモデル設定処理（`bridge_common.rs` の `set_agent_model_internal`）へ委譲する。委譲先から返ったエラー（入力検証エラー／backend mismatch／登録外モデル）を WebSocket クライアントへエラー応答として返却し、成功時は成功応答を返す
  - 担当しない: 入力検証ロジックそのもの、config 由来の登録済みモデル検証ロジックそのもの（これらは単一経路の `set_agent_model_internal` に集約する）、モデル一覧の保持

- **`src-tauri/src/lib.rs`（setup フック）**
  - 担当する: アプリ起動時に Registry の同期処理を `tokio::spawn` でバックエンド単位に独立して起動するトリガー、起動時に `agents.<backend>.model` が `agents.<backend>.models` に含まれるかを検査し、未登録なら警告ログを出して当該バックエンドの初期モデルを「未指定」として扱う
  - 担当しない: 同期処理本体のロジック、`agents.<backend>.model` の書き換え（サイレントなフォールバック書き換えはしない）

- **フロントエンド（`ModelSelector.tsx` / `MessageInput.tsx` / `session.ts` 等、および `src/remote/` 配下のモデル指定導線）**
  - 担当する: Tauri コマンドから返ったモデル一覧の表示、選択状態の保持、`value` をそのまま画面に表示、Remote UI から `AgentModelSetRequest` を送信
  - 担当しない: モデル一覧のキャッシュ・補完・フォールバック生成、`ModelInfo.displayName` 由来の派生表示

### データ/通信フロー

- **起動時のモデル一覧同期**:
  `lib.rs setup` → バックエンドごとに `tokio::spawn(Registry::refresh_models_to_config_for(backend_id, AppConfig))` →
  各 `AgentBackend::fetch_models_from_cli()` → 入力検証を通過した場合のみ `AppConfig::with_config_mut` で
  `agents.<backend>.models` を書き換え＆`write_config` → 失敗時は当該バックエンドのみ `log::warn!` で記録し他バックエンドは継続。
  アプリ起動自体はこの spawn を待たない。

- **`supported_models` メッセージ経由の更新**:
  起動済みエージェントプロセスからの `supported_models` 受信 → `bridge_common.rs` で入力検証 →
  config.toml の `agents.<backend>.models` を書き換え → 既存契約名 `agent-models-updated` イベントを
  config.toml 由来の最新値で配信（`available_models.json` / `available_models_{backend_id}.json`
  への書き込みは行わない）。

- **モデル選択候補の取得（UI 表示用）**:
  フロントエンド `ModelSelector` → 既存契約 `get_session` の response に含まれる `available_models`
  フィールド、および既存契約 `agent-models-updated` イベント（増分配信） →
  いずれも供給元を `config.toml` の `agents.<backend>.models` に統合し、`AgentBackendRegistry::available_models(id)`
  経由で `AppConfig` から読み出す → `Vec<ModelInfo { value }>` を返却（`value` のみ／空リストもあり得る）。

- **モデル指定の検証（経路別）**:
  - **共通の前段（入力検証）**: ユーザー／ワークフローによるモデル指定 → 入力検証（入力値を変更せず、Unicode White_Space のみで構成される文字列・空文字列・制御文字を含む文字列・上限長 128 文字を超える文字列は形式不正として即拒否）。
  - **set_agent_model / AgentModelSetRequest（既存セッションへのモデル設定）**:
    `model_id` が `None`（null）の場合は当該セッションの選択モデルを未指定状態へ戻し、後続の暗黙フォールバックを行わない →
    非 null の場合は当該セッションの `backend_id` に登録されたモデル一覧（`AgentBackendRegistry::available_models()` 経由で config から取得）とのみ突合 →
    別バックエンドのモデルは backend mismatch、当該バックエンドの一覧にも無いものは登録外として、いずれも明示エラーを呼び出し元へ返す（サイレントフォールバックなし）。
    検証ロジックは `bridge_common.rs` の `set_agent_model_internal` に集約し、Tauri コマンド／WebSocket handler はこれへ委譲する。
  - **ワークフロー step 実行（新規セッション起動）**:
    `model_id` が `None`（null／未指定）の場合は当該 step session の選択モデルを未指定状態のままとし、暗黙の既定モデルへフォールバックさせない →
    非 null の場合は共通の前段（入力検証）を通した上で、既存契約の `resolve_backend_for_model` により指定 model から backend を決定し、当該 step session の backend をその backend に切り替える →
    全 backend のいずれにも登録されていないモデルは登録外として明示エラー（validation error）で拒否する（サイレントフォールバックなし） →
    同一モデルIDが複数 backend に登録されている場合は所属 backend を一意に特定できないため、明示エラー（validation error）で拒否する（いずれかの backend へサイレントに割り当てない）。
    ワークフロー経路は当該セッションの backend_id を事前に固定しないため、backend mismatch ルールは適用しない。

### 状態 Owner

- **永続的なモデル一覧（バックエンド毎）**: `config.toml`（`AppConfig` がメモリ上のキャッシュと書き出しを Own。`available_models.json` は廃止）
- **CLI 取得処理の実装**: 各 `AgentBackend` 実装（`ClaudeBackend` / `CodexBackend`）
- **同期処理のスケジューリング（起動時 1 回、バックエンド単位）**: `lib.rs` の setup フック
- **モデル選択 UI の選択状態**: フロントエンド（既存の `useSessionStore` 等、本 issue で変更しない）
- **`ModelInfo` の正規定義**: `src-tauri/src/backends/mod.rs`（`value: String` のみ）／フロントは `src/types/session.ts`（`value: string` のみ）

### 境界

- Rust ↔ フロントエンド: `ModelInfo` は `{ value }` のみ。`displayName` フィールドをシリアライズに含めない。フロントは `value` をテキストとして表示し、HTML/JS として解釈しない（メイン UI・Remote UI ともに React 既定のテキストノードとして描画し、`dangerouslySetInnerHTML` 等での挿入を行わない）。対象は `ModelInfo.displayName` のみで、`PermissionRequest.display_name`（権限ダイアログ表示名）は本変更の対象外。
- バックエンド trait ↔ config: `AgentBackend` は config を直接書き換えない。書き換えは Registry の同期処理経由でのみ行う。
- CLI 取得 ↔ 永続化: CLI 取得失敗（プロセス起動失敗・タイムアウト・パース失敗・空応答）および入力検証失敗（空文字モデルID・制御文字・上限長超過・上限件数超過）は決して config を上書きしない。重複モデルIDは除去した上で保存する。
- 既存 JSON キャッシュの廃止: `bridge_common.rs` の `available_models_path` / `save_available_models` / `load_available_models` および `available_models.json` / `available_models_{backend_id}.json` 経路は廃止する。永続化先は `config.toml` のみとし、`agent-models-updated` イベントの供給元も config.toml に統合する。
- ハードコード禁止: `*_supported_models()` 由来のリテラルなモデル一覧はコード上に残さない（テストフィクスチャ内のモックを除く）。
- 同期処理 ↔ 起動シーケンス: 同期処理はバックエンドごとに独立して `tokio::spawn` し、起動フローをブロックしない。1バックエンドの失敗は他バックエンドの同期処理に伝播しない。
- バックエンド単位の原子性: モデル一覧の更新（起動時CLI同期および `supported_models` 受信経由）は各バックエンド単位で原子的に反映する。同一バックエンドに対する複数の更新が競合した場合は、最後にコミットされた有効値を `config.toml` の値とする。1 バックエンドの更新は他バックエンドの一覧に影響を与えない（他バックエンドの一覧を失わない）。
- モデルID入力検証: モデルIDは「入力値は変更しない（trim 等の正規化を一切行わない）」「Unicode White_Space のみで構成される文字列は空白のみとして拒否」「空文字列を拒否」「制御文字（U+0000–U+001F、U+007F）を含まない」「上限長 128 文字以内（UTF-8 のコードポイント数）」「同一文字列の重複は除去し、重複除去後のユニーク件数が 1 バックエンドあたり 256 件以内」を満たす場合のみ有効。129 文字以上のモデルIDは拒否する。配列の件数上限は重複除去後のユニーク件数で判定し、重複除去後のユニーク件数が 257 件以上なら拒否する（重複除去前の入力配列が 257 件以上でも、除去後に 256 件以内に収まる場合は受け入れる）。検証に失敗した場合は `config.toml` を更新しない／指定として受け入れない。CLI由来・ユーザー由来（`set_agent_model` Tauri コマンド／`AgentModelSetRequest` WebSocket／ワークフロー step config／Remote UI 自由入力）すべてに同一の検証ルールを適用する（経路間で正規化方針は分岐させない）。
- 未登録モデル指定の扱い: 入力検証を通過した上で当該セッションの `backend_id` の `config.toml` モデル一覧に登録されていないモデルIDが指定された場合、サイレントに既定モデルへフォールバックせず、明示エラー（validation error / エラー応答）を呼び出し元（ワークフロー検証経路 / Tauri コマンド呼び出し元 / WebSocket クライアント / Remote UI）へ返す。別バックエンドに登録されたモデルIDの指定は backend mismatch として拒否する。`model_id=null`（`Option<String>=None`）は選択解除として受け入れ、当該セッションのモデル選択を未指定状態へ戻す（暗黙の既定モデルへフォールバックさせない）。
- ワークフロー経路の所属バックエンド一意性: ワークフロー step 実行経路では当該セッションの `backend_id` を事前に固定しないため、指定モデルIDから所属バックエンドが一意に特定できる場合のみ受け入れる。同一モデルIDが複数バックエンドの `config.toml` モデル一覧に登録されている場合は所属バックエンドを一意に特定できないものとして、いずれかの backend にサイレント割り当てを行わず、明示エラーで拒否する。
- Remote UI / WebSocket 経路の認可: `AgentModelSetRequest` を含むモデル設定操作の認可は、既存の WebSocket 認証（HMAC トークン）およびセッション認可スキームに従う。本変更で新たな認可経路・権限モデルは導入しない。

### 実装に委ねること

- 各バックエンドCLI問い合わせの具体的なコマンド・引数・出力パース手順（例: `codex debug models` のフォーマット、Claude 側の取得手段選定）
- CLI 呼び出しのタイムアウト値・リトライ有無
- `fetch_models_from_cli()` のメソッド名・シグネチャの細部、および `AgentBackend` trait に追加するか別 trait に切り出すかの選択
- `AgentsSection` 配下の `models` フィールドを `ClaudeAgentSection` / `CodexAgentSection` に持たせるか共通の `HashMap<String, Vec<String>>` で持たせるかの選択
- Registry 同期処理の関数名・配置モジュール、およびバックエンド単位 spawn の具体的な並行制御手段
- ログメッセージの具体文言
- 既存 `collect_all_model_values` / `resolve_backend_for_model` を config 経由に切り替える際の内部リファクタリング詳細
- `available_models.json` 既存ファイルの除去タイミング（起動時クリーンアップを行うか単に参照をやめるかの選択）
- テストケースの具体的な配置（既存テストのうち `display_name` 依存箇所の更新方法、CLI モック化の手段）
- フロントエンド側 `displayName` 参照箇所の置き換え順序（型・コンポーネント・ストア・テスト）
