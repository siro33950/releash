## 要求

**種別**: リファクタリング
**ゴール**: パーミッションモードをプロバイダ非依存の3段階（`readonly` / `edit` / `full`）に統一し、ワークフローYAML・UI・セッション設定の全てで抽象モードのみを露出する。各CLIバックエンド固有のフラグ（Claude: `acceptEdits` / `bypassPermissions` / `plan` / `default`、Codex: `approval_policy` × `sandbox_mode`）への変換責務はRust側のバックエンド層に集約する
**背景**: 現在のパーミッションモードはClaude CLI固有の語彙（`acceptEdits` / `bypassPermissions` / `plan`）がワークフローYAML・UI・Rust型まで全層に露出しており、以下の問題を抱える
- Codexバックエンドには `acceptEdits` 等の概念が存在せず、マルチバックエンド構成で意味が通らない
- Codexは `approval_policy` × `sandbox_mode` の2軸であり、Claude単一軸の語彙とは直接対応しない
- 動作上はCodex側のJS bridge（`codex-sdk-bridge-utils.mjs`）が裏で変換しているが、マッピングがJS bridge側に隠れてRust型システムには見えず、責務配置として不適切（rust-first-logic原則違反）
- Codexユーザにとって、YAMLに書く語彙がClaude用なのは直感に反する

### 成功基準

- ワークフローYAMLの `permission:` フィールドは `readonly` / `edit` / `full` のみを受け付ける
- 対象外の値（`readonly` / `edit` / `full` 以外。代表例として旧語彙 `acceptEdits` / `bypassPermissions` / `plan` / `default`、未知語彙、空文字、未指定・欠落を含む）がYAMLや保存済みセッションに含まれている場合はバリデーションエラーで起動を拒否する（破壊的変更）。エラーメッセージには許可される抽象モード一覧（`readonly`, `edit`, `full`）を含む
- セッション設定UI・ワークフロー編集UI・リモート（モバイル/タブレット）UIともに3択（`Read Only` / `Edit` / `Full`）で表示・選択できる
- Claudeバックエンド実行時は `readonly` → `default`、`edit` → `acceptEdits`、`full` → `bypassPermissions` にRust側で変換されてCLIに渡る
- Codexバックエンド実行時は以下にRust側で変換されてCLIに渡る
  - `readonly` → `sandbox_mode=read-only` / `approval_policy=never`
  - `edit` → `sandbox_mode=workspace-write` / `approval_policy=on-request`
  - `full` → `sandbox_mode=danger-full-access` / `approval_policy=never`
- JS bridge（`codex-sdk-bridge-utils.mjs`）側に隠れていたマッピング処理はRust側に移譲され、JS bridgeは具体的なCodexフラグをそのまま受け取って渡すだけになる
- Tauri invoke ペイロードおよびリモート WebSocket ペイロード（`AgentMessageRequest.permission_mode` 等）の境界で対象外の値を受信した場合、Rust側で拒否しセッション状態を変更せず bridge へ送らずバリデーションエラーを返す
- 権限ランク `readonly(0) < edit(1) < full(2)` がRust側で表現される
- 既存テスト（Issue #939由来含む）が新語彙に更新され、`pnpm lint` / `pnpm test` / `cargo clippy -- -D warnings` / `cargo test` が全て通る

### 許可主体

- パーミッションモードの選択・変更操作（`full` を含む）は、対象UI（セッション設定 / ワークフロー編集 / リモートエージェント操作画面）にアクセスできるユーザーであれば行える。リモートUIは既存の HMAC トークン認証によるアクセス制御をそのまま継承する。
- 本Issueではロールベースの細分化や `full` 専用の追加ガードは導入しない。

### スコープ

- **対象**: パーミッションモードの表現と変換責務の再配置（Rust・フロント・JS bridge・YAMLスキーマ・UI・リモートUI・関連テスト）
- **スコープ外**:
  - `required_permission_mode`（ステップが要求する最低権限）の宣言機構
  - プロファイルベースの上書き機構
  - 旧語彙データの自動マイグレーション（破壊的変更とし、ユーザに手動更新を求める）
  - ロールベースのアクセス制御細分化や `full` 専用の追加認可（既存アクセス制御を継承するのみ）

## 振る舞い定義

