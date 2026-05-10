## 要求

**種別**: 新機能 + 改善
**ゴール**: Settings上にWorkflow / Facetの管理画面（Automation）を追加し、診断・閲覧・編集をYAMLやMarkdownを直接触らずにUI上で完結できるようにする。ビルトインは常に最新版に上書きし、カスタマイズは複製で対応する方針に統一する
**背景**:
1. ワークフローとファセットはYAML/Markdownで管理されるが、設定画面上で構造や参照関係・診断状態を把握する手段がない
2. ビルトイン初期化処理がファイル存在チェックでスキップする設計のため、アプリ更新でビルトインが改善されても既存ユーザーに反映されない
3. ファセットやワークフローの編集には外部エディタが必要で、typoや参照切れを事前に防ぐ仕組みがない

**関連Issue**: #900, #901, #902, #903

### スコープ

#### 1. 診断API（#900）

- workflow YAML の parse / schema / 参照整合性チェック
- ファセット Markdown の parse / テンプレート変数（`{{task}}`等）チェック
- workflow step からファセットへの参照確認（存在しないファセットキーの検出）
- severity を `error` / `warning` / `info` で返す
- 診断対象の workflow 名、step 名、ファセット名、フィールド情報を返す
- workflow / ファセット一覧で診断状態を表示できるサマリを返す
- builtin / custom の両方を診断対象にする
- workflow / ファセットディレクトリをファイル監視し、外部エディタでの変更検出時に診断を自動再実行する

診断例:
- Error: YAML構文が壊れている
- Error: 存在しない step を `rules.next` / `collect.from` / `pass_output_from` が参照している
- Error: 存在しないファセットを workflow step が参照している
- Warning: `any_needs_fix` / `all_passed` の collect 元 step に rules がない
- Warning: 到達不能な step がある
- Info: builtin workflow / builtin ファセット

#### 2. ビルトイン管理（#901）

- ビルトインのワークフロー/ファセットは `include_str!()` でバイナリに埋め込まれており、アプリ更新で常に最新版が提供される（ディスク書き出し不要）
- Tauriコマンド層でビルトインの編集・削除を拒否するガードを追加

#### 3. Automation管理画面（#901）

- Settings内のWorkflows領域をAutomation管理画面として拡張
- `Workflows` / `Facets` タブを追加
- Facets タブ内に種別サブタブ（Policy / Knowledge / Instruction / OutputContract）を設ける
- 左ペインに一覧、右ペインに詳細を表示
- workflow一覧に名前、説明、builtin/custom、診断状態を表示
- ファセット一覧に名前、説明、builtin/custom、使用状況（参照元 workflow 件数）を表示（種別サブタブで絞り込み）
- workflow詳細に steps / mode / ファセット参照 / rules / collect / reduce / pass_previous_response / pass_output_from / cycle_guard / parallel（aggregate含む）を表示
- ファセット詳細に本文プレビュー / variables / Used by を表示
- 診断APIの error / warning / info を一覧と詳細で表示

#### 4. ファセット編集（#902）

- custom ファセットの作成・編集・削除
- builtin ファセットの複製（Duplicate as custom）
- ファセット本文エディタ
- テンプレート変数（`{{task}}`等）の表示・編集
- サンプル値によるプレビュー
- Used by 表示（参照元 workflow / step）
- 保存前診断
- 保存後のworkflow参照状態更新

#### 5. Workflow編集（#903）

- custom workflow の作成・編集・削除
- builtin workflow の複製（Duplicate as custom）
- workflow名・説明編集
- step追加 / 削除 / 並び替え
- step mode設定: auto / interactive / approval
- ファセット参照選択（policy / knowledge / instruction / output_contract）。persona 参照はstep編集のGUIには含めない（Step 構造体の persona フィールドは外部エディタでの YAML 直接編集でのみ設定可能。Facets タブでのサブタブ表示・閲覧・管理も対象外）
- inline prompt設定
- transition rules設定
- cycle guard / pass_previous_response / pass_output_from / collect / reduce 設定
- 保存前診断

### UX方針

**builtin / custom の操作権限**:
- builtin は閲覧のみ。編集・削除・外部エディタで開く、いずれも不可
- builtin を複製して custom として作成できる（Duplicate as custom）
- custom はUI上での閲覧・編集に加え、外部エディタで開く機能も提供する

**編集体験**:
- step追加・削除・並び替え、mode設定、ファセット参照選択、inline prompt設定、transition rules設定、collect/reduce設定、output受け渡し設定、cycle guard設定はGUIで完結する。それ以外の複雑な設定は外部エディタで扱える
- step参照やファセット参照は候補選択を基本にし、typoを減らす
- ファセットやworkflow変更時に影響範囲が見える
- 保存前に error / warning を表示する

