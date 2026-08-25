## B-001: per-worktree PTY cap到達の記録

GIVEN AgentSessionのterminal spawnがper-worktree PTY cap到達によって失敗する
WHEN 起動失敗の処理が完了する
THEN 失敗後に参照できるReleashの記録から、失敗理由がper-worktree PTY cap到達であると区別できる
AND その記録に対象のworktree pathが含まれる

## B-002: PTY総数cap到達の記録

GIVEN AgentSessionのterminal spawnがPTY総数cap到達によって失敗する
WHEN 起動失敗の処理が完了する
THEN 失敗後に参照できるReleashの記録から、失敗理由がPTY総数cap到達であると区別できる

## B-003: owner衝突の記録

GIVEN AgentSessionのterminal spawnがowner衝突によって失敗する
WHEN 起動失敗の処理が完了する
THEN 失敗後に参照できるReleashの記録から、失敗理由がowner衝突であると区別できる

## B-004: PTYのopenまたはfork失敗の記録

GIVEN AgentSessionのterminal spawnがPTYのopenまたはfork失敗によって失敗する
WHEN 起動失敗の処理が完了する
THEN 失敗後に参照できるReleashの記録から、失敗理由がPTYのopenまたはfork失敗であると区別できる
AND その記録に発生源が返したエラー内容が含まれる

## B-005: workflowのSession Node起動失敗の記録

GIVEN workflowのSession Nodeのterminal spawnがper-worktree PTY cap到達、PTY総数cap到達、owner衝突、PTYのopenまたはfork失敗、またはこれら以外のspawn失敗によって失敗する
WHEN そのNodeの失敗が記録される
THEN Nodeの失敗理由として残る記録から、該当する失敗理由を区別できる
AND per-worktree PTY cap到達の場合は対象のworktree pathが含まれる
AND PTYのopenまたはfork失敗、またはこれら以外のspawn失敗の場合は発生源が返したエラー内容が含まれる

## B-006: standalone起動失敗の記録

GIVEN workflow外でstandalone AgentSessionのterminal spawnがper-worktree PTY cap到達、PTY総数cap到達、owner衝突、PTYのopenまたはfork失敗、またはこれら以外のspawn失敗によって失敗する
WHEN standalone起動の失敗処理が完了する
THEN 失敗後に参照できるReleashの記録から、該当する失敗理由を区別できる
AND per-worktree PTY cap到達の場合は対象のworktree pathが含まれる
AND PTYのopenまたはfork失敗、またはこれら以外のspawn失敗の場合は発生源が返したエラー内容が含まれる

## B-007: history resume失敗の記録

GIVEN workflow外でAgentSessionのhistory resumeに伴うterminal spawnがper-worktree PTY cap到達、PTY総数cap到達、owner衝突、PTYのopenまたはfork失敗、またはこれら以外のspawn失敗によって失敗する
WHEN history resumeの結果が確定する
THEN 失敗後に参照できるReleashの記録から、該当する失敗理由を区別できる
AND per-worktree PTY cap到達の場合は対象のworktree pathが含まれる
AND PTYのopenまたはfork失敗、またはこれら以外のspawn失敗の場合は発生源が返したエラー内容が含まれる

## B-008: GUIプロセスの警告およびエラーログ

GIVEN パッケージ版として配布されたReleashのGUIプロセスが起動している
WHEN `log` crate経由で警告およびエラーが記録され、GUIプロセスが終了する
THEN 警告およびエラーの内容がローカルのログファイルに書き出されている
AND プロセス終了後もそのログファイルから内容を参照できる

## B-009: ローカルログの増大上限

GIVEN パッケージ版Releashがローカルのログファイルへ継続して警告またはエラーを書き出している
WHEN ログの保持量が定められたサイズまたは世代の上限に達する
THEN 保持されるローカルログはその上限を超えて増大しない

## B-010: ローカルログの外部非送信

GIVEN パッケージ版Releashが`log` crate経由の警告またはエラーをローカルのログファイルへ書き出す
WHEN ログの記録が完了する
THEN そのローカルログファイルの内容は、このログ記録機能によってReleashの外部へ送信されない

## B-011: 上記以外のspawn失敗の記録

GIVEN AgentSessionのterminal spawnがper-worktree PTY cap到達、PTY総数cap到達、owner衝突、PTYのopenまたはfork失敗のいずれでもない失敗によって失敗する
WHEN 起動失敗の処理が完了する
THEN 失敗後に参照できるReleashの記録から、失敗理由がこれら4分類以外のspawn失敗であると区別できる
AND その記録に発生源が返したエラー内容が含まれる

## B-012: CLIプロセスの警告およびエラーログ

GIVEN パッケージ版として配布されたReleashのCLIプロセスが実行されている
WHEN `log` crate経由で警告およびエラーが記録され、CLIプロセスが終了する
THEN 警告およびエラーの内容がローカルのログファイルに書き出されている
AND プロセス終了後もそのログファイルから内容を参照できる

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-003, B-004, B-011 |
| R-002 | B-001, B-005, B-006, B-007 |
| R-003 | B-004, B-005, B-006, B-007, B-011 |
| R-004 | B-005 |
| R-005 | B-006, B-007 |
| R-006 | B-008, B-012 |
| R-007 | B-009 |
| R-008 | B-010 |