```gherkin
Feature: プロバイダ非依存のパーミッションモード

  Rule: ワークフロー定義は抽象パーミッションモードのみを受け入れる

    Scenario Outline: 抽象モードを指定したワークフローは検証を通過する
      Given ワークフローステップに抽象モード "<抽象モード>" が指定されている
      When ワークフローを読み込む
      Then ワークフローは妥当と判定される

      Examples:
        | 抽象モード |
        | readonly   |
        | edit       |
        | full       |

    Scenario Outline: 対象外のパーミッション値を指定したワークフローは起動を拒否される
      Given ワークフローステップのパーミッションに対象外の値 "<対象外の値>" が指定されている
      When ワークフローを読み込む
      Then 起動はバリデーションエラーで拒否される
      And エラーメッセージには許可される抽象モード一覧（readonly, edit, full）が含まれる

      Examples: 旧語彙（破壊的変更により対象外）
        | 対象外の値        |
        | acceptEdits       |
        | bypassPermissions |
        | plan              |
        | default           |

      Examples: 未知語彙・形式不正
        | 対象外の値 |
        | unknown    |
        | readwrite  |
        | (空文字)   |

    Scenario: ステップにパーミッションを指定しないワークフローは起動を拒否される
      Given ワークフローステップにパーミッションが指定されていない
      When ワークフローを読み込む
      Then 起動はバリデーションエラーで拒否される
      And エラーメッセージには許可される抽象モード一覧（readonly, edit, full）が含まれる

    Scenario Outline: 並列ステップ配下の子ステップのパーミッションも同じ検証対象である
      Given 並列ステップの子ステップのパーミッションに対象外の値 "<対象外の値>" が指定されている
      When ワークフローを読み込む
      Then 起動はバリデーションエラーで拒否される
      And エラーメッセージには許可される抽象モード一覧（readonly, edit, full）が含まれる

      Examples:
        | 対象外の値        |
        | acceptEdits       |
        | unknown           |
        | (空文字)          |
        | (未指定)          |

  Rule: 保存済みセッションは抽象パーミッションモードのみを受け入れる

    Scenario Outline: 抽象モードが保存されたセッションは保存値のまま復元される
      Given セッションのパーミッションモードに抽象モード "<抽象モード>" が保存されている
      When セッションを起動する
      Then セッションは保存値のまま復元される

      Examples:
        | 抽象モード |
        | readonly   |
        | edit       |
        | full       |

    Scenario Outline: 対象外のパーミッションモードが保存されたセッションは起動を拒否される
      Given セッションのパーミッションモードに対象外の値 "<対象外の値>" が保存されている
      When セッションを起動する
      Then 起動はバリデーションエラーで拒否される
      And エラーメッセージには許可される抽象モード一覧（readonly, edit, full）が含まれる

      Examples: 旧語彙（破壊的変更により対象外）
        | 対象外の値        |
        | acceptEdits       |
        | bypassPermissions |
        | plan              |
        | default           |

      Examples: 未知語彙・形式不正・欠落
        | 対象外の値             |
        | unknown                |
        | (空文字)               |
        | (フィールドなし/欠落)  |

  Rule: 抽象モードはバックエンドの種類に応じた具体的な権限設定としてCLIに反映される

    Scenario Outline: Claudeバックエンドでは抽象モードがClaude固有の権限設定として適用される
      Given Claudeバックエンドが選択されている
      And パーミッションモードに "<抽象モード>" が指定されている
      When セッションを起動する
      Then Claudeバックエンドは抽象モードに対応するClaude固有の権限設定で動作する

      Examples:
        | 抽象モード |
        | readonly   |
        | edit       |
        | full       |

    Scenario Outline: Codexバックエンドでは抽象モードがCodex固有の権限設定として適用される
      Given Codexバックエンドが選択されている
      And パーミッションモードに "<抽象モード>" が指定されている
      When セッションを起動する
      Then Codexバックエンドは抽象モードに対応するCodex固有の権限設定で動作する

      Examples:
        | 抽象モード |
        | readonly   |
        | edit       |
        | full       |

  Rule: 権限ランクは readonly < edit < full の順序を持つ

    Scenario Outline: 抽象モード間の権限ランク比較
      Given 2つの抽象モード "<低>" と "<高>"
      When 両者の権限ランクを比較する
      Then "<低>" は "<高>" より低位と判定される

      Examples:
        | 低       | 高    |
        | readonly | edit  |
        | edit     | full  |
        | readonly | full  |

  Rule: ユーザーはアクセス場所に関わらず同じ抽象モード3択からパーミッションモードを選択する

    Scenario Outline: パーミッションモードの選択肢は抽象モード3択のみが提示される
      Given ユーザーが "<選択場所>" でパーミッションモードを変更しようとしている
      When パーミッションモード選択肢が提示される
      Then 3つの抽象モード（readonly / edit / full）に対応する選択肢のみが提示される
      And 対象外の値（readonly/edit/full 以外。代表例として旧語彙 acceptEdits 等）は提示されない

      Examples: パーミッションモードを選択できる場所
        | 選択場所                       |
        | セッション設定                 |
        | ワークフロー編集               |
        | リモートエージェント操作画面   |

    Scenario Outline: バックエンドの種類によって選択肢の内容は変わらない
      Given バックエンドに "<バックエンド>" が選択されている
      When パーミッションモード選択肢が提示される
      Then 3つの抽象モード（readonly / edit / full）に対応する選択肢のみが提示される

      Examples:
        | バックエンド |
        | Claude       |
        | Codex        |

    Scenario Outline: セッション設定UIまたはリモート操作画面での選択結果はセッションに記録される
      Given ユーザーが "<選択場所>" で抽象モードのいずれかを選択する
      When 選択が反映される
      Then 選択された抽象モードがそのままセッションのパーミッションモードに記録される

      Examples:
        | 選択場所                     |
        | セッション設定               |
        | リモートエージェント操作画面 |

    Scenario: ワークフロー編集UIでの選択結果はワークフローステップ定義に記録される
      Given ユーザーがワークフロー編集UIで抽象モードのいずれかを選択する
      When 選択が反映される
      Then 選択された抽象モードがそのままワークフローステップ定義のパーミッションに記録される

  Rule: 外部から対象外のパーミッションモードを要求された場合は受け付けず現状を維持する

    Scenario Outline: 対象外の値によるパーミッションモード変更要求は拒否される
      Given セッションが稼働している
      When 対象外の値 "<対象外の値>" でパーミッションモード変更が要求される
      Then 要求はバリデーションエラーで拒否される
      And セッションのパーミッションモードは変更されない
      And 稼働中セッションの有効な権限も変更されない

      Examples:
        | 対象外の値        |
        | acceptEdits       |
        | bypassPermissions |
        | plan              |
        | default           |
        | unknown           |
        | (空文字)          |
        | (フィールドなし)  |
```