### 受け入れ条件

**診断**:
- 壊れた workflow は実行前に `error` として検出できる
- 存在しないファセット参照を検出できる
- collect / reduce 周辺の問題（collect 元 step に rules 未設定等）を `warning` として返せる
- 設定画面が一覧と詳細で使える粒度の診断結果を取得できる

**ビルトイン管理**:
- ビルトインワークフロー/ファセットがアプリ更新時に常に最新版に更新される
- ビルトインは閲覧のみ可能。編集・削除・外部エディタで開く操作はUI・Tauriコマンド層の両方で拒否される
- builtin と custom の違いがUI上で分かる
- builtin を複製して custom として作成できる

**閲覧**:
- YAMLやMarkdownを直接読まなくても workflow の構造が分かる
- ファセットがどの workflow / step から参照されているか分かる
- error / warning が一覧と詳細の両方で見える

**ファセット編集**:
- custom ファセットをSettings上で作成・編集・削除できる
- custom ファセットを外部エディタで開ける
- builtin ファセットから custom ファセットを複製できる
- ファセット本文とテンプレート変数の不整合を保存前に検出できる
- 保存後、workflow側の参照状態や診断状態が更新される

**Workflow編集**:
- custom workflow を外部エディタで開ける
- GUIでworkflowの作成・step管理・各種設定（スコープ5に列挙した操作）を実行できる
- step参照やファセット参照の候補は既存定義から選べる
- 壊れたworkflowは保存できない（GUI・Tauriコマンド層で保存を拒否）。保存後に参照先ファセットが削除される等で壊れた場合は、診断で error として検出し一覧に表示する（実行時はエンジンの既存エラーハンドリングが適用される）
- collect / reduce / output受け渡し設定をGUI上で確認・編集できる
- 外部エディタでファイルが変更された場合、ファイル監視で変更を検出し診断が自動で再実行される

### 前提（実装済み）

- #859（Workflow Schema & Storage）
- #861（ファセット指向プロンプト合成）
- #896（Step Output / Collect / Reduce）

## アーキテクチャ概要

### 責務配置

- **診断エンジン (`workflow/diagnostics`)** [新規モジュール]
  - 担当: 全 workflow / facet を走査し、severity (error / warning / info) 付きの診断結果リストを返す。YAML parse エラー、schema 違反、step 参照整合性、ファセット参照存在確認、collect 元 step の rules 有無チェック、到達不能 step 検出、テンプレート変数整合性、ファセットキー/workflow 名の命名規則チェック、ビルトイン info 報告
  - 担当しない: ファイル I/O（storage / facet 層に委譲）、UI 表示、保存処理

- **ビルトイン管理 (`workflow/builtin`)** [既存モジュール・拡張]
  - 担当: `include_str!()` によるビルトイン workflow / facet の提供、ビルトイン判定 (`is_builtin_workflow` / `is_builtin_facet`)、複製用データの提供
  - 担当しない: ディスク書き出し（現在の設計を維持）、編集・削除ガード（コマンド層が担当）

- **ストレージ (`workflow/storage`, `workflow/facet`)** [既存モジュール・拡張]
  - 担当: custom workflow / facet の CRUD、名前/キーバリデーション、ビルトインとカスタムのマージ一覧提供
  - 担当しない: 診断ロジック、ビルトインガード（一次責任はコマンド層。既存のストレージ層ガードは defense-in-depth として維持）
  - 変更点: `list_facets` の返却値を拡張し、builtin / custom 区分と説明を含むサマリ形式にする
  - ファセットの「説明」: Markdown ファイルの先頭行から取得する。先頭行が `# ` で始まる場合はその見出しテキストを、そうでなければ先頭の非空行をそのまま説明として使用する。説明が取得できない場合は空文字列とする

- **Tauri コマンド層 (`workflow/commands`)** [既存モジュール・拡張]
  - 担当: フロントエンド API の提供、ビルトイン操作ガード（save / edit / delete / open_in_editor でビルトイン拒否）、診断結果の中継、複製コマンド、ファイル監視起動・停止
  - 担当しない: ビジネスロジックの実装（診断エンジン・storage に委譲）
  - 新規コマンド: `diagnose_all`, `duplicate_workflow`, `duplicate_facet`, `open_facet_in_editor`
  - 既存コマンド拡張: `save_workflow` / `save_facet` にビルトインガード追加、`list_facets` のレスポンス拡張、`list_workflows` のレスポンスに `is_running` フラグ追加（全 worktree の `WorkflowEngine` 実行状態を集約）

