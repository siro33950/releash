# Milestone 84 実装順序

更新日: 2026-07-24

## Status

本書はmilestone 84「Agentチャット安定化」のIssue間の順序、依存関係、吸収済みIssueを定める。Phase名とIssue分類子は計画上のlabelであり、runtime module、type、table、physical store identityではない。

Phaseは0から7までの直列である。Phase NはPhase N-1までが完了してから開始し、同じPhase内のIssue同士には依存辺を置かない。

## 吸収済みIssue

| Issue | 統合先で保持する契約 |
| --- | --- |
| #1384（F2） | #1499 R-022 / B-104のcanonical message semanticsと境界 |
| #1385（F3） | #1499 R-013 / R-018の恒久SQLite storeとstartup contract |
| #1494（F9） | #1499 R-017のhistory-independentなbounded read |

吸収済みIssueは独立実装しない。#1499は固定pathの恒久SQLite storeを直接create / openし、legacy file-store互換機構を持たない。

## Phase allocation

| Phase | Issue | 役割 |
| --- | --- | --- |
| 0 | #1499 | command受理、terminal、recovery、Session lifecycle、shutdown、canonical message semantics、恒久SQLite authorityを一つのcontractとして完成させる |
| 1 | #1491（F8） | #1499を利用し、追加のbounded projectionとID-based queryでdetail取得の性能退行を解消する |
| 2 | #1386（F4）、#1387（F5）、#1446（F7）、#1497（F10）、#1413（P1）、#1521 | provider adapter、queue lifecycle、presentation、およびfrontend subscription lifecycleを拡張する |
| 3 | #1388（F6）、#1389（S1）、#1390（S2）、#1391（S3）、#1392（S4）、#1393（S5）、#1394（S6）、#1397（S9）、#1400（S11）、#1404（L3）、#1406（L5）、#1516 | safety機構とbackground activity / Workspace quiescenceを拡張する |
| 4 | #1395（S7）、#1396（S8）、#1399（S10b）、#1401（S12）、#1410（L9）、#1415（P3）、#1447（S13）、#1448（S14）、#1449（S15）、#1470（L13）、#1472（L14）、#1498（L15） | liveness、設定、capability、status presentationを拡張する |
| 5 | #1450（L12） | Phase 4までの設定・capability契約をworkflow、queue、restartへ継承する |
| 6 | #1412（L11）、#1451（P4） | 前段の完成contractを利用するlifecycleとpresentationを完成させる |
| 7 | #1416（T1） | Phase 0〜6のbackend / surface contractをcross-backend parity E2Eで統合検証する |

Phase 0の完了条件は、[Issue #1499 Primary Spec](../../docs/specs/issues-1499/requirements.md)、[close / quit decision table](close-quit-decision-table.md)、milestone 84の現行正本を満たすことである。吸収済みIssueを別途待たない。

## Hard dependencies

| Consumer | Predecessor |
| --- | --- |
| #1491 | #1499 |
| #1521 | #1491 |
| #1386 / #1387 | #1383、#1445 |
| #1516 | #1499、#1386、#1387 |
| #1410 / #1415 | #1516 |
| #1450 | Phase 4の設定・capability契約 |
| #1451 | #1450を含む前段contract |
| #1416 | Phase 0〜6の統合contract |

依存辺は常に小さいPhaseから大きいPhaseへ向く。後続Issueは先行contractを利用し、同義の暫定store、bridge、projection、authorityを再定義しない。

## 更新規則

- 新しい所見は、ownerとhard dependencyを確定してから既存Issueへの吸収または独立Issue化を決める。
- 吸収する場合は統合先のacceptanceへ含め、独立Phaseへ重複配置しない。
- 独立Issueは全predecessorより後の最初のPhaseへ置く。
- 同じPhase内に依存辺を作らず、必要ならconsumerを後のPhaseへ移す。
- 本書は順序とroutingだけを所有し、実装型、schema、処理順、test decompositionを定義しない。