## アーキテクチャ概要

### 責務配置

- **Rustコア型層**（`src-tauri/src/` 直下の型定義 — 配置先は実装判断）:
  - 担当: 抽象パーミッションモード（`readonly` / `edit` / `full`）の唯一の表現を提供する。3値の列挙、シリアライズ語彙、権限ランク順序、対象外の値（旧語彙・未知語彙・空文字・欠落を含む）→拒否のパース失敗、を一箇所に集約する。
  - 担当しない: 各CLIバックエンド固有フラグの定義・保持。

- **ワークフロースキーマ・検証層**（`src-tauri/src/workflow/schema.rs`, `src-tauri/src/workflow/validation.rs`）:
  - 担当: YAMLステップの `permission` フィールドを抽象モード型に受け取り、対象外の値（旧語彙・未知語彙・空文字・未指定）をバリデーションエラーで拒否する。parallel ステップの子ステップ permission も同じ検証対象とする。エラーメッセージで許可される抽象モード一覧（`readonly`, `edit`, `full`）を提示する。
  - 担当しない: バックエンド固有フラグへの変換。

- **セッション保存層**（`src-tauri/src/session/store.rs`, `src-tauri/src/session/mod.rs`）:
  - 担当: セッションJSONに抽象モードのみを保存・復元する。復元時に対象外の値（旧語彙・未知語彙・空文字・欠落）を検出したら起動を拒否する。
  - 担当しない: 対象外値（旧語彙含む）の自動マイグレーション（破壊的変更）。

- **通信プロトコル層**（`src-tauri/src/protocol/agent.rs`, `src-tauri/src/ws_server/`, `src/types/session.ts`, `src/types/protocol.ts`）:
  - 担当: Rust ↔ フロント間（Tauri invoke ペイロード）および Rust ↔ リモート間（WebSocket `AgentMessageRequest.permission_mode` 等）で抽象モードのみをやり取りする共通語彙を提供する。境界で対象外の値を受信した場合、Rust 側で拒否し、セッション状態を変更せず bridge へ送らずバリデーションエラーを返す。
  - 担当しない: CLI固有フラグの伝搬。

