# Behavior

対象 Issue: #986「`notion` integration のロジックをクリーンアーキテクチャ構成へ移行する」

本書は `requirements.md` の要求を、実装詳細を含まない**外部から観測可能な振る舞い**として Gherkin で定義する。型名・関数名・モジュール経路といった実装整合の詳細は design / 実装の責務とし、本書では「どの入力に対して、どの結果（戻り値・エラー表現）が観測されるか」と、本移行の成果物として観測可能な構造不変条件のみを規定する。

本 Issue は純粋なリファクタリング（クリーンアーキテクチャ移行）であり、Notion 連携機能の仕様変更・機能追加を含まない。したがって本書の中心は「移行の前後で、対象 Tauri command の引数・戻り値・エラー表現が等価であること（観測可能な振る舞いが不変であること）」である。

## 仮定

- **A1**【確定】: 本書での「観測」は、対象 Tauri command（`query_notion_tasks` / `fetch_notion_label_options` / `save_notion_config` / `get_notion_config` / `delete_notion_config` / `validate_notion_config`）への入力（引数）と、その `Result` 戻り値（成功値の構造・エラー文字列・serialize 表現）を指す。Notion API への HTTP 通信そのものは外部から直接見えないが、command の戻り値を介して観測可能とみなす。
- **A2**【確定】: 「移行前後で等価」とは、同一入力・同一外部状態（保存済み config の有無、Notion API の応答）に対して、command の戻り値（成功値の JSON 表現、エラー文字列、`NotionConfigStatus` の serialize 値）が一致することを指す。内部のレイヤー構成・型配置・呼び出し回数・clone 回数は観測対象に含めない。
- **A3**【確定】: 現状の `validate_notion_config` の判定（空 token または空 database_id を `NotConfigured` とする、HTTP status から `InvalidToken` / `InvalidDatabase` / `NetworkError` / `Configured` を決める）は仕様であり、移行後も等価に保持する（requirements Assumptions を継承）。
- **A4**【確定】: API token を含む値オブジェクトの `Debug` 出力が `[REDACTED]` 相当でマスクされる現状の振る舞いは仕様であり、移行後も維持する（requirements Constraints を継承）。
- **A5**: 保存済み config の serialize/deserialize における後方互換（`labels` の文字列配列形式、`title` の既定値 `"Name"`、`branch_prefix` の既定空文字）は、config 読み取り結果に影響する観測可能な振る舞いとして移行後も維持する。command 入力の protocol DTO では `labels` 文字列配列の後方互換は維持せず、構造化された label property 配列を受け付ける。
- **A6**: ゲートウェイ／インフラ層の最終配置、shared config model の配置・mapping 方針といった実装上の選択は本書の対象外とし、design.md で確定する。本書は「依存方向の規約に違反せず、`notion` への逆依存が残らないこと」を観測可能な構造不変条件としてのみ規定する。

---

## Feature: Notion integration の振る舞いを保ったままクリーンアーキテクチャ構成へ移行する

Notion task query・label option fetch・Notion config の save/get/delete/validate を、レイヤー規約に沿った構成へ移行する。移行は純粋移行であり、対象 Tauri command の観測可能な振る舞いを一切変えない。

### Background

```gherkin
Given Releash バックエンドが本 Issue（#986）の notion 移行を適用済みのビルドで動作している
And app-config repository（保存済み Notion config の get/upsert/remove）が利用可能である
And 対象 Tauri command（query_notion_tasks / fetch_notion_label_options / save_notion_config / get_notion_config / delete_notion_config / validate_notion_config）が登録され invoke 可能である
```

---

### Rule: 保存済み config がある repo に対する task query は task page を返す

```gherkin
Scenario: configured な repo で task を query する
  Given repo_path に対応する Notion config が保存されている
  And Notion API が task の page を正常に返す
  When query_notion_tasks を repo_path と query 指定で呼ぶ
  Then tasks・has_more・next_cursor を含む task page が成功として返る

Scenario: page_size と cursor を指定して次ページを query する
  Given repo_path に対応する Notion config が保存されている
  And query に cursor と page_size が指定されている
  When query_notion_tasks を呼ぶ
  Then 指定に対応する範囲の task page が返り、has_more と next_cursor で続きの有無が示される
```

### Rule: config 未保存の repo に対する Notion 操作は「設定が見つからない」エラーを返す

```gherkin
Scenario: unconfigured な repo で task を query する
  Given repo_path に対応する Notion config が保存されていない
  When query_notion_tasks を呼ぶ
  Then "Notion設定が見つかりません" を含むエラーが返る
  And Notion API への問い合わせは行われない

Scenario: unconfigured な repo で label option を fetch する
  Given repo_path に対応する Notion config が保存されていない
  When fetch_notion_label_options を呼ぶ
  Then "Notion設定が見つかりません" を含むエラーが返る
```

### Rule: 保存済み config がある repo の label option fetch は label option の一覧を返す

```gherkin
Scenario: configured な repo で label option を fetch する
  Given repo_path に対応する Notion config が保存されている
  And Notion API が property とその選択肢を正常に返す
  When fetch_notion_label_options を呼ぶ
  Then property_name・property_type・options（および option_ids）を持つ label option の一覧が成功として返る
```

### Rule: Notion API 呼び出しが失敗した場合はエラー表現を保ったまま失敗を返す

