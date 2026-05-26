# Behavior

## Source
- requirements.md

## Behavior

```gherkin
Feature: 環境別 CLI alias と workflow 変数による facet 展開

  Rule: 起動環境に応じた CLI alias の解決
    Scenario: 本番起動の agent は releash を受け取る
      Given Releash が本番ビルドで起動している
      When agent が facet 経由で CLI コマンドの提示を受ける
      Then 提示されるコマンドは `releash workflow ...` 形式である

    Scenario: dev 起動の agent は releash-dev を受け取る
      Given Releash が dev ビルドで起動している
      When agent が facet 経由で CLI コマンドの提示を受ける
      Then 提示されるコマンドは `releash-dev workflow ...` 形式である

  Rule: dev 起動による本番 CLI の不変性
    Scenario: dev 起動しても本番 CLI 実体は維持される
      Given 本番 CLI が既にシステムにインストールされている
      When Releash を dev ビルドで起動する
      Then 本番 CLI は dev 起動後も本番用 CLI として解決・動作し続ける

  Rule: CLI alias と実行対象の一意な対応
    Scenario: dev alias は dev データ領域に紐づく
      Given Releash が dev ビルドで起動している
      When agent が `releash-dev` 経由で workflow の状態参照コマンドを実行する
      Then 参照される workflow 状態は dev データ領域のものである

    Scenario: 本番 alias は本番データ領域に紐づく
      Given Releash が本番ビルドで起動している
      When agent が `releash` 経由で workflow の状態参照コマンドを実行する
      Then 参照される workflow 状態は本番データ領域のものである

    Scenario: 子プロセス側の明示指定が alias 内包値より優先される
      Given Releash が dev ビルドで起動している
      And 子プロセスが RELEASH_DATA_DIR を明示指定して起動される
      When その子プロセス内で workflow の状態参照コマンドを実行する
      Then 参照される workflow 状態は明示指定されたデータ領域のものである

  Rule: agent 子プロセスへの実行環境の伝搬
    Scenario: agent が起動する子プロセスは起動環境の alias を解決できる
      Given Releash が特定のビルド種別で起動している
      When agent がコマンド実行用の子プロセスを起動する
      Then 子プロセスはその環境に対応する CLI alias とデータ領域を解決できる

  Rule: facet テンプレートにおける CLI alias の展開
    Scenario: 本番環境では {{path_alias.releash}} が releash に展開される
      Given Releash が本番ビルドで起動している
      And facet 本文に {{path_alias.releash}} が含まれている
      When facet 本文が agent 向けに展開される
      Then 該当部分は `releash` に置き換えられる

    Scenario: dev 環境では {{path_alias.releash}} が releash-dev に展開される
      Given Releash が dev ビルドで起動している
      And facet 本文に {{path_alias.releash}} が含まれている
      When facet 本文が agent 向けに展開される
      Then 該当部分は `releash-dev` に置き換えられる

    Scenario: built-in prompt 内の固定 releash 表記も環境別 alias に展開される
      Given built-in prompt 内で従来固定で `releash` を出していたコマンド表記が含まれている
      When built-in prompt が agent 向けに展開される
      Then 本番起動では `releash`、dev 起動では `releash-dev` として agent に提示される

  Rule: workflow 定義変数の facet 展開
    Scenario: workflow が宣言した変数は {{vars.<name>}} で facet から参照できる
      Given workflow 定義が変数 `<name>` を宣言している
      And facet 本文に {{vars.<name>}} が含まれている
      When facet 本文が agent 向けに展開される
      Then 該当部分は workflow 定義側で宣言された値に置き換えられる

    Scenario: 未定義の {{vars.<name>}} を参照する workflow は拒否される
      Given workflow 定義に存在しない `<name>` を facet 本文が {{vars.<name>}} として参照している
      When その workflow を読み込みまたは保存または展開しようとする
      Then 未定義変数参照として明示的なエラーが提示される

  Rule: 既存プレースホルダの互換性
    Scenario: {{project_name}} / {{task}} の展開結果は変更前と等価である
      Given facet 本文に {{project_name}} や {{task}} などの既存プレースホルダのみが含まれている
      When facet 本文が agent 向けに展開される
      Then 展開結果は本変更前と等価である
```
