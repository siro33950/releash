# Milestone 84 実装順序

更新日: 2026-07-30

## Status

本書はmilestone 84「Agentチャット安定化」のIssue間の順序、依存関係、吸収済みIssue、および各Issueが解消する問題を定める。Phase名とIssue分類子は計画上のlabelであり、runtime module、type、table、physical store identityではない。

Phaseは0から8までの直列である。Phase NはPhase N-1までが完了してから開始し、同じPhase内のIssue同士には依存辺を置かない。同一Phase内で同時に着手できる組み合わせは、契約依存とは別にコード接触に基づく実装レーン（後述）が定める。現在はPhase 2まで完了しており、Phase 3以降は着手可能である。

各Issueが解消する問題の正本は[agent-chat-instability-audit.md](agent-chat-instability-audit.md)（調査基準 `be37b7d2e`）であり、本書のIssue対応と台帳のownerは双方向に一致する。

## 吸収済みIssue

| Issue | 統合先で保持する契約 |
| --- | --- |
| #1384（F2） | #1499 R-022 / B-104のcanonical message semanticsと境界 |
| #1385（F3） | #1499 R-013 / R-018の恒久SQLite storeとstartup contract |
| #1494（F9） | #1499 R-017のhistory-independentなbounded read |
| #1412（L11） | #1561の3分解（遷移・受理=domain集約、駆動=usecase orchestration、外部接触=gateway / infrastructure）とlock owner契約（ST-3は台帳の解消済みを参照） |

吸収済みIssueは独立実装しない。#1499は固定pathの恒久SQLite storeを直接create / openし、legacy file-store互換機構を持たない。

## Phase allocation

| Phase | Issue | 役割 |
| --- | --- | --- |
| 0 | #1499 | command受理、terminal、recovery、Session lifecycle、shutdown、canonical message semantics、恒久SQLite authorityを一つのcontractとして完成させる |
| 1 | #1491（F8） | #1499を利用し、追加のbounded projectionとID-based queryでdetail取得の性能退行を解消する |
| 2 | #1561 | session ライフサイクルのDomain整理（集約確立・usecase 3分解）と、audit台帳・本書のrouting再構築 |
| 3 | #1386（F4）、#1387（F5）、#1446（F7）、#1497（F10）、#1413（P1）、#1521、#1525、#1526、#1529、#1555、#1556、#1562、#1571、#1572、#1573 | provider adapter、設定domain、presentation、frontend subscription lifecycleを拡張し、storage・commit / projection 整合の保全を修復する |
| 4 | #1388（F6）、#1389（S1）、#1390（S2）、#1391（S3）、#1392（S4）、#1393（S5）、#1394（S6）、#1397（S9）、#1400（S11）、#1404（L3）、#1406（L5）、#1516 | semantic domain語彙、queue / recovery、background activity / Workspace quiescenceを拡張する |
| 5 | #1395（S7）、#1396（S8）、#1399（S10b）、#1401（S12）、#1410（L9）、#1415（P3）、#1447（S13）、#1448（S14）、#1449（S15）、#1470（L13）、#1472（L14）、#1498（L15） | protocol拡張、診断、activity表示、Agent設定、provider runtime healthを拡張する |
| 6 | #1450（L12） | Phase 5までの設定・capability契約をworkflow、queue、restartへ継承する |
| 7 | #1451（P4） | 前段の完成contractを利用するpresentationを完成させる |
| 8 | #1416（T1） | Phase 0〜7のbackend / surface contractをcross-backend parity E2Eで統合検証する |

