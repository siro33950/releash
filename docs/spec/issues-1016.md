# issues-1016: ContractFacet 化と builtin instruction の DRY 違反解消

## 要求

**種別**: リファクタリング

**ゴール**:
`*-from-task.md` 系 11 ファイル（`implement-from-task.md` / `implement-fix-from-task.md` /
`implementation-approval-from-task.md` / `implementation-fix-policy-from-task.md` /
`fix-policy-auto-from-task.md` / `review-acceptance-from-task.md` /
`review-structure-from-task.md` / `review-quality-from-task.md` /
`review-test-from-task.md` / `review-security-from-task.md` /
`review-architecture-from-task.md`）の本文重複（約 500 行）を解消する。
合わせて `OutputContract` facet を `Contract` facet に格上げし、
step 境界における input/output データ仕様を Contract 1 概念で双方向に表現可能にする。

**背景**:
`feat/builtin-workflow` ブランチで `spec-implement` / `spec-review` /
`spec-review-auto` / `bug-fix` の 4 つの builtin workflow を追加した際、
それぞれが `task` 引数経由で Spec ファイルパスを受け取る前提のため、
対応する instruction を `*-from-task.md` として 11 ファイル新規追加した。
しかし対応する非 `-from-task` 版（`spec-driven-development.yml` 用）との差分は
「Spec パスをどこから取得するか」を説明する数行のみで、本文の 95% 以上が完全に二重管理状態。
`/review` skill による包括的レビュー（指摘 R2-01, R2-02, R3-01）でも構造・品質モジュールが
共通 FAIL 基準 #5「DRY 違反」に該当して FAIL 判定。
根本原因は「data-flow 情報（task / pass_output_from のどちらから読むか）が
prompt 本体（instruction）に漏れている」設計問題であり、
この機会に Contract 体系を双方向化することで構造的に解消する。

**対象ユーザー**:
Releash の builtin workflow / instruction facet を保守・拡張する開発者。
特に task 引数を使う新規 builtin workflow を追加する場合、
入力経路ごとに instruction を複製する必要がなくなる。

**制約**:
- 既存の workflow primitives（`pass_output_from`、`{{task}}` / `{{previous_response}}` /
  `{{project_name}}` テンプレ変数機構）と整合させる
- 既存 builtin workflow（特に `spec-driven-development`）の LLM への入力プロンプトが
  実質的に同等であること（リファクタ前後で agent の振る舞いが変わらない）
