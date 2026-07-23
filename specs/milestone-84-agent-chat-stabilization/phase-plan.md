# Milestone 84 実装順序

## Status

本書はmilestone 84「Agentチャット安定化」の実装順序、依存関係、吸収済みIssueを定める正本である。GitHub milestone descriptionと各Issueの「位置づけ」は本書に一致させる。

Phaseは0から7までの直列であり、subphaseは存在しない。Phase NはPhase N-1までが完了してから開始する。同じPhaseに置くIssue同士には依存辺を持たせない。Issue名に含まれるF / S / L / P / Tなどの記号は既存の分類子であり、Phaseや並列groupを表さない。

## 読み方

- 「Phaseに含む」は、そのPhaseでcloseする独立Issueを表す。
- 「吸収済み」は、別Issueとして実装せず、統合先Issueのacceptanceへ含めることを表す。吸収済みIssueをPhaseへ重複配置しない。
- 後続Issueは先行Issueのpublic contractとproduction codeを利用し、同義の暫定store、bridge、projection、authorityを作らない。
- Phase内の実装着手順は定めない。直列依存が判明した場合は同じPhase内で順番を付けず、consumerを次のPhase以降へ移す。
- GitHubのIssue本文に古いPhaseや吸収前Issueへの依存が残る場合は、本書を正として本文を更新する。

## 吸収済みIssue

次のIssueは#1499のacceptanceへ統合済みであり、独立実装しない。

| Issue | #1499へ統合した責務 | 管理結果 |
| --- | --- | --- |
| #1384（F2） | canonical domain `MessagePart`、legacy / persistence / public DTO境界 | #1499 R-022 / B-104へ統合し、#1384はsupersededとしてclose済み |
| #1385（F3） | bundled SQLite local event store、multi-stream transaction、Legacy→SQLite cutover | #1499 R-013 / R-018へ統合し、#1385はsupersededとしてclose済み |
| #1494（F9） | append / terminal / recoveryのhistory independenceとquery-plan / latency gate | #1499 R-017へ統合し、#1494はsupersededとしてclose済み |

#1499の実装では、吸収元の記号や`phase0`をmodule、type、table、migration generationへ使わない。

## Phase 0

Phase 0は完了済み12 Issueの保証を#1499で恒久store上へ閉じる。個別の応急処置や後続bridgeへ分割しない。

完了済みbaseline:

- #1445（D1）
- #1383（F1）
- #1402（L1）
- #1403（L2）
- #1405（L4）
- #1407（L6）
- #1408（L7）
- #1409（L8）
- #1411（L10）
- #1398（S10a）
- #1414（P2）
- #1417（X1）

Phase 0で実装するIssue:

- #1499 — command受理、terminal、recovery、session lifecycle、shutdown、canonical message type、恒久SQLite store、history-independent closureを一つのauthorityで完了する。

Phase 0の完了条件は#1499 Primary Spec、close / quit decision table、統合fault matrixをすべて満たすことである。#1384、#1385、#1494を別途待たない。

## Phase 1

- #1491（F8）— Node / Session detail等が全Session eventを再読込するbackend性能退行を、bounded projection、ID-based query、query-plan gateで解消する。

#1491は#1499の恒久storeとtransaction内projectionを前提とする。利用者影響が大きいため#1499直後に単独Phaseで実施し、他のPhase 2項目を待たない。

## Phase 2

- #1386（F4）
- #1387（F5）
- #1446（F7）
- #1497（F10）
- #1413（P1）
- #1521 — Workspace切替時のsubtree remount、listener再登録、cached detail再取得、event burst refreshというfrontend lifecycle増幅を解消する。

#1521は#1491のbackend queryを利用するconsumerである。責務は重複しない。

- #1491はRust / SQLite側の全event replayとunbounded queryを除去する。
- #1521はfrontend側のWorkspace subtree lifetime、subscription ownership、ID-based detail cacheを修正し、同じbounded queryの重複発行を防ぐ。