- **ファイル監視 (`watcher`)** [既存モジュール・拡張]
  - 担当: workflow / facet ディレクトリの変更検出、Tauri イベント発火
  - 担当しない: 診断実行（フロントエンドがイベント受信後に `diagnose_all` を再取得）

- **Step スキーマ (`workflow/schema`)** [既存モジュール・拡張]
  - 変更点: `Step` に `inline_prompt: Option<String>` フィールドを追加（`ParallelStep` は対象外）。`inline_prompt` が設定されている step はファセット参照がなくてもバリデーションエラーにしない

- **Automation 管理画面（フロントエンド）** [新規]
  - 担当: Settings 内の Automation セクション UI（Workflows / Facets タブ、一覧・詳細表示、編集フォーム）、Tauri invoke 呼び出し、診断結果の表示、Tauri イベントのリスン
  - 担当しない: バリデーション、診断、ファセット合成、ビジネスロジック全般

### データ/通信フロー

- **一覧取得**: UI → `list_workflows` / `list_facets` invoke → Rust storage（disk + builtin merge）→ サマリリスト → UI。`list_workflows` は各 workflow の `is_running` フラグを含む（全 worktree の WorkflowEngine 実行状態を集約）
- **詳細取得**: UI → `get_workflow` / `get_facet` invoke → Rust storage（disk 優先、builtin fallback）→ 定義データ → UI
- **診断実行**: UI → `diagnose_all` invoke → Rust diagnostics engine（全 workflow + 全 facet 走査）→ severity 付き診断結果リスト → UI
- **保存（workflow）**: UI → `save_workflow` invoke → Rust コマンド層（ビルトインガード → `validation::validate()` による fail-fast バリデーション → storage 保存）→ 成功/バリデーションエラー → UI。保存成功後、UI が `diagnose_all` を再取得して画面を更新。保存前に実行するのは `validation::validate()`（最初のエラーで停止）であり、診断エンジンではない
- **保存（facet）**: UI → `save_facet` invoke → Rust コマンド層（ビルトインガード → ファセットキー命名規則バリデーション + テンプレート変数整合性チェック → storage 保存）→ 成功/バリデーションエラー → UI。保存成功後、UI が `diagnose_all` を再取得して画面を更新
- **ビルトイン複製**: UI → `duplicate_workflow` / `duplicate_facet` invoke → Rust コマンド層（builtin 読み込み → 名前/キー重複チェック → storage.save）→ 成功/エラー → UI
- **外部編集検出**: Rust watcher がファイル変更を検出 → Tauri イベント発火 → UI がイベント受信 → `diagnose_all` + 一覧再取得 → UI 更新
- **外部エディタで開く**: UI → `open_workflow_in_editor` / `open_facet_in_editor` invoke → Rust（ビルトインガード → 外部エディタ起動）
- **Used by 取得**: `diagnose_all` のレスポンスに含まれるファセット→workflow 参照マップから UI が抽出

### 状態 Owner

- **workflow / facet 定義データ（Source of Truth）**: Rust storage 層（ディスクファイル + builtin 埋め込み）
- **ビルトイン / カスタム区分**: Rust builtin モジュール（`is_builtin_workflow` / `is_builtin_facet` で判定）
- **診断結果**: Rust diagnostics engine がオンデマンドで計算。フロントエンドが取得してローカルに保持
- **ファイル監視状態**: Rust watcher（watcher_id によるライフサイクル管理）
- **UI 表示状態（選択中タブ、選択中アイテム、サブタブ等）**: フロントエンド React state
- **編集中ドラフト（workflow / facet の未保存変更）**: フロントエンド React state
- **テンプレート変数サンプル値**: フロントエンド React state（プレビュー用の一時データ）

### 境界

- フロントエンドは Tauri invoke / listen のみでバックエンドと通信する。ファイルシステムに直接アクセスしない
- 全てのバリデーション・診断ロジックは Rust 側に実装する。フロントエンドは結果を受け取って表示するだけ
- ビルトインガードは Tauri コマンド層が最終防衛線。フロントエンドでも操作不可 UI を表示するが、バイパスされても Rust 側で拒否する
- 既存の `validation::validate()` は保存前のfail-fast バリデーション（最初のエラーで停止）として維持する。診断エンジンは全件走査して全ての問題を収集する別関数として実装する
- ファイル監視イベントは汎用 Tauri イベント経由。フロントエンドはリスナーで受信し、必要なデータを再取得する
- テンプレート変数のレンダリングは Rust `WorkflowEngine::render_facet_variables()` が担当。プレビュー用のレンダリングも Tauri コマンド経由で Rust 側で実行する