- 信頼境界（既存方針の明文化）:
  - 信頼済み: アプリに同梱される builtin の workflow YAML / facet 本文。
  - 信頼境界外: user-authored facet（user 領域のファイル）、フロントエンド経由の facet 編集入力、
    workflow の実行時入力（`task` 等）、上流 step からの agent 応答。
  - Contract による参照妥当性検査は、上記の境界外入力に対して行う（builtin 同梱物はビルド時に整合する前提）。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` /
  `pnpm test` / `pnpm lint` の全パス

**影響範囲**:
- `src-tauri/src/workflow/` の `facet.rs` / `schema.rs` / `engine.rs` /
  `validation.rs` / `storage.rs` / `builtin.rs`
- `src-tauri/src/workflow/builtin_facets/output_contracts/` →
  `src-tauri/src/workflow/builtin_facets/contracts/` ディレクトリリネーム
- `src-tauri/src/workflow/builtin/*.yml`（既存 6 ファイル全て）
- `src-tauri/src/workflow/builtin_facets/instructions/*-from-task.md` 11 ファイル削除
- 既存の OutputContract を扱うフロントエンド CRUD UI があれば rename 追従
  （新規 input_contracts 編集 UI 追加は別タスク）
- 関連テスト全般

## 振る舞い定義

```gherkin
Feature: builtin workflow の振る舞い保全
  リファクタリング前から提供されている builtin workflow の利用者から見た振る舞いを保全する。

  Rule: 既存 builtin workflow の利用者から見た振る舞いは変わらない
    Scenario: 既存 builtin workflow を実行する
      Given リファクタリング前から提供されている builtin workflow が存在する
      When 利用者がその workflow を従来と同じ条件で実行する
      Then agent に渡される指示の意味は従来と同等であり、得られる成果も従来と同等である
```

## アーキテクチャ概要

### 責務配置

- **`workflow/facet.rs`（facet 体系の owner）**:
  - 担当する: facet 種別 enum、Contract（旧 OutputContract）の格上げ、key validation、facet ファイル I/O（読み書き）の単一窓口、prompt 合成（`compose_facets`）。
  - 担当しない: step 間データ受け渡し経路の選択（engine の責務）、instruction 本文の解釈。
- **`workflow/schema.rs`（step 境界仕様の owner）**:
  - 担当する: step（`NodeDefinition` / `ChildNodeDefinition`）が input 側 / output 側で Contract 参照を持つ表現、`ResolvedFacets` への解決済み本文格納、双方向 Contract の概念定義。
  - 担当しない: 値の引き渡し処理、ファイル I/O。
- **`workflow/builtin.rs`（builtin 資産の registry）**:
  - 担当する: builtin workflow YAML / facet の登録と lookup、kind 別件数の不変条件、入力経路非依存に再編した instruction 一覧の宣言。
  - 担当しない: instruction 本文の中身、step 間のデータ移送方法。
- **`workflow/builtin/*.yml`（step 配線の owner）**:
  - 担当する: instruction 参照、input/output Contract 参照、`pass_output_from` / `task` 等の入力経路宣言。
  - 担当しない: instruction 本文の重複保持。
- **`workflow/builtin_facets/contracts/`（旧 `output_contracts/` をリネーム）**:
  - 担当する: step 境界で受け渡されるデータ仕様の定義本体。input 側・output 側のいずれからも同一定義を参照可能な配置にする。
  - 担当しない: 入力経路の表現。
- **`workflow/builtin_facets/instructions/`（手順の owner）**:
  - 担当する: ビジネス手順としての指示本文。
  - 担当しない: 「task からか pass_output_from からか」といった経路情報の保持（DRY 違反の根本原因なので明示的に排除する）。
- **`workflow/engine.rs`（実行時の経路解決の owner）**:
  - 担当する: step の input Contract 宣言と入力経路（`{{task}}` / `{{previous_response}}` / `pass_output_from`）の対応付け、テンプレ変数展開、合成済み prompt の agent 投入。
  - 担当しない: 経路情報を instruction 本文へ漏らすこと。
- **`workflow/validation.rs`**:
  - 担当する: Contract 双方向化に伴う参照妥当性検査（input/output 側それぞれの参照、type 別の許容、解決前後の不変条件）。
  - 担当しない: facet 本文の意味検査。
- **`workflow/storage.rs`**:
  - 担当する: facets ディレクトリ構造の I/O。`contracts/` リネームへの追従。
- **フロントエンド（CRUD UI）**:
  - 担当する: OutputContract → Contract のリネーム追従（表示・呼出し名のみ）。
  - 担当しない: input 側 Contract の新規編集 UI（別タスク）、ロジック保持。

### データ/通信フロー

- **workflow の load**: YAML 読込 → `facet::resolve_workflow_facets`（Contract / instruction / policy / knowledge の本文を `ResolvedFacets` に格納）→ workflow 構造を engine へ返却。`validation::validate`（Contract 双方向参照を含む構造検査・参照妥当性検査）は信頼境界外入力（user-authored facet、フロントエンド経由の編集入力、user-authored workflow / builtin override、実行時入力）に対して適用する。builtin 同梱の workflow / facet はビルド時に整合する前提のためランタイム検証を省略してよい（テストにおける構造整合チェックは別途確保する）。
- **step 実行時の prompt 合成**: Contract 本文は純粋なデータ shape 仕様（フォーマット + フィールドルール）のみを保持し、「レスポンスに含めること」「前段出力 or task から抽出すること」等の方向別説明文は持たない。これらの方向別定型文は engine が input / output いずれの用途で合成するかに応じて前置として自動付与する。`compose_facets` は system_prompt = policy + (engine 前置 + output_contract)、user_message = knowledge + (engine 前置 + input_contracts 連結) + instruction の構造で合成 → engine が step 定義から入力経路を解決して値を取り出し（`task` / `previous_response` / `pass_output_from`）→ テンプレ変数展開 → agent 呼び出し。この構造により同一 Contract 本文が input / output 双方で再利用される（双方向対称性のデータフロー側の実現）。
- **step 出力の引き渡し**: agent 応答 → output Contract に従って後段 step が参照可能な形で保持 → 後段 step は input Contract 宣言を通じて同一概念で参照。
- **facet listing / 編集**: フロントエンド → Tauri command → `facet.rs` の list/load/save/delete → user 領域と builtin 領域をマージして返却。

### 状態Owner

- **facet 種別の集合定義**: `FacetKind` enum（`facet.rs`）。Contract 双方向化に伴いここが起点。
- **step ごとの Contract / instruction / policy / knowledge 参照キー**: `NodeDefinition` / `ChildNodeDefinition`（`schema.rs`）。YAML から deserialize される未解決参照を保持。
- **解決済み facet 本文**: `ResolvedFacets`（`schema.rs`）。load 経路でのみ populate され、`#[serde(skip)]` により永続化対象外。
- **builtin facet 本文と一覧**: `BUILTIN_FACETS`（`builtin.rs`）。コンパイル時 `include_str!` で同梱。
- **builtin workflow YAML 本文と description メタデータ**: `BUILTINS`（`builtin.rs`）。
- **user 領域 facet ファイル**: `facets_base_dir()` 配下のファイルシステム（`storage.rs` 経由）。
- **step 間の中間値（previous_response / step_outputs）**: 実行ランタイム側（engine の run state）。schema / facet は所有しない。

### 境界

- **instruction ↔ 入力経路の分離**: instruction 本文は入力経路を一切知らない。`task` か `pass_output_from` か `previous_response` かの判別ロジックは step 定義と engine に閉じる。この境界が破れることが DRY 違反の根本原因なので、本リファクタリングの最重要不変条件として明示する。
- **Contract の双方向対称性**: input 側 / output 側の Contract 参照は同一の facet 本文を共有する。「同じ概念を入口と出口で表す」ことを型レベルで保証する。
- **load ↔ 実行の分離**: facet 参照の解決は load 経路に閉じる。実行時に未解決 ref が schema 層に残ってはならない（`ResolvedFacets` に必ず格納される）。
- **builtin ↔ user-authored の経路統一**: 同一の load パイプラインを通り、user 側 override が builtin より優先される（既存方針を踏襲）。
- **Rust ↔ フロントエンド**: ロジックは Rust 側に閉じる。UI のリネームは表示と呼出しのみ。

### 実装に委ねること

- `FacetKind` の rename 方針（`OutputContract` → `Contract` への直接変更か、双方向を表す新 variant 構成か）。
- schema フィールド名の選定（`input_contract` / `output_contract` の併存、または単一 `contract` フィールドに方向属性を持たせる等）。
- 既存 `-from-task.md` 系 11 ファイルの本文を非 `-from-task` 版に統合する具体的手法（共通本文を base にしてどう差分を吸収するか）。
- Contract 参照の YAML 表現の細部記法。
- `validation.rs` における方向違反・参照不整合の検出ルール詳細。
- 互換シム（旧キー名の一時許容、deprecation warning など）の要否と粒度。
- フロントエンドの rename 反映範囲の特定（grep ベースで進める）。
- 各レイヤーのテスト追加位置・命名・件数（既存テスト規約に従う）。
- helper 関数の命名・内部データ変換の構造・細かい型分割。