#1386 / #1387はF1 #1383のgolden基盤に加えて、D1 #1445が定義した`BackendProtocolIdentity` / capability semanticsをhard dependencyとする。型を再定義せず、#1386はCodex runtime identity / compatibilityとapproval・tool mutation interception capability、#1387はClaude binary identity / compatibilityとtask / live-set / permission・auto-allow mutation interception capabilityをpinする。

## Phase 3

- #1388（F6）
- #1389（S1）
- #1390（S2）
- #1391（S3）
- #1392（S4）
- #1393（S5）
- #1394（S6）
- #1397（S9）
- #1400（S11）
- #1404（L3）
- #1406（L5）
- #1516 — 親Turnの完了とbackground activityの継続を分離し、Workspaceの安定を必要とするWorkflow処理だけをquiescenceで待機させる。

#1516のhard dependencyは#1499、#1386、#1387である。

- #1499は恒久SQLite transactionと、Turn完了後も別domain eventを追記できる基盤を提供する。
- #1386と#1387は、実行中backendのprotocol identity、capability、typed wireを提供する。
- #1516はbackground activityのdurable stateとWorkspace quiescenceを所有する。provider wireの再定義、通常chatの一律停止、親Turn resultの遡及変更は行わない。
- 詳細なstall診断とUIは後続#1410、#1415が#1516のprojectionを利用して実装する。

#1516はPhase 3の他Issueを前提にしない。他Issueも#1516を前提にしないため、Phase 3内に依存辺はない。

## Phase 4

- #1395（S7）
- #1396（S8）
- #1399（S10b）
- #1401（S12）
- #1410（L9）
- #1415（P3）
- #1447（S13）
- #1448（S14）
- #1449（S15）
- #1470（L13）
- #1472（L14）
- #1498（L15）

#1410と#1415は#1516をhard dependencyに追加する。

- #1410はbackground activityを含むliveness / stall診断を、Turn stateやtranscriptから推測せずdurable activity projectionから読む。
- #1415はtask / activity statusとWorkspace quiescenceをbackend-owned projectionから表示する。

両Issueは#1516のconsumerであり相互依存しない。他のPhase 4 Issueとも依存辺を持たない。

## Phase 5

- #1450（L12）

#1450はPhase 4で確定するS13 / S14 / S15等の設定・capability契約をworkflow / queue / restartへ継承する。

## Phase 6

- #1412（L11）
- #1451（P4）

#1412と#1451はPhase 5以前の完成contractをconsumerとし、相互依存しない。

## Phase 7

- #1416（T1）

#1416はPhase 0〜6のbackend / surface contractをcross-backend parity E2Eとして統合検証する最後のPhaseである。

## 依存辺の検証表

| Consumer | 必須predecessor | predecessor Phase | Consumer Phase |
| --- | --- | --- | --- |
| #1491 | #1499 | 0 | 1 |
| #1521 | #1491 | 1 | 2 |
| #1386 | #1383、#1445 | 0 | 2 |
| #1387 | #1383、#1445 | 0 | 2 |
| #1516 | #1499、#1386、#1387 | 0、2 | 3 |
| #1410 | #1516 | 3 | 4 |
| #1415 | #1516 | 3 | 4 |
| #1450 | Phase 4のS13 / S14 / S15 contract | 4 | 5 |
| #1451 | #1450を含む前段contract | 5以前 | 6 |
| #1416 | Phase 0〜6の統合contract | 0〜6 | 7 |

上表のすべての依存辺は小さいPhase番号から大きいPhase番号へ向く。同じPhase内の依存辺は0件である。

## 更新規則

新しい監査所見またはIssueをmilestone 84へ追加する場合は、次の順で更新する。

1. 正本上のownerとhard dependencyを確定する。
2. 既存Issueへ吸収する場合は吸収先のacceptanceと「吸収済みIssue」を更新し、独立Phaseへ置かない。
3. 独立Issueの場合は全predecessorより大きい最小Phaseへ置く。
4. 同じPhaseに依存辺が生じた場合はconsumer以降のPhase番号を増やし、subphaseやA / B / C suffixを作らない。
5. 本書、GitHub milestone description、対象Issue本文を同じ変更で一致させる。