- **バックエンド変換層**（`src-tauri/src/backends/claude.rs`, `src-tauri/src/backends/codex.rs`, `src-tauri/src/backends/bridge_common.rs`）:
  - 担当: 抽象モード→各CLI固有フラグへの変換責務を一手に引き受ける。Claude側は `default` / `acceptEdits` / `bypassPermissions`、Codex側は `sandbox_mode`（`read-only` / `workspace-write` / `danger-full-access`）および `approval_policy`（`readonly`→`never` / `edit`→`on-request` / `full`→`never`）を組み立て、JS bridgeに渡す init/setMode 引数に格納する。UI由来の setMode 相当コマンドについても、Rust 変換層が抽象モード→CLI固有フラグに変換した結果を bridge runtime に送る。
  - 担当しない: 抽象モード自体の保持、UI表現。

- **JS Bridge層**（`src-tauri/resources/codex-sdk-bridge-utils.mjs`, `src-tauri/resources/codex-sdk-bridge-runtime.mjs`, `src-tauri/resources/claude-sdk-bridge.mjs`）:
  - 担当: Rust側から受け取った具体的なCLIフラグをそのまま該当CLI/SDKに転送する薄いパススルーになる。Codex bridge utils 内の `approvalPolicyFromPermissionMode` / `sandboxModeFromPermissionMode` 相当のマッピング関数は廃止または無効化する。
  - 担当しない: 抽象モードの解釈、フラグへの変換。

- **UI層**（`src/components/panels/AgentChatPanel/ModeSelector.tsx`, `src/components/panels/AgentChatPanel/CodexPermissionControl.tsx`, ワークフロー編集UI、`src/remote/components/RemoteAgentPanel.tsx` 等のリモートUI）:
  - 担当: 抽象モード3択（`Read Only` / `Edit` / `Full`）のみを表示・選択させる。バックエンドに応じた表示差異を持たない（バックエンドの違いはRust変換で吸収済み）。選択結果は Tauri invoke / WebSocket ペイロードに抽象モードのみを載せて送信し、対象外の値は送信しない。
  - 担当しない: バックエンドごとに異なる選択肢の出し分け、CLI固有用語の露出。

- **ビルトインワークフロー定義**（`src-tauri/src/workflow/builtin/spec-driven-development.yml`）:
  - 担当: 全ステップに抽象モードの `permission:` フィールドを明示的に付与する。書込系（Spec/コード更新）は `edit`、レビュー・判断系は `readonly` を割り当て、`full` は使用しない。
  - 担当しない: バックエンドごとの差異吸収、デフォルトモードの動的決定。

  ステップごとの割り当て:

  | ステップ | permission |
  | --- | --- |
  | plan_requirements / plan_behavior / plan_architecture / plan_fix | edit |
  | plan_review_completeness / plan_review_clarity / plan_review_security / plan_review_consistency | readonly |
  | plan_fix_policy / plan_approval | readonly |
  | implement / fix | edit |
  | code_review_acceptance / code_review_structure / code_review_quality / code_review_test / code_review_security / code_review_architecture | readonly |
  | implementation_fix_policy / implementation_approval | readonly |

### データ/通信フロー

- **ワークフロー読込**: YAML → `workflow/schema` でデシリアライズ → `workflow/validation` で抽象モード検証（対象外の値は拒否） → ステップ実行時に抽象モードのまま `workflow/engine` が保持。
- **セッション起動 / 復元**: SessionStoreから抽象モード読出 → 抽象モード検証 → `backends/*` が抽象モード→CLI固有フラグに変換 → bridge_common 経由でJS bridge init コマンドに具体的フラグを載せる → JS bridge は変換せず CLI に渡す。
- **UIからのパーミッション変更（セッション設定UI / リモートエージェント操作画面）**: UI（3択） → Tauri invoke / WebSocket（抽象モードのみ。対象外値は Rust 側境界で拒否） → Rust側でセッションのパーミッションモードを保存（抽象モード） → Rust バックエンド変換層が抽象モード→CLI固有フラグに変換し、setMode 相当コマンドに具体的フラグを載せて稼働中の bridge runtime に送る → bridge runtime はそのまま CLI に反映する（稼働セッションへ即時反映）。
- **UIからのパーミッション変更（ワークフロー編集UI）**: UI（3択） → Tauri invoke（抽象モードのみ。対象外値は Rust 側境界で拒否） → Rust側でワークフロー定義のステップ `permission` を保存（抽象モード）。稼働中セッションへの即時反映は行わず、CLI への反映は当該ステップが次回起動される際にワークフローエンジン経由で行われる（「ステップ実行時の権限切替」フローに合流）。
- **ステップ実行時の権限切替**: `workflow/engine` が抽象モードを `bridge_common::sync_pre_turn_settings` に渡し、内部でバックエンド変換層を経て具体的フラグに変換された上で bridge に送出される。
- **ビルトインワークフロー起動**: `spec-driven-development.yml` のステップごとに記載された抽象モード `permission` がYAMLパーサーで検証され、エンジンが各ステップ実行直前にバックエンド変換層へ渡す。