```gherkin
Scenario: task query 中に API がエラーを返す
  Given repo_path に対応する Notion config が保存されている
  And Notion API がリクエスト失敗・API エラー・パース不能のいずれかで応答する
  When query_notion_tasks を呼ぶ
  Then 失敗が現状と等価なエラー文字列でユーザーに返る
  And 成功値（task page）は返らない
```

### Rule: config の save / get / delete は app-config repository を介して反映される

```gherkin
Scenario: Notion config を保存する
  Given repo_path と api_token・database_id・property_mapping が与えられている
  When save_notion_config を呼ぶ
  Then その config が repo_path に対して upsert され、成功（Ok）が返る

Scenario: 保存済み config を取得する
  Given repo_path に対応する Notion config が保存されている
  When get_notion_config を呼ぶ
  Then 保存済み config が Some として返る

Scenario: 未保存 repo の config を取得する
  Given repo_path に対応する Notion config が保存されていない
  When get_notion_config を呼ぶ
  Then None が成功として返る

Scenario: Notion config を削除する
  Given repo_path に対応する Notion config が保存されている
  When delete_notion_config を呼ぶ
  Then その config が削除され、成功（Ok）が返る
```

### Rule: validate_notion_config は token / database_id と API 応答から config 状態を判定する

```gherkin
Scenario Outline: 空入力は NotConfigured と判定する
  Given api_token が "<token>"、database_id が "<db>" である
  When validate_notion_config を呼ぶ
  Then status が not_configured で properties が空の結果が返る
  And Notion API への問い合わせは行われない

  Examples:
    | token | db   |
    |       | db-1 |
    | tok-1 |      |
    |       |      |

Scenario: 正しい token と database で validate する
  Given 空でない api_token と database_id が与えられている
  And Notion API が database を正常に返す
  When validate_notion_config を呼ぶ
  Then status が configured で、database の property 情報を含む結果が返る

Scenario: 認証に失敗する token で validate する
  Given 空でない api_token と database_id が与えられている
  And Notion API が UNAUTHORIZED を返す
  When validate_notion_config を呼ぶ
  Then status が invalid_token の結果が返る

Scenario: 存在しない / 不正な database で validate する
  Given 空でない api_token と database_id が与えられている
  And Notion API が NOT_FOUND・BAD_REQUEST・その他の非成功・パース不能のいずれかで応答する
  When validate_notion_config を呼ぶ
  Then status が invalid_database の結果が返る

Scenario: ネットワークに到達できない状態で validate する
  Given 空でない api_token と database_id が与えられている
  And Notion API への送信がネットワーク要因で失敗する
  When validate_notion_config を呼ぶ
  Then status が network_error の結果が返る
```

### Rule: config の serialize / deserialize の後方互換と既定値が保たれる（A5）

```gherkin
Scenario: NotionConfigStatus が snake_case で serialize される
  Given validate 結果の status が determined である
  When 結果を JSON へ serialize する
  Then status は not_configured / configured / invalid_token / invalid_database / network_error のいずれかの snake_case 文字列になる

Scenario: 旧形式（文字列配列）の labels を読み取る
  Given 保存済み config の labels が property 名の文字列配列で記録されている
  When その config を読み取る
  Then 各 label は property_type を "select" とする label として解釈される

Scenario: command 入力では構造化 labels 配列を受け付ける
  Given command 入力の property_mapping.labels が渡されている
  When save_notion_config を呼ぶ
  Then labels は name と property_type を持つ構造化 label property 配列として解釈される
  And 旧形式の文字列配列は command 入力の後方互換対象ではない

Scenario: 省略された mapping 項目に既定値が適用される
  Given 保存済み config で property_mapping の一部項目が省略されている
  When その config を読み取る
  Then title は省略時 "Name" になり、branch_prefix は省略時 空文字になる
```

### Rule: API token はログ・Debug 出力でマスクされる（A4）

```gherkin
Scenario: token を含む値オブジェクトを Debug 出力する
  Given api_token を保持する Notion config 値オブジェクトがある
  When その値を Debug 整形する
  Then api_token は [REDACTED] 相当にマスクされ、生の token が出力に現れない
```

---

### Rule: 移行の成果として、レイヤー規約違反と `notion` への逆依存が残らない（構造不変条件）

> 本 Rule は純粋移行の受け入れ条件であり、開発者・CI から観測可能な構造的成果を coarse に規定する。詳細な配置・分割方針は design.md で確定する。

```gherkin
Scenario: 旧 notion モジュールが除去されている
  Given 移行が完了している
  When バックエンドのモジュール構成を確認する
  Then 旧構成 src-tauri/src/notion/ が存在せず、lib.rs に mod notion が残っていない
  And 同一責務の重複実装が残っていない

Scenario: app_config ゲートウェイから notion への逆依存が解消されている
  Given 移行が完了している
  When app_config ゲートウェイの依存を確認する
  Then adaptor/gateway/app_config が crate::notion::types を import していない
  And レイヤー間の依存方向が規約（依存は内向きのみ）に従っている

Scenario: command 登録が新配置へ整合している
  Given 移行が完了している
  When Tauri command の登録を確認する
  Then 対象 command は crate::notion::* を直接登録せず、ユースケースを呼ぶ薄い入口として新配置から登録されている

Scenario: 品質チェックが通過する
  Given 移行が完了している
  When CI 相当の品質チェックを実行する
  Then cargo fmt --check / cargo clippy -- -D warnings / cargo test が通過する
  And フロントエンド lint / test / build が通過する
```

---

## Open Questions

なし。
