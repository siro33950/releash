# Agent チャット不安定性監査

- 調査日: 2026-07-07
- 最終整理日: 2026-07-24
- 調査基準: main `b3f9f54c` 付近の実装

本書は milestone 84 の問題、利用者影響、解消先を記録する監査台帳である。ここにある実装名、legacy JSON、当時の file-store、CLI version は historical evidence であり、現行 implementation contract ではない。

現行契約は次を正本とする。

- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md)
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md)
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md)
- [close-quit-decision-table.md](close-quit-decision-table.md)
- [Issue #1499 Primary Spec](../../docs/specs/issues-1499/requirements.md)
- [phase-plan.md](phase-plan.md)

## 監査の読み方

- Problem は調査時点で観測した欠落・分岐・構造要因の要約である。
- Impact は利用者または workflow に現れる結果である。
- Canonical resolution は現行正本上の解消先であり、実装方法や過去の物理 store を指定しない。
- 監査件数は CL 7、CX 11、SD 7、OB 8、RT 8、FE 7、RG 9、ST 9 の合計 66 件を維持する。

## 2026-07-19 追補所見

| Finding | Impact | Canonical resolution |
| --- | --- | --- |
| active-turn steer が write-ahead されない | response loss で入力消失または二重適用 | lifecycle I6、#1498 |
| terminal / list / lookup が全履歴に依存 | history growth で操作が遅くなる | #1499 R-017、#1491 |
| persistence work が history size に依存 | append 自体が劣化する | #1499 R-013 / R-017 |
| queue status read が全履歴に依存 | 小さな操作が重くなる | lifecycle I4、#1497 |
| dangling turn / fork recovery が不完全 | crash 後に未完了 state が残る | lifecycle I2 / I9、#1406 |
| Notice / feedback の owner が分裂 | 別 Session の failure 消失や unsafe text | vocabulary V-D4、presentation P4 |
| runtime owner と I/O boundary が混在 | lifecycle 修正の回帰が起きやすい | Rust-owned boundary、#1412 |
| parent turn と background activity が混在 | workflow が workspace 安定前に進む | #1516 |

## #1499 統合所見

| Problem | Impact | Canonical resolution |
| --- | --- | --- |
| send の stable acceptance identity がない | response loss / retry で重複送信 | R-001〜R-003 |
| accepted queue input が restart で消える | 送信済み message が実行されない | R-001、R-010 |
| canonical mutation failure が warning だけで続行される | live / reload divergence | R-004〜R-005 |
| terminal、Stop、recovery、shutdown の authority が分裂 | 同じ operation に異なる結果 | R-013、R-020〜R-021 |
| shutdown summary と detail の正本が分裂 | save / verify / effect gate / public read が矛盾 | SQLite plan / ordered target rows と同じ revision だけを使う単一 shutdown aggregate |
| startup failure の public outcome がない | normal workbench が半端に開く、終了不能 | closedなRust startup outcome、pre-admission Tauri read、process-local Quit |
| 初回 create 残骸と既存 store 破損の区別がない | 永久 block または data loss | normal admission前のinitial-create evidenceとB-071 matrix |
| operation binding に複数の永続 identity scope が混在 | restart replay と idempotency scope が不安定 | singleton metadataが所有するimmutable installation identity |
| legacy migration bridge が現行契約に残る | 二重 authority と特殊 quit が復活する | SQLite-only startup、legacy 非参照 |

変更前の file-store data の厳密な非参照保証は #1499 R-018 / B-070 を正本とする。通常の SQLite schema evolution、configuration compatibility、watch subscription initialization はこの禁止対象ではない。

## CL: Claude input の欠落

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| CL-1 | permission 取消が domain に届かない | 失効した dialog を操作できる | lifecycle I7、presentation permission UX |
| CL-2 | control response の success / failure が反映されない | UI と provider state が乖離する | lifecycle I8 / I14 |
| CL-3 | turn result の理由と stats が潰れる | failure 原因と cost が見えない | vocabulary TurnResult / TokenUsage |
| CL-4 | stop reason が失われる | refusal 等を workflow が判断できない | vocabulary V-D7 |
| CL-5 | provider 初期化 warning が失われる | MCP / configuration failure が見えない | vocabulary Notice |
| CL-6 | tool result の非 text content が失われる | 画像結果を利用者が確認できない | vocabulary V-D2 |
| CL-7 | provider 起点の Plan state が同期されない | 表示 mode と実挙動がずれる | lifecycle I14 |

## CX: Codex input の欠落

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| CX-1 | question identity と answer semantics が失われる | 利用者回答が無回答になる | vocabulary V-D6 |
| CX-2 | elicitation に応答しない | 理由不明の turn hang | lifecycle I7 / I10 |
| CX-3 | reasoning が live / history に出ない | 長考が停止に見える | vocabulary Thinking、presentation P3 |
| CX-4 | token usage を正しく解釈できない | usage / workflow 集計が誤る | vocabulary V-D8 |
| CX-5 | plan / todo を破棄する | Agent の進行が見えない | vocabulary V-D3 |
| CX-6 | control error response を表示しない | 設定変更や Stop が成功に見える | lifecycle I8 / I14 |
| CX-7 | warning / reroute を破棄する | 挙動差の原因が見えない | vocabulary V-D4 |
| CX-8 | retryable error を terminal error にする | 成功 turn に failure が残る | vocabulary V-D5 |
| CX-9 | command discovery の source が不正 | slash command が常に空 | protocol compatibility |
| CX-10 | image / review / collab item を表示しない | 実行内容と結果が見えない | vocabulary ToolCall / Notice |
| CX-11 | web search query を固定文言に潰す | 検索根拠を監査できない | vocabulary V-D2 |

## SD: Backend 間の意味差

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| SD-1 | resume failure の回復が backend で異なる | Codex Session だけ恒久利用不能になる | lifecycle I9 |
| SD-2 | Stop の受理と fallback が異なる | provider により止まらない | lifecycle I5 |
| SD-3 | malformed / oversized output の扱いが異なる | 一方だけ Session が突然終了する | lifecycle I10 |
| SD-4 | liveness signal が異なる | 正常な長考を stall と誤判定する | lifecycle I11 |
| SD-5 | tool output と completion の意味が異なる | 進行中 tool を完了表示する | vocabulary V-D2 |
| SD-6 | permission の表示情報が異なる | raw data と tool 名が不一致になる | vocabulary V-D6 |
| SD-7 | compaction failure の終端が異なる | 進行表示が残り続ける | vocabulary Notice、lifecycle I2 |

## OB: Outbound input の喪失

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| OB-1 | early Stop が無言で失われる | 再度 Stop できず実行が続く | lifecycle I5 |
| OB-2 | send failure 前に composer を clear する | 本文と添付が失われる | lifecycle I6、presentation P5 |
| OB-3 | queue が memory-only | restart / close で送信済み input が消える | lifecycle I4 |
| OB-4 | cancelled queue message の意味が残らない | reload で通常 message として復活する | lifecycle L-D4 |
| OB-5 | Stop 後に queue を自動 drain する | 止めた直後に次の作業が始まる | lifecycle L-D5 |
| OB-6 | queue start failure の着地点がない | queue が理由なく停止する | lifecycle I4 / I8 |
| OB-7 | image-only send の backend semantics が違う | 一方だけ送信失敗する | vocabulary MessagePart |
| OB-8 | resume recovery で editor context が落ちる | 再試行 turn の判断材料が欠ける | lifecycle I6 / I9 |

## RT: Runtime から read model までの喪失

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| RT-1 | close / quit が finalization を共有しない | parts、permission、tool が未完了のまま残る | lifecycle I1 |
| RT-2 | crash recovery が dangling turn を閉じない | spinner / permission 残骸が残る | lifecycle I2 / I3 |
| RT-3 | queued turn と human message の lifecycle が分裂 | 返信されない message が残る | lifecycle I4 |
| RT-4 | persistence crash 後の自己回復がない | その Session の全 mutation が失敗する | #1499 R-013 / R-018 |
| RT-5 | workflow terminal に failure reason が届かない | workflow 判断材料が欠ける | vocabulary V-D7 |
| RT-6 | Idle failure が durable surface に届かない | 理由不明の Error になる | lifecycle I12 |
| RT-7 | queue recovery failure を握りつぶす | queue が無言停止する | lifecycle I8 |
| RT-8 | partial projection が完全な parts を上書きする | 保存済み本文が欠落する | lifecycle I3 / L-P6 |

## FE: Presentation の不整合

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| FE-1 | cancelled permission が live では操作可能 | live / reload で dialog が違う | presentation P1 / permission UX |
| FE-2 | crash error が reload 後だけ見える | live では無言停止に見える | presentation P3 |
| FE-3 | hydration と delta の gap を修復しない | 本文が画面上で欠ける | presentation P1 |
| FE-4 | usage を表示しない | context / cost を判断できない | presentation S7 |
| FE-5 | failure banner が Session-scoped でない | 別 Session で消える・混ざる | presentation P4 |
| FE-6 | Task child content を描画しない | subagent の判断材料が欠ける | presentation Tool / Task |
| FE-7 | permission decision reason を表示しない | 拒否理由を監査できない | presentation permission UX |

## RG: Vocabulary gap

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| RG-1 | Codex reasoning の意味が配線されない | thinking が不可視 | vocabulary Thinking |
| RG-2 | Codex plan / todo の意味が配線されない | 進行が不可視 | vocabulary V-D3 |
| RG-3 | stop reason が一級でない | 拒否・上限・取消を区別できない | vocabulary V-D7 |
| RG-4 | tool result が success / error の二値 | denied / timeout / interrupt を区別できない | vocabulary V-D2 |
| RG-5 | todo が completed / not completed の二値 | current work と priority が消える | vocabulary V-D3 |
| RG-6 | operational Notice の受け皿がない | warning / rate limit が消える | vocabulary V-D4 |
| RG-7 | image tool result を保持しない | 承認判断の材料が欠ける | vocabulary V-D2 |
| RG-8 | command exit status を構造化しない | 失敗原因を判断しにくい | vocabulary V-D2 |
| RG-9 | cost を usage に含めない | workflow cost を確認できない | vocabulary V-D8 |

## ST: 構造要因

| ID | Problem | Impact | Canonical resolution |
| --- | --- | --- | --- |
| ST-1 | Codex wire が型安全な contract boundary を持たない | 新 notification / field drift を検出できない | vocabulary V-D12a |
| ST-2 | Claude wire が型安全な contract boundary を持たない | 未知 message の無言破棄を検出できない | vocabulary V-D12b |
| ST-3 | runtime owner と transition が一箇所に混在 | lifecycle 修正の考慮漏れが起きる | Rust owner boundary、#1412 |
| ST-4 | persistence failure を握りつぶす | memory と durable state が乖離する | lifecycle L-P3 |
| ST-5 | lock ownership が不明確 | 将来 deadlock と停止を招く | lifecycle I13 |
| ST-6 | MessagePart が複数定義 | 語彙拡張が片側だけになる | vocabulary V-D1、#1499 R-022 |
| ST-7 | fixture / parity coverage が不足 | provider update の退行を検出できない | #1499 R-012 |
| ST-8 | frontend が domain state を再構築 | live / reload 差が繰り返される | presentation frontend boundary |
| ST-9 | invisible wait の診断が限定的 | 停止原因の発見が遅れる | lifecycle I11 / I12 |

## 相互参照

| Root cause | Related findings | Canonical owner |
| --- | --- | --- |
| reasoning / plan input loss | CX-3、CX-5、RG-1、RG-2、SD-4 | vocabulary、provider adapter |
| permission response / cancellation loss | CL-1、CX-1、CX-2、FE-1、FE-7 | vocabulary V-D6、lifecycle I7 |
| queue durability / pause | OB-3〜OB-6、RT-3、RT-7 | lifecycle I4 / I5 |
| terminal / crash divergence | RT-1、RT-2、RT-8、FE-2 | lifecycle I1〜I3 |
| operation authority split | #1499 統合所見 | #1499 R-001〜R-021 |
| physical legacy contract | #1499 統合所見 | fixed SQLite authority、legacy 非参照 |

## 検証で却下した historical candidates

以下は調査時に候補となったが、記載された利用者影響を実コードから立証できなかったため active requirement にしない。

| Candidate | Rejection summary |
| --- | --- |
| stream delta suppression が必ず本文を恒久消失させる | 保存経路が別にあり、提示された因果が成立しない |
| Claude user text block drop が built-in command 出力を消す | 実測した output channel が異なる |
| token usage 更新頻度差が表示を freeze する | 調査時 UI に表示自体がなかった |
| serverRequest/resolved drop が長時間 dead dialog を作る | 外部解決経路が turn 境界に限られていた |
| permission mode authority が backend ごとに恒常的に異なる | 一部の前提と持続時間が不正確だった |
| backend 間 token usage の値意味が記載どおり異なる | Codex 側の実際の問題は decode failure だった |
| token usage が reload で消えることが画面上の退行になる | 調査時 UI に表示がなく user-visible でなかった |
| complete assistant message drop が既知 suppression と必ず恒久欠損を作る | 提示された persistence 因果が成立しない |

## 現行方針

- audit の legacy JSON、file-store、旧 CLI version、過去 code location は historical evidence である。
- active contract は fixed SQLite authority、Rust-owned lifecycle、backend-owned read model である。
- 旧 file-store はhistorical evidenceとしてのみ扱い、現行runtime contractに取り込まない。
- shutdownの旧page / ref / root / hash表現と、store / generation / app-data generation identityは削除対象を特定するhistorical evidenceであり、現行authorityまたはschema契約ではない。
- OPEN 事項はない。