### 状態Owner

- **抽象モードの語彙定義（3値・ランク順序）**: Rustコア型層（単一の正典）。
- **ワークフローステップごとの permission 値**: ワークフロースキーマ層（YAML起源、抽象モード）。
- **セッションごとの permission_mode 値**: セッション保存層（SessionStore、抽象モード）。
- **CLI固有フラグ（acceptEdits / sandbox_mode / approval_policy 等）**: どこにも永続保存しない。バックエンド変換層の出力としてその場で生成され、JS bridgeに渡って消費されるだけの揮発値。
- **UIの選択状態（表示中の選択）**: 各UIコンポーネント（フロントエンド、ただし永続化はRust側Storeに往復）。

### 境界

- Rustコア型より外側のあらゆる境界（YAMLテキスト、セッションJSON、Tauri invoke ペイロード、WebSocketリモート通信ペイロード、フロント型、UI表示）には抽象モードの3値のみが出現する。これらの場所に対象外の値（`readonly` / `edit` / `full` 以外。代表例として旧語彙 `acceptEdits` / `bypassPermissions` / `plan` / `default`、未知語彙、空文字、欠落）が出現してはならない。
- CLI固有フラグの語彙はバックエンド変換層の出力以降（JS bridge init/setMode の引数、CLIのコマンドライン）にのみ出現する。Rust側で抽象モードを保持するコード、フロント、UIに漏出してはならない。
- JS bridge層は「具体的フラグを受け取ってそのまま渡す」境界に位置する。抽象モードの解釈責務を持たない（rust-first-logic原則の徹底）。
- 対象外の値との互換は持たない。境界での検出時はバリデーションエラーで起動拒否（マイグレーションは行わない）。Tauri invoke / WebSocket 入力で対象外の値が来た場合、Rust 側で拒否し、セッション状態を変更せず bridge へ送出せずエラーを返す。

### 実装に委ねること

- 抽象モード型の名称（`PermissionMode` / `PermissionLevel` / `AbstractPermissionMode` 等）と配置モジュール（`backends/mod.rs` 内 / `protocol/` 配下 / 新規 `permission.rs` 等）の選択。
- 権限ランク順序の表現方法（`#[derive(PartialOrd, Ord)]` を使う / `rank() -> u8` メソッドを持つ / 比較関数を別途定義する 等）。
- 抽象モード→CLI固有フラグへの変換関数のシグネチャ・配置（バックエンドtrait拡張 / 各バックエンドimpl内の private fn / `From`/`Into` 実装 等）。
- 対象外の値検出時のエラーメッセージ文言（成功基準を満たし、許可される抽象モード一覧を含む範囲内で）。
- JS bridge側のマッピング関数の扱い（完全削除 / 恒等関数化 / `@deprecated` を残す 等）と、bridge側のテストファイル（`codex-sdk-bridge-utils.test.mjs`）の更新方針。
- UIコンポーネントの再利用範囲（既存 `ModeSelector` の改修 / `CodexPermissionControl` の統合または削除 / 共通の3択コンポーネントへの集約 / リモートUIへの同一コンポーネント適用）。
- UIラベル文言の細部（`Read Only` を `Read-Only` にする等）。
- テストケースの具体的配置（既存テストファイル内追加 / 新規 `*.test.*` ファイル作成）と細かい補助モックの構成。
- セッション復元時の対象外値拒否を「セッション読み込み時」「セッション起動時」のどの呼び出しタイミングで行うか。
- ビルトインワークフローYAMLにおける `permission:` フィールドの記述位置（`model:` の隣など）と、コメントを残すかどうか。なお、ステップごとの割り当て値そのものは責務配置に記載済みの表に従う。