### 実装に委ねること

- 診断結果の内部型名・構造の詳細（`Diagnostic`, `DiagnosticItem` 等の命名）
- フロントエンドコンポーネントの分割単位（Automation セクション内のコンポーネント構成）
- ファイル監視のデバウンス間隔の具体値
- 診断結果のキャッシュ戦略の有無
- helper 関数の命名・シグネチャ
- テストケースの具体的な配置と構成
- UI の状態管理パターン（useState / useReducer / カスタムフック構成）
- Persona 種別は Facets タブのサブタブに含めない。Persona は外部エディタでの YAML 直接編集でのみ設定する
- parallel block の作成・サブstep管理・aggregate設定の編集UI（表示はスコープ3に含まれるが、parallel固有の編集操作は実装時に範囲を判断する）
- ファセットキーの変更（ファセットキーは作成後イミュータブルとして扱う。変更が必要な場合は削除→新規作成で対応する）
- 一覧の並び順（builtin 先頭 or アルファベット混在等）
- 診断結果の一覧用サマリと詳細用データを同一レスポンスで返すか分割するか
- Workflow リネーム（名前変更）時の旧ファイル削除メカニズム（`save_workflow` に `original_name` パラメータを追加する、`rename_workflow` コマンドを新設する等）。Scenario「元の名前の workflow が一覧から消える」を実現する手段は実装判断に委ねる
- Custom workflow 新規作成時の初期状態（デフォルト step 構成、空 step で保存可能にするか等）
- `save_workflow` / `save_facet` における create と update の区別方法（パラメータ分離、別コマンド化等）

## 振る舞い定義

