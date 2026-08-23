## B-001: provider非依存permission値の受理

GIVEN `session.permission`以外が妥当なYAMLまたはLuaのworkflow定義がある
WHEN `permission`に`manual`、`auto`、`bypass`、`read-only`のいずれかを指定してloadする
THEN `permission`を理由とするError Diagnosticは発生しない

## B-002: manualのprovider別起動

GIVEN `permission: manual`を指定したSessionがloadされている
WHEN Releashが選択されたproviderでSessionを起動する
THEN permissionに対応するclaudeの起動フラグ列は`--permission-mode default`となる
AND permissionに対応するcodexの起動フラグ列は`--sandbox workspace-write --ask-for-approval on-request`となる
AND claudeはツールの初回使用ごとに確認し、codexはモデルが必要と判断したときに確認する

## B-003: autoのprovider別起動

GIVEN `permission: auto`を指定したSessionがloadされている
WHEN Releashが選択されたproviderでSessionを起動する
THEN permissionに対応するclaudeの起動フラグ列は`--permission-mode auto`となる
AND permissionに対応するcodexの起動フラグ列は`--approve-for-me`となる

## B-004: bypassのprovider別起動

GIVEN `permission: bypass`を指定したSessionがloadされている
WHEN Releashが選択されたproviderでSessionを起動する
THEN permissionに対応するclaudeの起動フラグ列は`--permission-mode bypassPermissions`となる
AND permissionに対応するcodexの起動フラグ列は`--dangerously-bypass-approvals-and-sandbox`となる

## B-005: read-onlyのprovider別起動

GIVEN `permission: read-only`を指定したSessionがloadされている
WHEN Releashが選択されたproviderでSessionを起動する
THEN permissionに対応するclaudeの起動フラグ列は`--permission-mode plan`となる
AND permissionに対応するcodexの起動フラグ列は`--sandbox read-only --ask-for-approval never`となる

## B-006: permission省略時のprovider既定値

GIVEN `permission`を省略した妥当なSession定義がloadされている
WHEN ReleashがSessionを起動する
THEN permissionに対応する起動フラグはprovider CLIへ渡されない
AND 権限の動作はprovider CLIの既定に委ねられる

## B-007: 未知permission値の拒否

GIVEN `session.permission`以外が妥当なYAMLまたはLuaのworkflow定義がある
WHEN `permission`に4つの受理値以外を指定してloadする
THEN 値が不正であることを示すError Diagnosticが返る
AND Diagnosticは当該`permission` fieldの位置を示す
AND provider固有値を含む不正値は受理値のaliasへ変換されない
AND そのworkflow定義は実行できない

## B-008: YAMLとLuaで共通のpermission Diagnostic

GIVEN 同じ不正な`permission`を持つ同等のworkflow定義がYAMLとLuaで記述されている
WHEN 両方の定義をloadする
THEN 同じ分類と同じ原因を表すDiagnosticが返る
AND どちらのDiagnosticも誤りのある`permission` fieldの位置を示す

## B-009: autoでの完了提出

GIVEN `permission: auto`で起動したSession nodeが完了条件を満たす提出値を確定している
WHEN Session nodeが`releash workflow output submit`による完了アクションを実行する
THEN claudeとcodexのどちらでもpermissionを理由にコマンド実行を拒否されない
AND 成功したSubmitはworkflowから完了信号として観測できる

## B-010: read-onlyでの完了提出

GIVEN `permission: read-only`で起動したSession nodeが完了を提出しようとしている
WHEN Session nodeが`releash workflow output submit`による完了アクションを実行する
THEN claudeではコマンド実行が拒否され、Session nodeは自動では完了しない
AND claudeでは人間がprovider側の権限を変更するまで自動完了は利用できない
AND codexではpermissionを理由にコマンド実行を拒否されない

## B-011: builtin Sessionのauto統一

GIVEN builtinとして登録される8本のworkflow定義がある
WHEN Releashが8本をbuiltin workflowとしてloadする
THEN 58個のSessionすべてに`permission: auto`が指定されている
AND claudeとcodexで異なるprovider固有値は記述されていない

## B-012: 完成形workflow例のload

GIVEN `workflows/examples/full-cycle-development.yml`がある
WHEN workflow例をloadする
THEN すべての`permission`は4つの受理値のいずれかである
AND Error Diagnosticは発生しない

## B-013: LuaLS stubのpermission補完

GIVEN workflow用のLuaLS補完ファイルを生成する
WHEN 利用者が`.releash/releash.lua`のSession optionsを参照する
THEN `permission`の受理値として`manual`、`auto`、`bypass`、`read-only`が示される
AND 任意の`string`を受理するfieldとしては示されない

## B-014: model指定の無変換維持

GIVEN `model`を指定した妥当なSession定義がloadされている
WHEN ReleashがSessionを起動する
THEN 指定した`model`はprovider CLIへ無変換で渡される
AND `permission`の指定または省略によって`model`の意味は変更されない

## B-015: workflow定義正本でのpermission説明

GIVEN 利用者が`docs/glossary/WORKFLOW.md`でSessionの起動設定を確認する
WHEN `model`と`permission`の説明を読む
THEN `permission`はReleashが所有する4値でありproviderごとの起動フラグへ写像されるものとして説明されている
AND `model`はprovider CLIへ無変換で渡されるものとして`permission`と区別されている
AND `manual`ではclaudeとcodexの確認頻度の決定者が異なることが説明されている
AND `read-only`ではclaudeで完了提出が拒否され自動完了しないことが説明されている

## B-016: 旧permission値を含む実行履歴の読み出し

GIVEN provider固有のpermission値を含むfactが永続化された実行木がある
WHEN Releashがその実行木の履歴を読み出す
THEN 読み出しは失敗し、履歴は取得できない
AND その実行木を含む起動時reconciliationはエラーを返す

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-003, B-004, B-005 |
| R-002 | B-006 |
| R-003 | B-007 |
| R-004 | B-007, B-008 |
| R-005 | B-002, B-003, B-004, B-005 |
| R-006 | B-002, B-015 |
| R-007 | B-009 |
| R-008 | B-010, B-015 |
| R-009 | B-011 |
| R-010 | B-012 |
| R-011 | B-001, B-007, B-008, B-013 |
| R-012 | B-014, B-015 |
| R-013 | B-016 |
