## 要求

**種別**: リファクタリング

**ゴール**: ワークフローのファセット種別から `persona` を完全に廃止し、4種（`policy` / `knowledge` / `instruction` / `output_contract`）に削減する。ファセット合成では、Persona に代えて `policy` と `output_contract` を `system_prompt` のソースとして扱う方針に切り替える。完了時点で、Rust・フロントエンドの production コード／型定義／ビルトインファセット定義のいずれにも `persona` / `Persona` への参照が残らない状態にする。ユーザーディレクトリ上の `personas/` 配下の物理ファイルは削除せず、UI・コマンド経路から参照不能とする。

**背景**:

- 学術研究（EMNLP 2024、USC 2026）により、「あなたはシニアエンジニアです」的な性格づけペルソナはコーディング・知識タスクで効果なし〜逆効果と判明している。
- 効果があるのはタスクスコープの限定と行動制約の付与であり、これは `policy` ファセットでカバーできる。
- ビルトインペルソナファイル（planner.md / coder.md / reviewer.md）は #898 ですでに削除済み。
- 現在ファセット種別は5種（`persona`, `policy`, `knowledge`, `instruction`, `output_contract`）あるが、`persona` を残しておく根拠が失われているため、種別自体を廃止して4種に削減する。

### 廃止後のファセット合成方針

Persona 廃止後の `system_prompt` のソース不在を、以下のように再設計する。

- **`system_prompt`**: `policy` + `output_contract`（いずれもターン非依存な常設情報のため）
- **`user_message`**: `knowledge` + `instruction`（参照知識とそのターンのタスク手順）

`ComposedPrompt.system_prompt` フィールドおよび `AgentSession` への `system_prompt` 受け渡し経路は維持する。

### 後方互換の方針

- **既存ユーザー定義ワークフロー YAML に `persona: ...` 行が残っていた場合**: 現状 `Workflow` / `Step` / `ParallelStep` に `#[serde(deny_unknown_fields)]` は付与されていないため、`Step.persona` フィールド削除後は serde のデフォルト挙動により当該行は黙って無視される。この挙動は serde のデフォルトに委ね、明示テスト（後方互換テスト）は追加しない。明示的なエラー化・警告ハンドリングも追加しない。
- **persona のみで成立していた既存ステップ**: ファセット参照も `inline_prompt` も持たず `persona` のみで成立していたステップは、`Step.persona` 削除後は `validation::validate` の既存ロジックにより参照不在エラーで失敗する。これは本タスクの非スコープとし、マイグレーションや救済処理は実装しない。
- **ユーザーが個別に作成した `personas/` ディレクトリ・`*.md` ファイル**: ディスク上はそのまま放置する。Rust 側から `FacetKind::Persona` が消えることでコードからは参照されなくなり、UI・コマンド経由でも見えなくなる。物理ファイルの自動削除や警告ログは行わない。

### 対象範囲（廃止対象）

- `FacetKind::Persona` enum variant（`facet.rs`）と関連する `parse_facet_kind("persona")` 等のマッピング
- `Step.persona` / `ParallelStep.persona` フィールド（`schema.rs`）
- `has_any_facet_ref` / `has_facet_refs` の `persona` 引数（`schema.rs`）
- `compose_facets_from_refs` / `compose_facets` の `persona` 引数と `system_prompt = persona` のロード処理（`facet.rs`）
- `engine.rs` 内でステッププロンプトを構築する経路の `persona` 参照（`build_step_prompt` / `build_parallel_step_prompt` 等）
- `validation.rs` / `storage.rs` / `log.rs` / `diagnostics.rs` 内のステップ生成・ファセット参照リスト構築での `persona`
- `session/mod.rs` のステップ生成箇所での `persona`
- フロントエンドの `src/types/workflow.ts` の `persona?: string` フィールド・`"persona"` リテラル
- フロントエンドの `src/components/panels/automation/utils.ts` の `persona: "personas"` マッピング
- フロントエンドの `src/components/panels/automation/WorkflowDetail.tsx` の persona 表示行
- ビルトイン関連: `builtin.rs` 内の `FacetKind::Persona` 参照と関連テスト。`src-tauri/src/workflow/builtin_facets/` 配下に personas 系ビルトイン定義ファイルが存在しない状態を維持する
- 関連テスト（`facet.rs`, `engine.rs`, `schema.rs`, `validation.rs`, `storage.rs`, `log.rs`, `diagnostics.rs`, `commands.rs`, `session/mod.rs` 内のテスト）

