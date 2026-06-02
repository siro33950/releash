# Design

## Behavior Coverage

behavior.md の各 Rule を、以下の設計方針で満たす。本 issue は純粋移行であり、すべての Rule は「外部 I/F を不変に保ったままロジックの所在を移す」ことで充足する。

- **Rule: デスクトップ利用者の Git 操作結果は移行前後で等価である**
  repository 責務の Tauri コマンドは、コマンド名・引数・戻り値 DTO・エラーの serialize 形式を移行前と等価に保つ。ビジネスロジックは usecase / domain / gateway へ移すが、フロントから観測される invoke の入出力 JSON 表現は不変とする。成立しない状況（リポジトリ未発見、ブランチ重複等）は、移行後も同じエラー表現として serialize される。

- **Rule: リモート利用者の Git 操作結果は移行前後で等価である**
  repository 責務に対応する WebSocket メッセージ型（request / response）とルーティングのディスパッチを保持する。ハンドラは usecase を呼ぶ薄い入口に再配置し、メッセージ形式・意味・付随イベントを不変に保つ。

- **Rule: repository ドメインの機能は別エントリポイントから再利用できる**
  ドメイン層・ユースケース層を Tauri / git2 / ファイル I/O 非依存にする。これにより CLI 等の別エントリポイントが、Tauri の AppState 配線に依存せずに同一の usecase を構築・呼び出し、デスクトップ／リモートと等価な操作結果を得られる。

## Key Decisions

- **repository はクリーンアーキテクチャ移行の早期実装者である。** 現状 `adaptor/` と `infrastructure/` は未作成で、`domain/`・`usecase/` も最小限しか存在しない。本移行は repository 責務の配置に必要な範囲で、`docs/architecture/` 規約に沿った各層の足場（controller / gateway / protocol、infrastructure の git2・永続化、gateway/shared の git2 共通ラッパー、横断的なアプリエラー型）を新設する。足場は後続ドメインが再利用できる形を志向するが、本 issue では repository に必要な分のみを作り、他ドメインの移行は行わない。

- **責務分割の単位は branch / commit・log / status / worktree / git_config / repo_paths とする。** 各責務に対応するドメイン抽象を切り、CQRS に従って読み取り（Query）と書き込み（Command）を分離する。commit は現状書き込みコマンドが存在せず log / status からの読み取りのみだが、責務としては commit・log を同一の歴史読み取り領域として扱う。

- **外部 I/F は完全保持する純粋移行とする。** エラー表現は規約に従い DomainError → UsecaseError → AppError の階層へ再構成するが、最終的にフロントおよびリモートへ serialize される表現を現行（GitError 等）と等価に保つことを契約とする。

- **repo_paths の永続化は app_config（config.toml）に依存するが、app_config は移行しない。** repo_paths はリポジトリパス一覧をアプリ設定ファイルへ永続化する責務であり、その保存先は app_config ドメインが所有する設定機構である。本移行では repository の repo_paths 永続化抽象を定義し、その実装が既存のアプリ設定永続化機構を利用する形にとどめる。app_config 自体の層移行は非対象とする（requirements Non-goals に整合）。

- **採らなかった代替案: 旧 `git/` モジュールを温存し薄いラッパーのみ被せる案。** 実装コストは小さいが、「ドメイン層が外部リソース非依存」「旧構成の対応モジュール除去」「別エントリポイントからの再利用」という要件を満たせないため採らない。

## Responsibility Boundaries

| 領域 | 担当すること | 担当しないこと |
|---|---|---|
| domain（repository） | branch / commit・log / status / worktree / git_config / repo_paths のエンティティ・値オブジェクト・不変条件、永続化／外部リソースの抽象（trait）、ドメインサービス | git2・ファイル I/O・Tauri・tokio への依存、DTO の serialize 形式 |
| usecase（repository） | 各責務の Command / Query 業務手順、入力バリデーション、ドメイン抽象の組み合わせ、表示向け DTO への変換 | 具体的な外部リソース実装の知識、Tauri / git2 の直接利用 |
| adaptor/controller | Tauri コマンドと WebSocket ハンドラの薄い入口、引数の受け渡しと型変換、起動時 DI 配線の register | ビジネスロジック |
| adaptor/gateway | ドメイン抽象の具体実装、git2／ファイルアクセスの封じ込め、ドメイン型 ↔ 外部システム型の変換、外部エラー → ドメインエラー変換 | 業務手順、表示整形 |
| infrastructure | git2 クライアントとファイル／設定永続化の薄いラッパー | ドメイン知識、変換ロジック |
| adaptor/protocol | WebSocket メッセージ・コマンド引数の DTO 定義 | ドメイン型そのもの |

上表の「domain は DTO の serialize 形式を持たない」「adaptor/protocol はドメイン型そのものを持たない」は、**表示・転送の都合で形が決まる型を domain/protocol に置かない**という意味であり、`Serialize`/`Deserialize` 派生の一律禁止ではない。型の所属は [DOMAIN.md](../../architecture/DOMAIN.md) 「Entity か DTO か」の判定基準（**誰の都合でその形が決まっているか**）に従う。git のブランチ・コミット・ファイル状態のように domain 概念として形が定まり表示・転送都合で歪まない 1:1 同型の Entity は、serde を付与して controller から直接返してよく、同型の DTO を別途挟まない。1:1 同型の DTO 複製を強制することは DTO の濫用であり本方針の意図ではない。

`git/` 配下のうち code ドメイン責務（diff、hunk、patch、staging、lang、ファイル内容、diff_tree、branch_diff）のファイルは本移行の対象外であり、`git/` に残存することを許容する。repository 移行に必要な範囲での共有ユーティリティ・共有型の整理に限り、これら code 所属ファイルへの波及を許容する。

