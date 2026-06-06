# Design

## Source
- requirements.md
- behavior.md

本設計は純粋リファクタリング（issue 974）の設計境界を定義する。外部から観測可能な振る舞いは
behavior.md の全 Rule のとおり移行前後で等価に保つ。設計の目的は実装手順の規定ではなく、
`code` ドメインの責務・契約・状態 owner・層境界を `docs/architecture/` の規約に整合させて
明確化することにある。

## Behavior Coverage

behavior.md の各 Rule は「移行前後で結果が等価」という不変条件であり、設計はこれを
「同一責務を 1 つの実装に集約し、外部契約（Tauri コマンド I/F・WebSocket メッセージ・
共有型）を保持したまま層へ再配置する」ことで満たす。

| Rule | 設計上の満たし方 |
|---|---|
| ファイル内容の参照結果は等価（at_ref / at_branch_base / staged、binary 含む） | ファイル内容参照を `code` ドメインの責務として 1 実装に集約し、対応 Tauri コマンドは引数・戻り値・エラー表現を保持して usecase へ委譲する |
| 差分の閲覧結果は等価（diff・変更ファイル一覧 / diff_tree・branch_diff） | diff / diff_tree / branch_diff のロジックを domain + gateway（git2 アクセスは gateway/infra に閉じる）へ再配置し、出力型を保持する |
| 差分の部分単位の取り扱いは等価（hunk・patch） | hunk 区切り算出と patch 生成を domain の純粋ロジックとして集約し、結果（区切り・内容・パッチ文字列）を保持する |
| 差分の Approve（staging）結果は等価 | staging（stage / unstage / stage_hunk / unstage_hunk）を usecase 経由の書き込み操作として再配置し、ステージ範囲の結果を保持する |
| 言語判定の結果は等価 | language 判定を domain の純粋ロジックとして集約し、戻り値を保持する |
| ファイルメンション補完の結果は等価 | file_mention の候補列挙・参照解決を `code` ドメインへ移し、補完候補と解決結果、および公開型を保持する |
| 表示／非表示範囲の算出結果は等価（visible / hidden ranges） | visible/hidden range 算出を domain の純粋ロジックとして集約し、範囲結果を保持する |
| リモートクライアントから見た振る舞いも等価 | リモート入口（WebSocket）が存在する責務は薄いハンドラとしてアダプタ層へ再配置し、メッセージ形式・意味を保持する。リモートが Tauri コマンドと同じ契約を経由する場合はコマンド契約の保持がそのまま等価性を担保する |

## Key Decisions

- **クリーンアーキテクチャ規約への全面準拠**: `code` ドメインを domain / usecase /
  adaptor(controller・gateway・presenter) / infrastructure の各層へ分割する。配置基準は
  「どのドメインに凝集するか」であり、外部リソース依存の有無ではない（DOMAIN.md 原則）。
- **先行移行済み `repository` ドメインのパターンを踏襲**: error 変換チェーン
  （domain error → usecase error → `AppError`、`Display`/serialize 表現を保持）、
  `spawn_blocking` でブロッキング git2 呼び出しを非同期境界へ載せる薄いコマンド、
  composition root（controller）での DI 配線、という確立済みパターンに揃える。
- **CQRS の適用は責務の複雑さに見合う範囲に限定**: 読み取り（ファイル内容参照・diff 閲覧・
  hunk/patch/range 算出・language・mention 補完）が支配的なドメインであり、Command/Query の
  サービス分離は staging のような書き込みを伴う責務に対して意味を持つ。分離が複雑さに
  見合わない責務は統合してよい（GATEWAY.md / USECASE.md）。trait 化は「複数実装・モック
  差し替えが要る」場合に限り、迷ったら構造体直持ちから始める。
- **純粋ロジックは domain に置く**: hunk 区切り・patch 生成・language 判定・visible/hidden
  range 算出は git2 / ファイル I/O に依存しない純粋関数であり、domain のドメインサービス
  ／値オブジェクトとして集約する。外部リソースに触れる責務（ファイル内容参照・diff・
  staging）は domain で抽象（trait）を定義し、具体実装を gateway/infra に閉じる。