### 非スコープ

- ビルトインワークフロー YAML（`spec-driven-development.yml`）の再設計（既に persona 未使用のため変更不要）
- `policy` / `output_contract` 自体の中身の再設計
- ユーザー定義 `personas/` ディレクトリのマイグレーション機能の追加
- `deny_unknown_fields` の導入や、削除済みフィールドへの明示的なエラー／警告メッセージの追加

## 振る舞い定義

```gherkin
Feature: ワークフローのファセット種別からPersonaを廃止し4種に削減する
  ワークフロー実行時のプロンプト合成元から Persona を取り除き、
  policy / knowledge / instruction / output_contract の4種で
  system_prompt / user_message を再構成する。

  Rule: ワークフローエンジンはステップ宣言から system_prompt と user_message を合成する
    Scenario: policyとoutput_contractの両方を指定したステップから system_prompt が合成される
      Given ワークフローのステップに policy と output_contract が指定されている
      When ワークフローエンジンがステップのプロンプトを合成する
      Then system_prompt には policy と output_contract の本文が含まれる

    Scenario: policyのみを指定したステップでも system_prompt が合成される
      Given ワークフローのステップに policy のみが指定され output_contract は指定されていない
      When ワークフローエンジンがステップのプロンプトを合成する
      Then system_prompt には policy の本文が含まれる

    Scenario: output_contractのみを指定したステップでも system_prompt が合成される
      Given ワークフローのステップに output_contract のみが指定され policy は指定されていない
      When ワークフローエンジンがステップのプロンプトを合成する
      Then system_prompt には output_contract の本文が含まれる

    Scenario: policy も output_contract も指定がないと system_prompt は設定されない
      Given ワークフローのステップに policy も output_contract も指定されていない
      When ワークフローエンジンがステップのプロンプトを合成する
      Then system_prompt は設定されない

    Scenario: knowledgeとinstructionを指定したステップから user_message が合成される
      Given ワークフローのステップに knowledge と instruction が指定されている
      When ワークフローエンジンがステップのプロンプトを合成する
      Then user_message には knowledge と instruction の本文が含まれる

    Scenario: knowledge も instruction も指定がないと user_message は空文字として合成される
      Given ワークフローのステップに knowledge も instruction も指定されていない
      When ワークフローエンジンがステップのプロンプトを合成する
      Then ComposedPrompt.user_message は空文字列となる

    Scenario: 並列ステップの子ステップでも同じ合成ルールが適用される
      Given 並列ステップの子ステップに policy / output_contract / knowledge / instruction が指定されている
      When ワークフローエンジンが並列子ステップのプロンプトを合成する
      Then 子ステップでも policy と output_contract が system_prompt に集約され knowledge と instruction が user_message に集約される

    Scenario: 参照先ファセットが存在しないステップはプロンプト合成時に NotFound 相当のエラーで失敗する
      Given ワークフローのステップに存在しないファセットキーが policy / knowledge / instruction / output_contract のいずれかとして参照されている
      When ワークフローエンジンがそのステップのプロンプトを合成する
      Then compose_facets の load_facet 経路で NotFound 相当のエラーとなり サイレントに成功することはない

    Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
      Given ワークフローのステップに policy と output_contract が参照として指定されている
      And バックエンド起動経路（`start_agent_session_internal` 相当）はテストダブルで置換され受け取った引数を記録する
      When ワークフローエンジンがそのステップを実行し AgentSession を開始する
      Then policy と output_contract 由来で合成された `system_prompt` が AgentSession 開始引数の `system_prompt` としてバックエンドに渡される
      And `system_prompt` がドロップ・空文字置換されずに受け渡される

  Rule: ファセット種別は policy / knowledge / instruction / output_contract の4種に限定される
    Scenario: 列挙した各 Tauri コマンドは 4種それぞれの種別指定で正常経路に到達する
      Given `parse_facet_kind` を経由する Tauri コマンド `list_facets` / `get_facet` / `save_facet` / `delete_facet` / `list_facet_summaries` / `duplicate_facet` / `open_facet_in_editor` のそれぞれ
      And `list_facets` / `list_facet_summaries` 用にベースディレクトリが存在する
      And `get_facet` / `delete_facet` / `duplicate_facet` / `open_facet_in_editor` 用に 4種それぞれのディレクトリ配下に既存の非ビルトインファセット `key` に対応する `.md` ファイルが存在する
      And `save_facet` 用に保存対象 `key` と `content` 文字列が与えられている
      And `duplicate_facet` 用に既存 `key` と未使用の `new_key` が与えられている
      And `open_facet_in_editor` 用に外部エディタ呼び出しはテストダブルで置換され実プロセスを起動しない
      When 種別として "policy" / "knowledge" / "instruction" / "output_contract" のそれぞれを指定して呼び出す
      Then 列挙した各コマンド × 4種それぞれの組み合わせで 未知種別エラーではなく各コマンド固有の正常経路に到達する
      And `list_facets` / `list_facet_summaries` は当該種別ディレクトリの一覧結果を返す
      And `get_facet` は対象 `.md` の本文を返す
      And `save_facet` は対象パスに `content` を書き込む
      And `delete_facet` は対象 `.md` を削除する
      And `duplicate_facet` は `new_key` に対応する `.md` を生成する
      And `open_facet_in_editor` はテストダブルのエディタ呼び出しに対象パスを引き渡す

    Scenario: persona または未知種別を指定した Tauri コマンドは拒否される
      Given `parse_facet_kind` を経由する Tauri コマンド `list_facets` / `get_facet` / `save_facet` / `delete_facet` / `list_facet_summaries` / `duplicate_facet` / `open_facet_in_editor` のそれぞれ
      When 種別として "persona" もしくは未知の種別文字列を指定して呼び出す
      Then 列挙した各コマンドは未知種別エラーを返し personas/*.md を含むファセットファイルの読み書きを一切行わない

  Rule: ワークフロー詳細画面ではファセット参照として Persona を表示しない
    Scenario: ステップに紐づくファセット参照行から Persona が消える
      Given あるステップに policy / knowledge / instruction / output_contract がすべて設定されている
      When ユーザーがワークフロー詳細画面を開く
      Then ステップ表示には policy / knowledge / instruction / output_contract の 4 行が表示され Persona 行は表示されない

  Rule: production コード・型定義・ビルトイン定義に persona / Persona 参照が残らない
    Scenario: Rust 側 production コードに persona / Persona の参照が残っていない
      Given `src-tauri/src/` 配下全体のリファクタリングが完了している
      When `src-tauri/src/**/*.rs` の production コード（`#[cfg(test)]` 配下のテストコードおよびビルド生成物を除く）を以下の対象に限定して検索する: `FacetKind::Persona` / 文字列リテラル `"persona"` および `"personas"` / 単語境界での `\bPersona\b` / フィールド名や識別子としての `persona`（単語境界付き `\bpersona\b`）
      Then マッチが存在しない
      And `personal` 等の別語の部分一致は対象外とする

    Scenario: フロントエンド側 production コードに persona / Persona の参照が残っていない
      Given `src/types/workflow.ts` および `src/components/panels/automation/` のリファクタリングが完了している
      When 対象パス配下の production コード（`*.test.ts` / `*.test.tsx` を除く）を `persona` / `Persona` / `"persona"` で検索する
      Then マッチが存在しない

    Scenario: ビルトインファセット定義に persona 系の定義が存在しない
      Given `src-tauri/src/workflow/builtin.rs` および `src-tauri/src/workflow/builtin_facets/` 配下のリファクタリングが完了している
      When ビルトインファセットのキー一覧および定義ファイル一覧を確認する
      Then `FacetKind::Persona` を参照する箇所と personas 系ビルトイン定義ファイルが存在しない

  Rule: Persona廃止後もユーザーディレクトリ上の物理ファイルは保持される
    Scenario: 既存のpersonasディレクトリのファイルはディスク上に残るがアプリからは参照されない
      Given ユーザーのワークフローディレクトリに `personas/*.md` ファイルが存在する
      When ユーザーがアプリを起動してファセット一覧を表示する
      Then `personas/*.md` ファイルはディスク上に残ったままであり ファセット一覧には現れない
