# 恒久 Local Event Store 設計の統合記録

## Status

旧D3は「#1499で暫定file-store closureを作り、F3 #1385でSQLiteへ移す」二段構成を定義していた。この構成は2026-07-21の方針変更で廃止した。

F2 #1384とF3 #1385は#1499へ統合し、実装正本を次のPrimary Specへ一本化する。

- [requirements.md](../../docs/specs/issues-1499/requirements.md) R-013 / R-018 / R-022
- [behavior.md](../../docs/specs/issues-1499/behavior.md) B-050 / B-070 / B-071 / B-098 / B-104
- [design.md](../../docs/specs/issues-1499/design.md)

本書は統合判断と正本へのroutingだけを保持する。D3、F2、F3、Phase 0をruntime module、type、table、migration generationの名前に使わない。旧D3本文にあった暫定manifest、file materializer、Phase 0→F3 import、future SQLite gateは実装しない。

## Integrated decision

#1499は次を同じreleaseで実装する。

1. domain layerの一つの`MessagePart`。
2. domain-owned `AgentSessionDomainEvent` / `WorkflowDomainEvent`。
3. gateway-owned versioned persistence DTOとlegacy JSON compatibility。
4. `LocalEventTransactionRepository::commit_batch`を唯一のmutation portとするmulti-stream transaction。
5. bundled SQLiteを唯一の正常稼働authorityとするlocal event store。
6. operation binding、terminal、obligation、pending recovery、shutdownを同transactionへ参加させるdirect / bounded index。
7. Legacyからstaging SQLiteへのcrash-resumable one-shot migration。
8. parity後の`Legacy → Sqlite` authority pointer CAS。
9. cutover後のSQLite-only read / write。

導入順は実装タスク上の依存であり、別runtime phaseではない。

```text
canonical domain types
        ↓
SQLite transaction port / schema
        ↓
send・terminal・recovery・shutdown closure
        ↓
legacy migration / production cutover
        ↓
surface parity / fault matrix
```

## Authority boundary

| Concern | Owner |
| --- | --- |
| message part / domain event semantics | domain |
| command / recovery / shutdown orchestration | usecase |
| SQLite schema / transaction / persistence codec / legacy import | adaptor gateway |
| Tauri / WebSocket DTO | adaptor protocol / presenter |
| provider / workflow / native exit effect | infrastructure port implementation |
| display / caller attempt mirror | frontend |

domain / usecaseはserde、SQL、rusqlite、filesystem layout、Tauri / WebSocket DTOへ依存しない。gatewayはdomain eventの意味を決めず、codecとstorageだけを所有する。

## Store invariants

- SQLite COMMITだけが通常mutationのcommit pointである。
- 一batchは複数streamとstate participantを同じtransactionへ含める。
- stream head CAS、idempotency binding、terminal unique keyをtransaction内で検証する。
- commit結果不明は同じcommit / operation identityの`OutcomeUnknown`で解決する。
- eventと#1499に必要なdirect / bounded projectionを同transactionで更新する。
- global / stream sequenceは1..=`i64::MAX`、revision / epoch / ordinal / countは0..=`i64::MAX`である。
- unknown additive payloadはraw-preserveし、unknown required semantic versionはfail closedにする。
- startup normal path、identity lookup、pending first pageは全event / 全session scanへfallbackしない。
- legacy dual write、record単位fallback、cutover後rollbackを禁止する。

具体schema、queue / page上限、SQLite PRAGMA、transaction順、migration checkpoint、shutdown detail compactionは[Issue #1499 design](../../docs/specs/issues-1499/design.md)を参照する。本書で別値を定義しない。

## Migration boundary

Legacyはmigration sourceとmigration中のread-only表示にだけ使う。source inventoryを固定し、bounded batchでstaging SQLiteへ変換し、次を照合する。

- record count
- session / workflow public projection
- known event result
- terminal / permission / queue
- operation / owner relation
- pending work / shutdown detail
- unknown additive raw bytes

照合できない場合は`MigrationBlocked`でauthorityを切り替えない。成功時だけpointerをLegacyからSqliteへ一度切り替える。pointer結果不明はfresh readbackで解決し、Sqlite確認後はlegacyへ戻らない。

migration中quitはnormal shutdown planを作らず、同じbackend quit operation identityとmigration checkpointをdurableに保持して15秒以内にexitを決定する。再起動後は同じ`MigrationApplicationQuitProjection`を返す。

## Downstream boundary

次は統合しない。

- F4 #1386 / F5 #1387: provider wire全体のtyped adapter化
- F8 #1491: #1499に不要な追加bounded query / read model
- F10 #1497: queue cancel / rebase / drainを含むlifecycle全体
- managed backup / restore public command
- privacy purge、app-data reset、export / import public lifecycle

#1499に必要なproduction `apply_runtime_event` path、identity lookup、pending recovery、terminal、shutdown projectionはdownstreamへ残さない。

## Verification gate

実装完了には次をすべて要求する。

- legacy JSON / SQLite / public DTOのMessagePart compatibility
- Claude / Codex F1b production composition golden
- multi-stream transaction fault matrix
- same-key replay / conflict / principal separation
- terminal / Stop / recovery / session lifecycle / shutdown fault matrix
- Legacy→SQLite migrationのcheckpoint / pointer crash matrix
- cutover後のlegacy read / write / fallback 0件
- Tauri / WebSocket semantic parity
- 10件 / 1,000,000件fixtureのbounded query budget

Issue本文、マイルストーン配分、GitHub上のF2 / F3管理状態はこの設計統合とは別に更新できる。実装WorkflowはPrimary Specを入力とし、本書から旧二段構成を再導入しない。