```gherkin
Feature: Automation 診断
  ワークフローとファセットの設定に対して構文・参照整合性を診断し、
  severity 付きの結果を返す。

  Rule: ワークフロー設定の問題を severity 付きで検出する

    Scenario: 構文エラーのある workflow が error として検出される
      Given YAML 構文が壊れた workflow が存在する
      When 診断を実行する
      Then その workflow に対して severity "error" の診断結果が返る

    Scenario: スキーマ違反の workflow が error として検出される
      Given 有効な YAML だがスキーマに違反する workflow が存在する（必須フィールド欠如、型不一致等）
      When 診断を実行する
      Then その workflow に対して severity "error" の診断結果が返る

    Scenario: 存在しない step 参照が error として検出される
      Given workflow の rules.next / collect.from / pass_output_from のいずれかが存在しない step を参照している
      When 診断を実行する
      Then 参照元の step に対して severity "error" の診断結果が返る

    Scenario: 存在しないファセット参照が error として検出される
      Given workflow step が存在しないファセットキーを参照している
      When 診断を実行する
      Then 参照元の step に対して severity "error" の診断結果が返る

    Scenario Outline: collect 元 step に rules がない場合に warning が出る
      Given collect.reduce が <reduce_strategy> の step で collect 元 step に rules が未設定である
      When 診断を実行する
      Then その collect 設定に対して severity "warning" の診断結果が返る

      Examples:
        | reduce_strategy |
        | any_needs_fix   |
        | all_passed      |

    Scenario: 到達不能な step がある場合に warning が出る
      Given workflow 内に最初の step を除きどの step からも遷移されない step がある
      When 診断を実行する
      Then その step に対して severity "warning" の診断結果が返る

    Scenario: 未定義のテンプレート変数を持つファセットが error として検出される
      Given ファセット本文にシステム定義変数（{{project_name}}, {{task}}）以外のテンプレート変数 {{unknown}} が含まれている
      When 診断を実行する
      Then そのファセットに対して severity "error" の診断結果が返る

    Scenario: inline_prompt 設定済みの step でファセット参照がなくてもエラーにならない
      Given workflow step に inline_prompt が設定されておりファセット参照がない
      When 診断を実行する
      Then その step に対してファセット未設定の error は発生しない

    Scenario: 不正な文字を含むファセットキーが error として検出される
      Given ファセットキーに許可パターン（/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/）に違反する文字が含まれている
      When 診断を実行する
      Then そのファセットに対して severity "error" の診断結果が返る

    Scenario: 不正な文字を含む workflow 名が error として検出される
      Given workflow 名に許可パターン（/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/）に違反する文字が含まれている
      When 診断を実行する
      Then その workflow に対して severity "error" の診断結果が返る

  Rule: ビルトインであることを info レベルで報告する

    Scenario: ビルトイン workflow が info として報告される
      Given ビルトイン workflow が存在する
      When 診断を実行する
      Then その workflow に対して severity "info" の診断結果が返る

    Scenario: ビルトインファセットが info として報告される
      Given ビルトインファセットが存在する
      When 診断を実行する
      Then そのファセットに対して severity "info" の診断結果が返る

  Rule: ファイル変更を監視し診断を自動で再実行する

    Scenario: 外部エディタで workflow ファイルが変更されると診断が再実行される
      Given custom workflow を外部エディタで開いている
      When 外部エディタで workflow ファイルが保存される
      Then ファイル変更が検出され診断が自動で再実行される
      And Automation 画面の診断状態が更新される

    Scenario: 外部エディタでファセットファイルが変更されると診断が再実行される
      Given custom ファセットを外部エディタで開いている
      When 外部エディタでファセットファイルが保存される
      Then ファイル変更が検出され診断が自動で再実行される
      And Automation 画面の診断状態が更新される

    Scenario: GUI 編集中に外部ファイル変更が検出されると警告が表示される
      Given Automation 画面で custom workflow を編集中（未保存変更あり）である
      When 外部エディタで同じ workflow ファイルが変更される
      Then ファイル変更が検出され外部変更の警告が表示される
      And ユーザーはリロード（編集内容を破棄して最新を読み込み）または編集継続を選択できる

    Scenario: GUI で保存した場合も同じ診断が実行される
      Given Automation 画面で custom workflow を編集している
      When GUI から保存する
      Then 保存後に診断が実行され結果が画面に反映される

  Rule: 診断結果を一覧用サマリと詳細用の粒度で提供する

    Scenario: workflow 一覧に診断サマリが表示される
      Given 診断結果を持つ workflow が存在する
      When workflow 一覧を取得する
      Then 各 workflow に error / warning / info の件数サマリが含まれる

    Scenario: workflow 詳細に step 単位の診断結果が表示される
      Given 複数の step に診断結果がある workflow が存在する
      When workflow の診断詳細を取得する
      Then step 名・フィールド名・severity・メッセージが含まれる診断結果が返る

    Scenario: ファセット一覧に診断サマリが表示される
      Given 診断結果を持つファセットが存在する
      When ファセット一覧を取得する
      Then 各ファセットに error / warning / info の件数サマリが含まれる


Feature: ビルトイン管理
  ビルトインのワークフロー/ファセットは常に最新版に保ち、
  カスタマイズは複製で対応する。

  Rule: ビルトインはバイナリに埋め込まれており常に最新版が提供される

    Scenario: カスタム workflow がなくてもビルトイン workflow が一覧に含まれる
      Given カスタム workflow が1つも登録されていない
      When workflow 一覧を取得する
      Then ビルトイン workflow が一覧に含まれる

    Scenario: カスタムファセットがなくてもビルトインファセットが利用できる
      Given カスタムファセットが1つも登録されていない
      When ビルトインファセットを読み込む
      Then ビルトインファセットの内容が返る

  Rule: ビルトインは閲覧のみ可能で編集・削除・外部エディタでの操作ができない

    Scenario: ビルトイン workflow の編集が拒否される
      Given ビルトイン workflow が存在する
      When ビルトイン workflow を編集しようとする
      Then 編集が拒否される

    Scenario: ビルトイン workflow の削除が拒否される
      Given ビルトイン workflow が存在する
      When ビルトイン workflow を削除しようとする
      Then 削除が拒否される

    Scenario: ビルトインファセットの編集が拒否される
      Given ビルトインファセットが存在する
      When ビルトインファセットを編集しようとする
      Then 編集が拒否される

    Scenario: ビルトインファセットの削除が拒否される
      Given ビルトインファセットが存在する
      When ビルトインファセットを削除しようとする
      Then 削除が拒否される

    Scenario: ビルトイン workflow を外部エディタで開けない
      Given ビルトイン workflow が存在する
      When ビルトイン workflow を外部エディタで開こうとする
      Then 操作が拒否される

    Scenario: ビルトインファセットを外部エディタで開けない
      Given ビルトインファセットが存在する
      When ビルトインファセットを外部エディタで開こうとする
      Then 操作が拒否される

  Rule: ビルトインを複製して custom として利用できる

    Scenario: ビルトイン workflow を custom として複製する
      Given ビルトイン workflow が存在する
      When ビルトイン workflow に新しい名前を指定して複製する
      Then 指定した名前で同じ内容の custom workflow が作成される
      And 元のビルトイン workflow は変更されない

    Scenario: ビルトインファセットを custom として複製する
      Given ビルトインファセットが存在する
      When ビルトインファセットに新しいキーを指定して複製する
      Then 指定したキーで同じ内容の custom ファセットが作成される
      And 元のビルトインファセットは変更されない

    Scenario: 同名の custom workflow が既に存在する状態で複製すると拒否される
      Given 同名の custom workflow が既に存在する
      When ビルトイン workflow をその名前で複製しようとする
      Then 名前の重複エラーが表示される

    Scenario: 同名の custom ファセットが既に存在する状態で複製すると拒否される
      Given 同名の custom ファセットが既に存在する
      When ビルトインファセットをそのキーで複製しようとする
      Then キーの重複エラーが表示される

    Scenario: ビルトイン workflow と同名で複製すると拒否される
      Given 同名のビルトイン workflow が存在する
      When ビルトイン workflow をその名前で複製しようとする
      Then 名前の重複エラーが表示される

    Scenario: ビルトインファセットと同キーで複製すると拒否される
      Given 同キーのビルトインファセットが存在する
      When ビルトインファセットをそのキーで複製しようとする
      Then キーの重複エラーが表示される

    Scenario: 複製時に命名規則に違反する workflow 名を指定すると拒否される
      Given ビルトイン workflow が存在する
      When 許可パターン（/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/）に違反する名前で複製しようとする
      Then 命名規則違反エラーが表示される

    Scenario: 複製時に命名規則に違反するファセットキーを指定すると拒否される
      Given ビルトインファセットが存在する
      When 許可パターン（/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/）に違反するキーで複製しようとする
      Then 命名規則違反エラーが表示される

  Rule: builtin と custom が視覚的に区別できる

    Scenario: 一覧で builtin と custom の区分が表示される
      Given builtin と custom の workflow が混在する
      When workflow 一覧を表示する
      Then 各 workflow に builtin / custom の区分が表示される

    Scenario: 一覧で builtin と custom のファセットが区別できる
      Given builtin と custom のファセットが混在する
      When ファセット一覧を表示する
      Then 各ファセットに builtin / custom の区分が表示される


Feature: Automation 管理画面
  Settings 上でワークフローとファセットの構造・参照関係・診断状態を把握できる。

  Rule: Workflow の構造を一覧で把握できる

    Scenario: workflow 一覧に基本情報が表示される
      Given 複数の workflow が登録されている
      When Automation 画面の Workflows タブを表示する
      Then 各 workflow に名前、説明、builtin/custom 区分、診断状態が表示される

  Rule: Workflow の詳細構造を把握できる

    Scenario: workflow 詳細に構成要素が表示される
      Given step を持つ workflow が存在する
      When workflow の詳細を表示する
      Then steps、mode、ファセット参照、rules、collect、reduce、pass_previous_response、pass_output_from、cycle_guard、parallel（aggregate含む）の情報が表示される

    Scenario: workflow 詳細に診断結果が表示される
      Given 診断結果を持つ workflow が存在する
      When workflow の詳細を表示する
      Then step 単位の error / warning / info が表示される

  Rule: Facet の一覧を種別サブタブで把握できる

    Scenario: ファセット一覧が種別サブタブで表示される
      Given 複数の種別のファセットが登録されている
      When Automation 画面の Facets タブを表示する
      Then Policy / Knowledge / Instruction / OutputContract のサブタブが表示される

    Scenario: 種別サブタブ内にファセット一覧が表示される
      Given 選択した種別のファセットが登録されている
      When 種別サブタブを選択する
      Then その種別のファセットに名前、説明、builtin/custom 区分、使用状況（参照元 workflow 件数）が表示される

  Rule: Facet の詳細と参照関係を把握できる

    Scenario: ファセット詳細に内容と参照元が表示される
      Given workflow から参照されているファセットが存在する
      When ファセットの詳細を表示する
      Then 本文プレビュー、variables、参照元 workflow / step（Used by）が表示される

    Scenario: ファセット詳細に診断結果が表示される
      Given 診断結果を持つファセットが存在する
      When ファセットの詳細を表示する
      Then error / warning / info が表示される


Feature: ファセット編集
  custom ファセットを Settings 上で作成・編集・削除でき、
  builtin ファセットは複製してカスタマイズする。

  Rule: Custom ファセットを作成・編集・削除できる

    Scenario: custom ファセットを新規作成する
      Given Automation 画面の Facets タブで種別サブタブを表示している
      When その種別の新しい custom ファセットを作成する
      Then 該当種別のファセット一覧に新しいファセットが追加される

    Scenario: 既存のファセットキーと同じキーで新規作成すると拒否される
      Given 同じキーの custom ファセットが既に存在する
      When そのキーで新しい custom ファセットを作成しようとする
      Then キーの重複エラーが表示される

    Scenario: ビルトインファセットと同じキーで custom ファセットを新規作成すると拒否される
      Given 同じキーのビルトインファセットが存在する
      When そのキーで新しい custom ファセットを作成しようとする
      Then キーの重複エラーが表示される

    Scenario: custom ファセットの本文を編集する
      Given custom ファセットが存在する
      When ファセットの本文を変更して保存する
      Then ファセットの内容が更新される

    Scenario: custom ファセットを削除する
      Given custom ファセットが存在する
      When ファセットを削除する
      Then ファセット一覧からそのファセットが消える

  Rule: Workflow から参照されているファセットの削除は確認が必要

    Scenario: workflow から参照されている custom ファセットを削除しようとすると警告が表示される
      Given workflow step から参照されている custom ファセットが存在する
      When ファセットを削除しようとする
      Then 参照元の workflow 一覧が表示され確認が求められる

    Scenario: 参照されているファセットの削除を確認すると削除が実行される
      Given workflow step から参照されている custom ファセットの削除確認ダイアログが表示されている
      When 削除を確認する
      Then ファセットが削除される
      And 参照元 workflow の診断状態が再計算され参照切れ error が検出される

    Scenario: 参照されているファセットの削除をキャンセルすると削除が中止される
      Given workflow step から参照されている custom ファセットの削除確認ダイアログが表示されている
      When キャンセルする
      Then ファセットは削除されずそのまま残る

  Rule: Custom ファセットを外部エディタで開ける

    Scenario: custom ファセットを外部エディタで開く
      Given custom ファセットが存在する
      When ファセットを外部エディタで開く
      Then ファセットの Markdown ファイルが外部エディタで開かれる

  Rule: ファセット保存前にバリデーションが実行される

    Scenario: テンプレート変数の不整合が保存前に検出される
      Given ファセット本文に未定義のテンプレート変数が含まれている
      When 保存を試みる
      Then 変数の不整合がバリデーションエラーとして表示される
      And 保存が中断される

    Scenario: 命名規則に違反するファセットキーでの保存が拒否される
      Given ファセットキーに許可パターン（/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/）に違反する文字が含まれている
      When 保存を試みる
      Then 命名規則違反がバリデーションエラーとして表示される
      And 保存が中断される

  Rule: ファセットのテンプレート変数をプレビューできる

    Scenario: テンプレート変数にサンプル値を入力してプレビューする
      Given テンプレート変数 {{task}} を含む custom ファセットを編集している
      When task にサンプル値を入力する
      Then サンプル値で展開されたプレビューが表示される

  Rule: ファセット保存後に関連する診断状態が更新される

    Scenario: ファセット保存後に参照元 workflow の診断状態が再計算される
      Given workflow から参照されている custom ファセットが存在する
      When ファセットの内容を変更して保存する
      Then 参照元 workflow の診断状態が再計算される

    Scenario: ファセット削除後に参照元 workflow の診断状態が再計算される
      Given workflow から参照されている custom ファセットが存在する
      When ファセットを削除する
      Then 参照元 workflow の診断状態が再計算され参照切れ error が検出される

  Rule: ファセットの参照元を把握できる

    Scenario: ファセットを参照している workflow / step が Used by として表示される
      Given workflow step から参照されているファセットが存在する
      When ファセットの詳細を表示する
      Then 参照元の workflow 名と step 名が Used by として表示される


Feature: Workflow 編集
  custom workflow を GUI で作成・編集でき、
  保存前および外部編集時に診断で問題を検出する。

  Rule: Custom workflow を作成・削除できる

    Scenario: custom workflow を新規作成する
      Given Automation 画面の Workflows タブを表示している
      When 新しい custom workflow を作成する
      Then workflow 一覧に新しい workflow が追加される

    Scenario: 既存の workflow 名と同じ名前で新規作成すると拒否される
      Given 同じ名前の custom workflow が既に存在する
      When その名前で新しい custom workflow を作成しようとする
      Then 名前の重複エラーが表示される

    Scenario: ビルトイン workflow と同じ名前で custom workflow を新規作成すると拒否される
      Given 同じ名前のビルトイン workflow が存在する
      When その名前で新しい custom workflow を作成しようとする
      Then 名前の重複エラーが表示される

    Scenario: custom workflow を削除する
      Given custom workflow が存在する
      When workflow を削除する
      Then workflow 一覧からその workflow が消える

  Rule: 実行中の workflow を編集・削除する場合は警告が表示される

    Scenario: 実行中の workflow を編集して保存すると警告が表示される
      Given WorkflowEngine で実行中の custom workflow が存在する
      When その workflow を編集して保存しようとする
      Then 実行中であることの警告が表示され確認が求められる
      And 確認後に保存が実行される
      And 実行中のインスタンスには影響しない

    Scenario: 実行中の workflow を削除しようとすると警告が表示される
      Given WorkflowEngine で実行中の custom workflow が存在する
      When その workflow を削除しようとする
      Then 実行中であることの警告が表示され確認が求められる

    Scenario: 実行中の workflow の削除確認をキャンセルする
      Given WorkflowEngine で実行中の custom workflow を削除しようとしている
      When 確認ダイアログでキャンセルする
      Then 削除が中止され workflow はそのまま残る

  Rule: Workflow の基本情報を編集できる

    Scenario: workflow の名前と説明を編集する
      Given custom workflow を編集している
      When workflow の名前と説明を変更して保存する
      Then workflow 一覧に更新された名前と説明が表示される
      And 元の名前の workflow が一覧から消える

    Scenario: workflow 名を既存の custom workflow と同じ名前に変更すると拒否される
      Given 別の custom workflow と同じ名前に変更しようとしている
      When workflow の名前を変更して保存する
      Then 名前の重複エラーが表示される

    Scenario: workflow 名をビルトイン workflow と同じ名前に変更すると拒否される
      Given ビルトイン workflow と同じ名前に変更しようとしている
      When workflow の名前を変更して保存する
      Then 名前の重複エラーが表示される

  Rule: Custom workflow を外部エディタで開ける

    Scenario: custom workflow を外部エディタで開く
      Given custom workflow が存在する
      When workflow を外部エディタで開く
      Then workflow の YAML ファイルが外部エディタで開かれる

  Rule: Workflow の step を GUI で管理できる

    Scenario: workflow に step を追加する
      Given custom workflow を編集している
      When 新しい step を追加する
      Then workflow の step 一覧に step が追加される

    Scenario: workflow から step を削除する
      Given 複数の step を持つ custom workflow を編集している
      When step を削除する
      Then workflow の step 一覧からその step が消える

    Scenario: step の順序を変更する
      Given 複数の step を持つ custom workflow を編集している
      When step の順序を変更する
      Then workflow の step が新しい順序で並ぶ

    Scenario: 空の step 名で追加すると拒否される
      Given custom workflow を編集している
      When step 名を空にして追加しようとする
      Then step 名が空であるバリデーションエラーが表示される

    Scenario: 既存の step と同名で追加すると拒否される
      Given custom workflow に step "review" が存在する
      When 同名の step "review" を追加しようとする
      Then step 名の重複バリデーションエラーが表示される

  Rule: Step の設定を GUI で編集できる

    Scenario: step の mode を設定する
      Given custom workflow の step を編集している
      When mode を auto / interactive / approval から選択する
      Then step の mode が更新される

    Scenario: step にファセット参照を種別ごとの候補から選択して設定する
      Given custom workflow の step を編集している
      And 該当種別の登録済みファセットが存在する
      When ファセットスロット（policy / knowledge / instruction / output_contract。persona は対象外）の候補一覧から選択する
      Then step に選択した種別のファセットへの参照が設定される

    Scenario: step に inline prompt を設定する
      Given custom workflow の step を編集している
      When inline prompt のテキストを入力する
      Then step に inline prompt が設定される

    Scenario: step に transition rules を設定する
      Given custom workflow の step を編集している
      When transition rules の条件と遷移先を設定する
      Then step の遷移条件が更新される

    Scenario: step に collect / reduce を設定する
      Given custom workflow の step を編集している
      When collect 元と reduce 戦略を設定する
      Then step の collect / reduce 設定が更新される

    Scenario: step に output 受け渡しを設定する
      Given custom workflow の step を編集している
      When pass_previous_response または pass_output_from を設定する
      Then step の output 受け渡し設定が更新される

    Scenario: step に cycle guard を設定する
      Given custom workflow の step を編集している
      When cycle guard の max_iterations を設定する
      Then step の cycle guard 設定が更新される

  Rule: 壊れた workflow は保存できない

    Scenario: バリデーションエラーのある workflow の保存が拒否される
      Given custom workflow に validation::validate() で検出されるエラーがある
      When 保存を試みる
      Then バリデーションエラーにより保存が拒否されエラーの内容が表示される
```