```

## アーキテクチャ概要

### 責務配置

- **`src-tauri/src/workflow/facet.rs`（ファセット定義の中核）**
  - 担当する: `FacetKind` 列挙の定義、種別→ディレクトリ名マッピング、`ComposedPrompt`（`system_prompt` / `user_message`）の合成、`compose_facets_from_refs` / `compose_facets` での種別ごとの本文ロードと `system_prompt` / `user_message` への振り分け、ファセットファイルの CRUD と一覧取得。
  - 担当しない: ステップやワークフロー全体の構造解釈、ステップ→ファセット参照の取得元（呼び出し側で `Step` から渡す）、UI 表示、AgentSession への引き渡し。

- **`src-tauri/src/workflow/schema.rs`（型レベル契約）**
  - 担当する: `Step` / `ParallelStep` のファセット参照フィールド定義、`has_facet_refs` / `has_any_facet_ref` の存在判定、serde を介した YAML ↔ 構造体の対応。
  - 担当しない: ファセット本文のロード、合成、ステップ実行ロジック、永続化先パスの解決。

- **`src-tauri/src/workflow/engine.rs`（ステップ実行とプロンプト組み立て）**
  - 担当する: 通常ステップ／並列ステップそれぞれで `compose_facets` を呼び出して `(system_prompt, user_message)` を取り出す経路、テンプレート変数展開、AgentSession への `system_prompt` の引き渡し。
  - 担当しない: ファセット種別ごとの「どちらに振り分けるか」のルール決定（`facet.rs` の責務）。

- **`src-tauri/src/workflow/validation.rs` / `storage.rs` / `log.rs` / `diagnostics.rs`**
  - 担当する: ワークフロー定義の検証・読み書き・ログ出力・診断における、ファセット参照の種別ごとの取り扱い（種別一覧の列挙・参照存在チェック・使用元集計）。
  - 担当しない: 合成ロジック、UI 表示、AgentSession 連携。

- **`src-tauri/src/workflow/commands.rs`（Tauri コマンド面）**
  - 担当する: フロントエンドから渡される文字列としての種別名（`"policy" | "knowledge" | "instruction" | "output_contract"`）を `FacetKind` に解決する `parse_facet_kind`、未知種別の拒否。
  - 担当しない: 種別ごとの本文加工、UI 文言。

- **`src-tauri/src/workflow/builtin.rs`（ビルトインファセット）**
  - 担当する: 種別ごとのビルトインキー一覧と本文の供給、ビルトイン保護判定。
  - 担当しない: ユーザーディレクトリへの書き込み、UI 表示。

- **`src-tauri/src/session/mod.rs`（ステップ構造体の生成）**
  - 担当する: ワークフロー外部から `Step` 構造体を生成／コピーする経路で、Persona 廃止後の新フィールド集合に整合した値を渡す。
  - 担当しない: ファセット合成、種別解決、`system_prompt` のバックエンドへの受け渡し。

- **`src-tauri/src/backends/bridge_common.rs`（バックエンド起動への受け渡し層）**
  - 担当する: 上位から受け取った `system_prompt` をバックエンド起動コマンドに受け渡す。
  - 担当しない: ファセット合成や種別解決、`Step` 構造体の生成。

- **`src/types/workflow.ts`（フロント側型）**
  - 担当する: `Step` / `ParallelStep` のフィールド型、`FacetKind` リテラル型の宣言を、廃止後の4種に揃える。
  - 担当しない: ロジック実装。

- **`src/components/panels/automation/utils.ts`**
  - 担当する: フロントから扱う種別キー → ディレクトリ名マッピング（Rust の `FacetKind::dir_name()` と同期）。
  - 担当しない: ファセット本文取得や合成。

- **`src/components/panels/automation/WorkflowDetail.tsx`**
  - 担当する: ステップ詳細でのファセット参照行（4種）の表示。
  - 担当しない: 値の編集や種別変換。

### データ/通信フロー

- **ワークフロー実行時のプロンプト合成**:
  `engine.rs` がステップを取り出す → `facet::compose_facets(&step, base_dir)` を呼ぶ → `compose_facets_from_refs` が `policy` / `output_contract` を `system_prompt` に、`knowledge` / `instruction` を `user_message` に集約 → `ComposedPrompt` を返却 → engine がテンプレート変数を展開 → `start_agent_session_internal` に `system_prompt` を渡して AgentSession を開始 → `start_agent_turn_internal` に `user_message` 由来の `prompt` を渡してターンを実行する。

- **ファセット CRUD（Tauri コマンド経由）**:
  フロント UI → `invoke(...)` → `commands.rs` の `parse_facet_kind` が文字列を `FacetKind` に解決（未知種別はここで拒否） → `facet.rs` の `load_facet` / `save_facet` / `delete_facet` / `list_facets` がファイル I/O を実施。

- **ワークフロー YAML 読み込み**:
  `storage.rs` が YAML を読み込み → serde が `Step` / `ParallelStep` にデシリアライズ（未知フィールドは黙って無視される）→ `Step.persona` が型から消える結果、既存 YAML 中の `persona:` 行は自動的に捨てられる。

- **診断・使用元集計**:
  `diagnostics.rs` の `ALL_FACET_KINDS` 配列と `FacetRefs<'a>` 構造体が走査対象種別を列挙 → ステップ／並列子ステップを走査して種別ごとに参照存在チェックと使用元記録を行う。