## Contracts

外部から観測可能な契約。いずれも移行前後で等価に保つ。

- **Tauri コマンド（repository 責務、移行前後で名前・引数・戻り値・エラー表現を不変に保つ）**
  - branch: `list_branches` / `get_current_branch` / `get_default_branch` / `git_create_branch` / `delete_branch`
  - commit・log: `get_git_log`
  - status: `get_git_status` / `get_status_diff_stats`
  - worktree: `get_main_repo_path` / `get_worktree_dirty_count` / `list_worktrees` / `list_branches_with_status` / `create_worktree` / `remove_worktree`
  - git_config: `get_releash_base` / `set_releash_base` / `get_branch_base` / `set_branch_base`
  - repo_paths: `get_repo_paths` / `add_repo_path` / `remove_repo_path`
  - util（リポジトリパス系）: `get_cwd` / `get_repo_git_dir`

- **WebSocket メッセージ（形式・意味を不変に保つ）**
  - branch: branch 情報取得の request / response
  - worktree: worktree 一覧取得の request / response、worktree 選択の request / response

- **イベント（名前・ペイロード・発火タイミングを不変に保つ）**
  - repo_paths 変更時に発火するアプリイベント（`repo-paths-changed`）。worktree 等の操作で現状発火するイベントがあれば同様に保持する。

- **永続データ・設定フォーマット（不変）**
  - git config キー: `releash.base`（global）および `branch.<name>.releash-base`（per-branch）の読み書き形式と解決順序。
  - repo_paths の永続化先: アプリ設定ファイルの該当キー（`app.last_repo_paths`）の形式。

- **エラー serialize 契約**
  - フロント／リモートへ返却されるエラーの構造化表現を、移行前の表現と等価に保つ。

## Data / Communication Flow

境界をまたぐ主要 flow。

- **Tauri 経由**: フロント `invoke` → controller/command（薄い入口）→ usecase または query service → domain 抽象（trait）→ gateway 実装 → infrastructure（git2 / ファイル）→ 結果を DTO に整形して返却。

- **WebSocket 経由**: クライアントメッセージ → ルーティング → controller/handler（薄い入口）→ usecase または query service → 以降は Tauri 経由と同一の流れ → response メッセージを返却。

- **repo_paths 変更**: コマンド／ハンドラ → usecase → repo_paths 永続化抽象（アプリ設定機構へ保存）→ 変更イベントを発火。

- **別エントリポイント（CLI 等）**: usecase を直接構築 → domain 抽象 → gateway 実装 → infrastructure。Tauri / WebSocket を経由しない。

## State Ownership

- **branch / commit・log / status / worktree / git_config**: 信頼できる状態の所有者はディスク上の Git リポジトリ自体（git2 でアクセス）。これらのドメイン抽象・gateway はステートレスであり、メモリ上に独自の状態を保持しない。

- **repo_paths**: メモリ上の共有リスト（現 `SharedRepoPaths`）とアプリ設定ファイルの永続値を持つ。論理的な所有者は repository ドメインの repo_paths 責務だが、永続化先のファイルは app_config ドメインが所有する設定機構であり、repository はそれを利用するのみ（所有しない）。

- **DI 受け皿**: repository 責務の usecase / query service インスタンスは AppState（adaptor/controller/state）が `Arc` で保持する。

## Boundaries

越えてはいけない責務境界。

- domain 層は `tauri` / `git2` / `tokio` / ファイル I/O を直接参照しない。
- usecase 層は domain の抽象（trait）のみに依存し、gateway / infrastructure の具体実装を知らない。
- 依存方向は `infrastructure → adaptor（controller / gateway / presenter）→ usecase → domain`（依存は内向きのみ）に従い、逆方向（内側の層が外側の層に依存する向き）の依存を生まない。
- controller（Tauri コマンド / WebSocket ハンドラ）はビジネスロジックを持たず、usecase を呼ぶ薄い入口に徹する。
- code ドメイン責務のファイルは本移行で層移行しない。波及は repository 移行に必要な共有ユーティリティ・共有型の整理に限定する。
- app_config ドメインは移行しない。repo_paths は app_config の永続化機構を借用するにとどめ、app_config の責務を repository 側へ取り込まない。
- 外部から観測可能な振る舞い（コマンド I/F・WebSocket メッセージ・イベント・Git 操作結果・エラー表現）を一切変えない。
- 移行後に旧構成の対応モジュールを除去し、同一責務の重複実装を残さない。

## Implementation Freedom

実装に委ねる。

- 各責務のドメイン抽象（trait）の粒度と分割（責務ごとに分けるか統合するか）。
- usecase を `Arc<dyn Trait>` で trait 化するか、構造体のまま AppState に持たせるか（規約のデフォルトは「迷ったら構造体」）。
- 各層内のファイル分割の具体（CQRS の統合／分離の度合い、models ファイルの分け方）。
- gateway/shared の git2 共通ラッパーの形と、infrastructure の git2 クライアント／永続化ラッパーの構造。
- 起動時 DI 配線で register 関数を用いる際の、Tauri `invoke_handler` 単一呼び出し制約の解消方法（ドメインごと register か、関数リスト集約か）。ただし repository 責務の全コマンドが移行後も登録され invoke 可能であること、および未移行の code ドメインコマンドが共存し続けることを要件とする。
- DomainError / UsecaseError / AppError の variant 設計（serialize 結果の等価性は契約として維持する）。
- commit 責務を log と同一モジュールにまとめるか分けるか。
- git2 のブロッキング呼び出しを非同期境界へ載せる方法（spawn_blocking の適用箇所等）。
- 内部 helper の命名・配置。
