# 恒久 Local Event Store 設計の統合記録

更新日: 2026-07-24

## Status

本書は、旧D3とF2 / F3の計画をIssue #1499へ統合した判断と、現行正本へのroutingだけを保持する。これらの計画名とPhase 0はruntime module、type、table、physical store identityに使わない。

現行契約の正本は次のとおり。

- [requirements.md](../../docs/specs/issues-1499/requirements.md) R-013 / R-018 / R-022
- [behavior.md](../../docs/specs/issues-1499/behavior.md) B-050 / B-070 / B-071 / B-098 / B-104
- [design.md](../../docs/specs/issues-1499/design.md)

## 統合した決定

- domain event、command、terminal、obligation、recovery、shutdownを、一つの恒久local event storeで整合させる。
- 固定pathのSQLite storeを直接create / openし、runtimeの唯一のread/write authorityとする。
- SQLiteの通常のschema evolutionはstartup contractの一部であり、legacy data migrationとは区別する。
- 変更前のfile-storeは互換対象にせず、#1499 R-018 / B-070の非参照保証に従う。
- legacy data用の移行・互換・切替機構や同等の別名機構を作らない。
- 廃止するbootstrapはlegacy-data / 旧Phase 0のphysical store bootstrapに限る。watch subscription、provider connection、configurationの通常initializationで使うbootstrap語彙と処理は別概念であり、#1499の禁止対象にしない。
- Rustがdomain semantics、application orchestration、persistence authorityを所有し、frontendは表示と入力に限定する。
- operation、terminal、obligation、recovery、shutdownの確定は、同じ意味的transaction boundaryに参加する。
- normal admission後のpublic surfaceはTauriとWebSocketで同じ意味を返し、保存形式や内部処理順を公開契約にしない。pre-admissionのstartup failureはPrimary DesignどおりTauriのsafe surfaceだけを使い、WebSocket serverを起動しない。
- store / generation / app-data generationを別々の永続identityにせず、Primary Designのimmutable installation identityだけをoperation binding、HMAC、idempotency、obligation correlationのdomain separationに使う。
- shutdownの保存、検証、effect gate、current / history read、target paginationはPrimary DesignのSQLite plan / ordered target rowsへ統合し、旧page / ref / root / hash表現をauthorityにしない。
- 初回createの安全な再試行はPrimary Designのinitial-create evidenceでnormal admission前であることを証明し、既存の空fileだけから再初期化可能と推測しない。

## 後続Issueとの境界

- F4 #1386 / F5 #1387: provider wire全体のtyped adapter化
- F8 #1491: #1499の完了に不要な追加queryとread model
- F10 #1497: queue cancel、rebase、drainを含むlifecycle全体
- managed backup / restore、privacy purge、app-data reset、export / importのpublic lifecycle

これらの後続は#1499のauthorityを再定義せず、同義の暫定storeやcompatibility bridgeを作らない。

## Verification routing

実装のacceptanceはPrimary SpecのBehaviorを使う。B-050は恒久保存、B-070はproduction lifecycle全体でのlegacy非参照、B-071はstartupと初回作成再開、B-098はschema evolution、B-104はcanonical message semanticsを検証する。owner境界と実装方針はPrimary Designを正本とし、本書では再定義しない。