- **ワークフロー詳細表示**:
  Tauri から取得した `Workflow` を `WorkflowDetail.tsx` が描画 → 種別ごとの `FacetRefRow` を 4 種ぶん表示。

### 状態Owner

- **ファセット種別の列挙（`FacetKind`）**: `src-tauri/src/workflow/facet.rs`（唯一の真実の源）。
- **種別→ディレクトリ名マッピング**: Rust 側は `FacetKind::dir_name()`、フロント側は `automation/utils.ts` の `FACET_KIND_DIR_MAP`（Rust と同期する従属コピー）。
- **ステップ→ファセット参照（`policy` / `knowledge` / `instruction` / `output_contract`）の構造**: `src-tauri/src/workflow/schema.rs` の `Step` / `ParallelStep`。
- **ファセット本文の永続化先（`{base}/{dir_name}/{key}.md`）**: ファイルシステム（`storage.rs` がベースディレクトリ解決を、`facet.rs` が個別パス組み立てを担当）。
- **ビルトインファセットの定義**: `src-tauri/src/workflow/builtin.rs`。
- **プロンプト合成結果（`ComposedPrompt`）**: `facet.rs` で生成され、`engine.rs` を経て `AgentSession` に短命に引き渡される（永続状態を持たない）。
- **UI 上のファセット参照表示順・ラベル**: `WorkflowDetail.tsx`。

