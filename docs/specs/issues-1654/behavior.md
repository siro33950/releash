## B-001: 成功したsession nodeのterminal枠を解放する

GIVEN session node executionが`Running`で、そのnodeが起動したproviderプロセスがworktreeのterminal surface枠を占有し、プロセスの停止処理が成功する
WHEN node executionが`Succeeded`へ遷移する
THEN そのproviderプロセスは終了する
AND 対応するterminal surfaceはworktreeの枠を占有しなくなる

## B-002: 失敗したsession nodeのterminal枠を解放する

GIVEN session node executionがactiveで、そのnodeが起動したproviderプロセスがworktreeのterminal surface枠を占有し、プロセスの停止処理が成功する
WHEN node executionが`Failed`へ遷移する
THEN そのproviderプロセスは終了する
AND 対応するterminal surfaceはworktreeの枠を占有しなくなる

## B-003: abortされたsession nodeのterminal枠を解放する

GIVEN session node executionがactiveで、そのnodeが起動したproviderプロセスがworktreeのterminal surface枠を占有し、プロセスの停止処理が成功する
WHEN node executionが`Aborted`へ遷移する
THEN そのproviderプロセスは終了する
AND 対応するterminal surfaceはworktreeの枠を占有しなくなる

## B-004: 停止されたsessionの会話を引き継いで復帰する

GIVEN 終端したsession nodeのproviderプロセスが停止され、停止前のterminal表示とproviderの会話が存在する
WHEN 利用者が後からそのAgentSessionを開いて再開する
THEN 停止前のterminal表示を確認できる
AND providerとの会話を停止前の文脈から継続できる

## B-005: Runningのsession nodeは停止しない

GIVEN session node executionが`Running`で、そのnodeのproviderプロセスが動作している
WHEN node executionが`Running`のまま処理を継続する
THEN providerプロセスは停止されない
AND 利用者はそのAgentSessionを引き続き操作できる

## B-006: execution stop後にsession nodeを再開できる

GIVEN `Running`のsession node executionと、そのnodeが起動したproviderプロセスが存在する
WHEN 利用者がworkflow executionをstopしてnode executionが`Paused`になった後、workflow executionをresumeする
THEN providerプロセスはnode executionが`Paused`の間も停止されない
AND node executionは`Running`へ戻り、同じAgentSessionで処理を継続できる

## B-007: 承認待ちのsession nodeへ再指示できる

GIVEN session node executionが`WaitingApproval`で、そのnodeのproviderプロセスが動作している
WHEN 利用者がそのAgentSessionへ追加の指示を送る
THEN providerプロセスは停止されず、追加の指示を受け付ける
AND providerとの会話を同じAgentSessionで継続できる

## B-008: 終端済みnodeを重ねても新しいsession nodeを起動できる

GIVEN 同一worktreeでsession nodeを含むworkflowを`per_worktree_cap`を超える回数繰り返し実行し、それらのnode executionがすべて終端している
WHEN 新しいworkflowのsession nodeをactivateする
THEN 終端済みnode executionのsessionは`per_worktree_cap`の使用数に含まれない
AND `WorktreeCapReached`に起因する`TerminalUnavailable`にならず、session nodeを起動できる

## B-009: 終端済みnodeを重ねても手動sessionを作成できる

GIVEN 同一worktreeでsession nodeを含むworkflowを`per_worktree_cap`を超える回数繰り返し実行し、それらのnode executionがすべて終端している
WHEN 利用者がそのworktreeに手動のAgentSessionを作成する
THEN 終端済みnode executionのsessionは`per_worktree_cap`の使用数に含まれない
AND `WorktreeCapReached`に起因する`TerminalUnavailable`にならず、AgentSessionを作成できる

## B-010: 停止失敗でSubmitの確定結果を覆さない

GIVEN Submitによって`Succeeded`へ遷移するsession node executionがあり、そのnodeのproviderプロセスの停止が失敗する
WHEN 利用者がそのnodeの出力をSubmitする
THEN Submitは成功として返る
AND node executionは`Succeeded`のまま維持される

## B-011: 停止失敗で承認の確定結果を覆さない

GIVEN `WaitingApproval`のsession node executionがあり、そのnodeのproviderプロセスの停止が失敗する
WHEN 利用者がそのnodeを承認する
THEN 承認は成功として返る
AND node executionは`Succeeded`のまま維持される

## B-012: 停止失敗でabortの確定結果を覆さない

GIVEN activeなsession node executionがあり、そのnodeのproviderプロセスの停止が失敗する
WHEN 利用者がworkflow executionをabortする
THEN abortは成功として返る
AND node executionは`Aborted`のまま維持される

## B-013: provider lifecycleの記録失敗で受理済みStopを覆さない

GIVEN provider Stopによるnode executionの状態が確定し、同じStopに伴うprovider lifecycleの記録だけが失敗する
WHEN provider Stopの受理結果を確認する
THEN provider Stopは成功として扱われる
AND node executionの確定した状態は維持される

## B-014: provider session identity未確定の停止済みsessionはResumeできない

GIVEN 終端したsession nodeのAgentSessionが`Paused`で保持されている
AND provider session identityが確定していない
WHEN 利用者がそのAgentSessionを開く
THEN 停止中のAgentSessionとして表示される
AND Resumeは提示されない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-003 |
| R-002 | B-004 |
| R-003 | B-005, B-006, B-007 |
| R-004 | B-008, B-009 |
| R-005 | B-010, B-011, B-012 |
| R-006 | B-013 |
| R-007 | B-014 |