- **転送表現（serde）の層分離**: `code` ドメインの値オブジェクト（diff / hunk / range /
  diff_tree 系）は serde 非依存の純粋データとする。フロントへの転送表現（フィールド名・
  `camelCase`・省略）は usecase の DTO（`usecase/code_dto`）と controller 入口の入力型
  （`adaptor/protocol/code`）が保持し、移行前と等価に保つ（DOMAIN.md「フロントの都合を
  domain に漏らさない」/ USECASE.md「DTO は QueryService の Response」/ CONTROLLER.md
  「コマンド引数は protocol 型」準拠）。QueryService は domain サービスの算出結果（VO）を
  DTO へ詰め替えて返し、controller は入力型を VO へ変換して usecase へ渡す。
  `MentionReference` も他の domain VO と同じく serde 非依存の純粋データとし、フロント／
  永続化への転送表現（camelCase・行範囲省略）は adaptor の入出力型
  （`adaptor/protocol/mention::MentionReferenceInput`）が所有する。agent / session /
  workflow 等の他エントリポイントの公開境界はこの Input 型を受け取り、境界の直後に
  `into_domain()` で domain VO へ詰め替えてから usecase へ渡す。
- **共有ユーティリティ・共有型の扱い**: `git/` 配下に残る共有 util／型のうち `code` に
  属するものだけを本移行で整理する。`repository` と共有する型・util に変更が要る場合は、
  `repository` 側の配置・振る舞いの規約準拠を崩さない範囲に限定する（採らなかった代替案:
  共有層の大規模再編は scope 外であり本移行では行わない）。

## Responsibility Boundaries

- **domain/code**: `code` ドメインの概念・不変条件・純粋ロジックを表現する。ファイル内容
  参照・diff・diff_tree・branch_diff・staging に対する外部リソース抽象（trait）を定義する。
  外部ライブラリ（git2・tokio・tauri・std I/O）を直接 `use` しない。
- **usecase/code**: `code` の業務手順を表現する。domain の抽象のみに依存し、Command 側
  （staging 等の状態変更）と Query 側（内容参照・diff 閲覧・各種算出）を CQRS に従い分離する
  （複雑さに見合わなければ統合）。フロント／転送都合の read model（DTO）はここで定義する。
- **adaptor/controller/command/code**: `code` 責務の Tauri コマンド。引数の受け渡しと
  型変換のみを行い、ビジネスロジックを持たず usecase を呼ぶ。
- **adaptor/controller/handler/code**: `code` 責務にリモート入口が存在する場合の WebSocket
  ハンドラ。薄い入口に徹する。
- **adaptor/gateway/code**: domain が定義した trait の具体実装。git2・ファイルシステムの
  呼び出しと、ドメイン型 ↔ 外部型の変換を内部に閉じる。
- **infrastructure**: git2 クライアント・ファイルシステムアクセスの薄いラッパー。
- **しないこと**: いずれの層も Git 操作そのものの仕様変更・アルゴジズム挙動変更を行わない。
  `repository` ドメインの責務（branch / commit / log / worktree / status / repo_paths /
  git_config）には手を入れない。フロント／リモートクライアント側のコードは変更しない。

## Contracts

外部から見える契約。これらは移行前後で等価に保つ（引数・戻り値・エラー表現・メッセージ
形式・公開型）。内部 helper・詳細型・関数シグネチャは契約に含めない（実装の自由）。

### Tauri コマンド（I/F を保持）

- ファイル内容参照: `get_file_at_ref` / `get_staged_content` / `get_binary_staged_content` /
  `get_file_at_branch_base` / `get_binary_file_at_branch_base` / `get_binary_file_at_ref`
- diff / diff_tree / branch_diff: `build_diff_file_tree` / `get_file_navigation` /
  `get_branch_diff_summary` / `get_relative_path`
- hunk / patch / range: `compute_diff_hunks` / `generate_group_patch` /
  `compute_hidden_ranges` / `compute_hidden_ranges_from_content` /
  `compute_visible_markdown_blocks`
- staging: `git_stage` / `git_unstage` / `git_stage_hunk` / `git_unstage_hunk`
- language: `get_language_from_path`
- file_mention: `list_mentionable_files`

各コマンドの引数名・型、戻り値型、失敗時の serialize 表現（エラー文字列）を移行前と
等価に保つ。コマンドはアプリ起動時の DI 配線（composition root）に整合して登録する。

### WebSocket メッセージ

`code` 責務にリモート入口が存在する場合、そのメッセージの形式・意味を移行前と等価に保つ。

### 他エントリポイントへの公開型・関数

- `file_mention` のメンション参照型（agent / backends / workflow / session が prompt 解決で
  利用）を `code` ドメインへ再配置し、公開型を保持する。
- メンション参照の解決エントリ（mention 解決またはフォールバック）は、候補列挙
  （`list_mentionable_files`）と同じく `MentionRepository` 抽象と usecase（Query）を経由する
  単一経路へ統一する。gateway 実装関数を外部から直接呼ぶ利用形は廃し、外部呼び出し側
  （agent / backends / workflow / session）は usecase が公開する解決 API を経由する。この
  呼び出し形の変更は許容するが、解決結果・フォールバック挙動・エラー表現は等価に保つ。