### 境界

- **Rust 側 `FacetKind` を真実とし、フロントは「文字列リテラル＋同期コピー」しか持たない**。種別の追加・削除はまず Rust 側で行い、フロントの型・マッピングを後追いで合わせる。
- **`compose_facets_from_refs` 以外の場所で種別→`system_prompt` / `user_message` の振り分けを行わない**。engine やコマンド層は `ComposedPrompt` の構造に従う。
- **`commands.rs` 以外の層で生の種別文字列（`"persona"` 等）を `FacetKind` に変換しない**。文字列はフロント境界でしか登場させない。
- **`schema.rs` の `Step` / `ParallelStep` がファセット参照フィールドの単一定義**。`diagnostics.rs` の `FacetRefs<'a>` 等の補助構造はここから派生し、フィールド集合を独自拡張しない。
- **YAML 後方互換は serde のデフォルト挙動に任せる**。`deny_unknown_fields` の付与や独自警告ロジックは導入しない。
- **物理ファイル（ユーザーの `personas/*.md`）に Rust から触れない**。読み書き経路から `FacetKind::Persona` を除去することで自然に到達不能化する。

### 実装に委ねること

- ヘルパー関数名・内部リファクタリングの粒度（`has_any_facet_ref` の引数並び替え、`FacetRefs` のフィールド順 等）。
- `compose_facets_from_refs` における `policy` / `output_contract` の連結順序や区切り文字（観測可能な振る舞いは「両方の本文が含まれる」のみ）。
- 既存テスト内で `persona: None` を埋めていた箇所の整理方法（フィールドごと削除する／構造体リテラルを書き換える 等）。
- フロント側 `FacetRefRow` の表示順（ただし Persona 行が存在しないという制約は守る）。
- `parse_facet_kind` のエラーメッセージ文言（拒否されることだけが要件）。
- テストケースの配置先モジュールと命名（合成 / 種別解決 / UI 表示 / 物理ファイル保持の各シナリオ群を満たすこと）。