Phase 0の完了条件は、[Issue #1499 Primary Spec](../../docs/specs/issues-1499/requirements.md)、[close / quit decision table](close-quit-decision-table.md)、milestone 84の現行正本を満たすことである。吸収済みIssueを別途待たない。

## Issueと問題の対応

Phase 3以降の各Issueが解消する問題である。ID付きの値は台帳の「現存する問題」のownerと双方向に一致する。IDを持たない行は、Issueが所有する契約が解消対象である。

| Issue | Phase | 解消する問題 |
| --- | --- | --- |
| #1386（F4） | 3 | ST-1、CX-9 |
| #1387（F5） | 3 | ST-2 |
| #1446（F7） | 3 | —（設定domain基盤: lifecycle I14〜I16、vocabulary V-D10〜V-D12。S9 / S13〜S15 / L12 / P4のpredecessor） |
| #1497（F10） | 3 | —（スコープはstale projectionのfail-closed（`ProjectionBehind`）の要否確認のみ。queue pauseの読出しはsession projectionのbounded readであり、fail-closedが不要と確認できればcloseする） |
| #1413（P1） | 3 | FE-3、ST-8 |
| #1521 | 3 | —（Workspace / Node選択lifecycleのfrontend増幅解消。#1491のbounded query / snapshot identityを消費） |
| #1525 | 3 | NF-003 |
| #1526 | 3 | NF-004 |
| #1529 | 3 | NF-005 |
| #1555 | 3 | NF-006 |
| #1556 | 3 | NF-007 |
| #1562 | 3 | NF-008 |
| #1571 | 3 | NF-015 |
| #1572 | 3 | NF-016 |
| #1573 | 3 | RT-8 |
| #1388（F6） | 4 | SD-5、RG-4、RG-7、RG-8、CL-6 |
| #1389（S1） | 4 | CX-3、RG-1、SD-4 |
| #1390（S2） | 4 | CX-5、RG-2、RG-5 |
| #1391（S3） | 4 | RG-9、FE-4 |
| #1392（S4） | 4 | CL-3、CL-4、RG-3、RT-5、NF-011 |
| #1393（S5） | 4 | CL-5、CX-7、RG-6、SD-7 |
| #1394（S6） | 4 | CL-1、CX-1 |
| #1397（S9） | 4 | CL-2、CX-6 |
| #1400（S11） | 4 | CL-7 |
| #1404（L3） | 4 | OB-4、NF-012（スコープはdurable cancel・rebase / retry・NeedsResolution表現・queued claimのcommit回復。enqueueの永続化と再起動復元は#1499の契約） |
| #1406（L5） | 4 | RT-2 |
| #1516 | 4 | NF-010 |
| #1395（S7） | 5 | CX-2 |
| #1396（S8） | 5 | FE-7、SD-6 |
| #1399（S10b） | 5 | CX-8 |
| #1401（S12） | 5 | CX-10、CX-11 |
| #1410（L9） | 5 | ST-9 |
| #1415（P3） | 5 | FE-6 |
| #1447（S13） | 5 | —（5 Agent modeのcross-backend mapping契約: V-D10、I14） |
| #1448（S14） | 5 | —（ReasoningEffortのcross-backend契約: I16） |
| #1449（S15） | 5 | —（Agent Goalのlifecycle契約: I15） |
| #1470（L13） | 5 | NF-001 |
| #1472（L14） | 5 | NF-002 |
| #1498（L15） | 5 | NF-009 |
| #1450（L12） | 6 | —（設定・Goal・capability契約のworkflow / queue / restartへの継承: I4 / I9 / I14〜I16） |
| #1451（P4） | 7 | —（S9a / S9b / S9cのUI完成） |
| #1416（T1） | 8 | ST-7 |

## Phase内の実装レーン（コード接触）

同一Phase内のIssueは契約依存を持たないが、修正対象コードが重なるものは並列に実装できない。本節はコード接触に基づく並列不可集合（レーン）を定める。同一レーン内のIssueは直列に実装し（順序はレーン内で自由。明示の先頭固定がある場合を除く）、レーン間は並列に着手できる。粒度は保守的（ファイル接触があれば同一レーン）とする。

### Phase 3（6レーン）

| レーン | Issue | 共有コード |
| --- | --- | --- |
| W1 | #1386 | infrastructure/codex と gateway/codex（wire全面置換） |
| W2 | #1387 | infrastructure/claude と gateway/claude（同） |
| S | #1525、#1526、#1529、#1555、#1556、#1562 | adaptor/gateway/local_event_store（state_record_codec / commit / reader / store / projection_record_codec を横断共有） |
| C | #1571、#1572、#1573 | usecase/agent_session/session/store（repository_core / persistence / event_projection）と runtime の streaming・event_dispatch |
| F | #1413、#1521 | frontend の session 購読・読込 hooks（useAgentChat ほか） |
| D | #1446 | 設定 domain の新設中心。他レーンとの接触は session projection の additive 拡張のみ |

#1497 はスコープ要否の確認のみでレーンに属さない。

### Phase 4（3レーン）

| レーン | Issue | 共有コード |
| --- | --- | --- |
| 変換 | #1388（先頭固定）、#1389、#1390、#1391、#1392、#1393、#1394、#1400 | domain の part / turn / todo / usage / notice / permission 語彙、event projector、gateway claude・codex の convert、frontend レンダラ。#1388（ToolCall統合）が全件とファイルを共有するため先頭固定 |
| queue / recovery | #1404、#1406 | operation の send / recovery、runtime の queue_driver・recovery |
| runtime / 設定 | #1397、#1516 | runtime の driver・event_dispatch（設定 ack と activity 配線が接触） |

### Phase 5（7レーン）

| レーン | Issue | 共有コード |
| --- | --- | --- |
| permission | #1395、#1396 | permission 往復（operation の permission、PermissionDialog） |
| codex 変換 | #1399、#1401、#1472 | gateway/codex の convert・session |
| 設定 | #1447、#1448、#1449 | 設定 domain / capability（相互接触） |
| claude runtime | #1470 | gateway/claude の session・process 健全性 |
| 診断 | #1410 | watchdog / stall 診断 |
| task 表示 | #1415 | frontend の ActivityLog / Task 表示 |
| steer | #1498 | operation send の steer write-ahead |

Phase 6〜8 は各1件でレーン分割はない。

## Hard dependencies

| Consumer | Predecessor |
| --- | --- |
| #1491 | #1499 |
| #1561 | #1491 |
| #1521 | #1491 |
| #1525 / #1526 / #1529 / #1555 / #1556 / #1562 / #1571 / #1572 / #1573 | #1499 |
| #1386 / #1387 | #1383、#1445 |
| #1516 | #1499、#1386、#1387 |
| #1404 | #1499、#1497 |
| #1410 / #1415 | #1516 |
| #1470 | #1387、#1392、#1393 |
| #1472 | #1386、#1388、#1392、#1393 |
| #1450 | Phase 5の設定・capability契約 |
| #1451 | #1450を含む前段contract |
| #1416 | Phase 0〜7の統合contract |

依存辺は常に小さいPhaseから大きいPhaseへ向く。後続Issueは先行contractを利用し、同義の暫定store、bridge、projection、authorityを再定義しない。

## 更新規則

- 新しい所見は、ownerとhard dependencyを確定してから既存Issueへの吸収または独立Issue化を決める。
- 吸収する場合は統合先のacceptanceへ含め、独立Phaseへ重複配置しない。
- 独立Issueは全predecessorより後の最初のPhaseへ置く。
- 同じPhase内に依存辺を作らず、必要ならconsumerを後のPhaseへ移す。
- 実装レーンは修正対象コードの接触に基づく運用情報であり、Issueの対象コードが変わったら更新する。
- 本書は順序とroutingだけを所有し、実装型、schema、処理順、test decompositionを定義しない。