- MCP サーバ（`read_file`）と agent bridge が、ファイル内容参照（at_ref）・base branch 名解決・
  `code` ドメインのエラー型を gateway 実装関数の直呼びで利用している。これらを mention 解決と
  同じく usecase（`CodeUsecase` / Query）が公開する API 経由へ統一し、周辺入口が gateway 実装
  モジュールへ直接依存しない構造にする。呼び出し形の変更は許容するが、結果・エラー表現は等価に保つ。

## Data / Communication Flow

境界をまたぐ主要 flow（いずれも移行前後で結果等価）:

- **ファイル内容参照 / diff 閲覧（Query）**: 入口（Tauri コマンド / リモートハンドラ / MCP サーバ）→
  usecase（Query）→ domain 抽象 → gateway（git2 / ファイル I/O）→ ドメイン型／DTO を返す。
- **差分 Approve（Command）**: 入口 → usecase（Command）→ domain 抽象 → gateway（git2
  index 操作）。複数の前後関係を要する手順がある場合の順序制御は usecase が持ち、gateway は
  単一集約の I/O プリミティブに留める。
- **hunk / patch / range / language（純粋算出）**: 入口 → usecase → domain サービス（純粋
  関数）。外部リソースに触れない。
- **file_mention 解決（他エントリ経由）**: agent / backends / workflow / session →
  usecase（mention 解決 Query）→ `code` の mention 解決契約（`MentionRepository`）→
  候補列挙・参照解決。

## State Ownership

- **作業ツリー・index・オブジェクトの状態**: Git リポジトリが唯一の owner。`code` ドメインは
  これを読み取り（内容参照・diff）または変更（staging）する操作を提供するのみで、状態の
  複製・キャッシュを新たに持たない。
- **read model（DTO）**: usecase 層が所有・定義する（表示／転送のための形）。
- **DI で組み立てた usecase インスタンス**: composition root（controller）が組み立て、
  `AppState` ほか各エントリの State へ注入する形で owner を一元化する。QueryService 等の
  読み取り部品は usecase 内部に閉じ込め、外部へ直接配らない。
- 本移行で新規の永続状態・グローバル状態は導入しない。

## Boundaries

越えてはいけない責務境界（依存方向: `infrastructure → adaptor → usecase → domain`、
内向きのみ）:

- domain → usecase / adaptor / infrastructure への依存（逆依存）を作らない。domain は
  git2・tokio・tauri・std I/O を直接参照しない。
- usecase → adaptor / infrastructure への依存を作らない。usecase は domain の trait のみに
  依存する。
- controller / handler はビジネスロジックを持たない。usecase を呼ぶだけにする
  （QueryService / gateway を controller から直呼びしない）。
- gateway に業務手順（複数集約をまたぐオーケストレーション・順序制御）を潰し込まない。
- `repository` ドメインのファイル・責務へ影響を及ぼさない。共有型・util の変更は
  `repository` の規約準拠を崩さない範囲に限定する。
- 同一責務（例: ファイル内容参照・diff 算出）を複数箇所に重複実装しない。移行後に旧構成
  （`git/diff.rs`・`git/diff_tree.rs`・`git/branch_diff.rs`・`git/hunk.rs`・`git/lang.rs`・
  `git/stage.rs`・`file_mention.rs`、および `git/commands.rs` の `code` 責務部分）を除去し、
  重複を残さない。
- 外部 I/F（コマンド引数・戻り値・エラー表現、WebSocket メッセージ、補完結果）を変えない。

## Implementation Freedom

以下は実装に委ねる:

- domain のエンティティ・値オブジェクト・ドメインサービスの具体的な分割と命名。
- 各責務で CQRS（Command/Query サービス分離）を採るか統合するかの判断、および trait 化の
  要否（単一実装は構造体直持ちでよい）。
- gateway / infrastructure 内の git2・ファイル I/O 呼び出しの具体実装、ドメイン型 ↔ 外部型
  の変換方法。
- 内部 helper 関数・内部型・関数シグネチャ。
- ブロッキング呼び出しを非同期境界へ載せる方式の詳細（`repository` と整合する範囲で）。
- domain / usecase / gateway 各層のテストの構成（規約 TEST.md に従う）。
- `code` に属する共有 util／型を `git/` から整理する際の配置先の詳細。
- security 上の取り扱い（入力検証・パス取り扱い等）の実装方法。
