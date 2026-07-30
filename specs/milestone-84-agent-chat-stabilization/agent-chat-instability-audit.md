# Agent チャット不安定性監査

- 調査基準: branch `feature/1561` commit `be37b7d2e`
- 調査日: 2026-07-30

本書は調査基準時点の実装に対する不安定性の正であり、現存する問題、利用者影響、解消先 owner を記録する監査台帳である。ここにある実装名・file 位置は調査時点の historical evidence であり、現行 implementation contract ではない。

現行契約は次を正本とする。

- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md)
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md)
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md)
- [close-quit-decision-table.md](close-quit-decision-table.md)
- [Issue #1499 Primary Spec](../../docs/specs/issues-1499/requirements.md)
- [phase-plan.md](phase-plan.md)

## 監査の読み方

- 各 finding は stable ID を持つ。ID は意味を変えず、再利用しない。ID 空間は CL 7、CX 11、SD 7、OB 8、RT 8、FE 7、RG 9、ST 9（計 66）と NF 16 である。
- Status は次の2値に閉じる。

| Status | 定義 |
| --- | --- |
| 現存 | 利用者影響または保証 gap が調査基準の実装に存在し、解消先の owner Issue を持つ |
| 解消済み | 原因経路が調査基準の構造に存在せず、対応する behavior の検証が通る |

- 「現存する問題」の根拠列は現行の原因経路を、「解消済み」の根拠列は現行構造でその問題が成立しない理由を記す。
- 解消済みの behavior 共通根拠: 本更新時点で `cargo test` 3471 passed / 0 failed / 1 ignored。#1499 契約テスト（`issue_1499_contract_tests`）と Session 集約の遷移・受理・terminal・recovery 単体テストを含む。
- Owner は現存する問題の解消先 Issue（各 finding につき一つ）。[phase-plan.md](phase-plan.md) の Issue 対応と双方向に一致する。

## 現状サマリ

| Status | 件数 | 内訳 |
| --- | --- | --- |
| 現存 | 57 | CL 7、CX 10、SD 4、OB 1、RT 3、FE 4、RG 9、ST 5、NF 14 |
| 解消済み | 25 | CX-4、SD-1〜SD-3、OB-1〜OB-3、OB-5〜OB-8、RT-1、RT-3、RT-4、RT-6、RT-7、FE-1、FE-2、FE-5、ST-3〜ST-6、NF-013、NF-014 |

## 現存する問題

### CL: Claude input の欠落

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| CL-1 | permission 取消が domain に届かない | 失効した dialog を操作できる | wire に取消 message の定数・変換 arm がなく、provider 起点の取消は `gateway/agent_session/claude/convert.rs` の fallback で破棄される | #1394 |
| CL-2 | control response の success / failure が反映されない | UI と provider state が乖離する | control response の成否を読む変換がなく、set_model 等は provider ack 前に state を楽観更新する | #1397 |
| CL-3 | turn result の理由と stats が潰れる | failure 原因と cost が見えない | result 変換が subtype・duration・cost・num_turns を読まず Completed / Failed の二値へ潰す。domain 側にも保持先がない | #1392 |
| CL-4 | stop reason が失われる | refusal 等を workflow が判断できない | 変換が stop_reason: None を固定で渡す。`TurnStopReason` は Refusal 1 値の dead_code で production 経路に入らない | #1392 |
| CL-5 | provider 初期化 warning が失われる | MCP / configuration failure が見えない | system/init から session id と slash command のみ抽出し、MCP 状態・warning を破棄する。受け皿の Notice 語彙も存在しない（RG-6） | #1393 |
| CL-6 | tool result の非 text content が失われる | 画像結果を利用者が確認できない | tool result content の変換が text block のみ抽出し image block を無言で脱落させる | #1388 |
| CL-7 | provider 起点の Plan state が同期されない | 表示 mode と実挙動がずれる | permission mode 通知の変換は plan 値を None へ落とし、resync は保存済み mode の再放送に留まる | #1400 |

### CX: Codex input の欠落

installed codex-cli 0.145.0 の生成スキーマと現行変換を突合した結果である。

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| CX-1 | question identity と answer semantics が失われる | 利用者回答が無回答になる | 変換が question id / isSecret / isOther を捨て、現契約に存在しない field を読む。応答はキー・値形状とも契約（ToolRequestUserInputResponse）不一致 | #1394 |
| CX-2 | elicitation に応答しない | 理由不明の turn hang | server request の変換は approval 系 4 メソッドのみで、elicitation request へ応答を返す経路がない | #1395 |
| CX-3 | reasoning が live / history に出ない | 長考が停止に見える | reasoning delta 通知 3 種の変換 arm がなく、completed item の抽出も現契約（summary 配列）と不一致で常に失敗する | #1389 |
| CX-5 | plan / todo を破棄する | Agent の進行が見えない | turn/plan/updated 通知・plan item とも変換 arm がなく、TodoListSnapshot の生成経路は Claude 専用である | #1390 |
| CX-6 | control error response を表示しない | 設定変更や Stop が成功に見える | turn/interrupt の error response が log 止まりで利用者に見えない。設定変更は turn/start 載せ替えの設計でありその経路はない | #1397 |
| CX-7 | warning / reroute を破棄する | 挙動差の原因が見えない | warning / model rerouted / guardian・config warning / deprecation の変換 arm がなく全て破棄される | #1393 |
| CX-8 | retryable error を terminal error にする | 成功 turn に failure が残る | ErrorNotification の必須 field willRetry を読まず、無条件に恒久 Error part として記録する | #1399 |
| CX-9 | command discovery の source が不正 | slash command が常に空 | 抽出元の field は現契約のどの応答にも存在せず、一覧は常に空になる | #1386 |
| CX-10 | image / review / collab item を表示しない | 実行内容と結果が見えない | imageView / imageGeneration / review / collab / subAgentActivity の item が started / completed とも無変換 | #1401 |
| CX-11 | web search query を固定文言に潰す | 検索根拠を監査できない | query は ToolUse input に保持される。一方 completed の結果は固定文言のままで results / action を破棄する | #1401 |

### SD: Backend 間の意味差

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| SD-4 | liveness signal が異なる | 正常な長考を stall と誤判定する | watchdog と判定は共通だが入力が非対称。Codex は reasoning 中に progress 更新イベントを生成せず、閾値超の長考が stall と誤観測される | #1389 |
| SD-5 | tool output と completion の意味が異なる | 進行中 tool を完了表示する | Codex は実行中の outputDelta を ToolResult に変換し、消費側は「ToolResult あり = 完了」の単一解釈。fileChange は開始時点から完了表示になる | #1388 |
| SD-6 | permission の表示情報が異なる | raw data と tool 名が不一致になる | Question 系の domain 型は共通。ToolApproval は Codex が合成 tool 名 + params 全体で、transcript の tool 名と食い違い整形 UI も不適用 | #1396 |
| SD-7 | compaction failure の終端が異なる | 進行表示が残り続ける | Codex は失敗通知で終端する。Claude に失敗終端経路がなく、compaction 失敗後も in_progress 通知が残る | #1393 |

### OB: Outbound input の喪失

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| OB-4 | cancelled queue message の意味が残らない | reload で通常 message として復活する | cancel 操作は durable queue operation ができるまで常時エラーで無効化されており、L-D4（cancelled marker の保持と context 除外）の behavior が存在しない | #1404 |

### RT: Runtime から read model までの喪失

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| RT-2 | crash recovery が dangling turn を閉じない | spinner / permission 残骸が残る | pending obligation は fail-closed に fence され可視化されるが、crash 中断 turn の event log 終端を書く startup 経路がなく、未終端 turn が Active 表示・Pending permission・queue 保留として session close まで残る | #1406 |
| RT-5 | workflow terminal に failure reason が届かない | workflow 判断材料が欠ける | failure kind の型付き分類・exit code・interrupted は届くが、turn failure の理由文言が projection 変換で脱落し、node failure record が汎用文言になる | #1392 |
| RT-8 | partial projection が完全な parts を上書きする | 保存済み本文が欠落する | terminal 確定前に予約された遅延 stream flush が terminal 後に発火すると、persist 判定は経過時間のみで（terminal・lease の検査なし）、reset 済みの空 parts と初期 seq を確定済み message projection へ無条件上書きする。message projection はリロード表示の正本で、後続 send は旧 turn を bounding して project するため修復 commit が発生しない | #1573 |

### FE: Presentation の不整合

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| FE-3 | hydration と delta の gap を修復しない | 本文が画面上で欠ける | hydration snapshot は lock 内 publish で購読と順序付き、再読込と resync fallback を持つ。一方 seq 連続性の検証がなく（delta の seq 未使用）、欠落の検出・自己修復の保証がない | #1413 |
| FE-4 | usage を表示しない | context / cost を判断できない | backend からイベント・reducer・getter まで経路は存在するが、描画する component が存在しない | #1391 |
| FE-6 | Task child content を描画しない | subagent の判断材料が欠ける | tool child は描画されるが、thinking child は分岐で捨てられ、text child は 200 字 + 高さ制限で全文アクセスがない | #1415 |
| FE-7 | permission decision reason を表示しない | 拒否理由を監査できない | decision reason は backend から frontend 型まで到達するが、pending / resolved いずれの UI も描画しない | #1396 |

### RG: Vocabulary gap

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| RG-1 | Codex reasoning の意味が配線されない | thinking が不可視 | Thinking 語彙は存在するが Codex reasoning からの配線がない（CX-3 と同根） | #1389 |
| RG-2 | Codex plan / todo の意味が配線されない | 進行が不可視 | TodoListSnapshot への Codex 配線がない（CX-5 と同根） | #1390 |
| RG-3 | stop reason が一級でない | 拒否・上限・取消を区別できない | TurnStopReason は Refusal 1 値の dead_code で、上限系・取消の語彙も production 配線もない | #1392 |
| RG-4 | tool result が success / error の二値 | denied / timeout / interrupt を区別できない | ToolResult は content + is_error の二値である | #1388 |
| RG-5 | todo が completed / not completed の二値 | current work と priority が消える | TodoListItem は text + completed のみで in progress / priority を表現できない | #1390 |
| RG-6 | operational Notice の受け皿がない | warning / rate limit が消える | SystemNotificationType は Compaction / SessionRecovery の 2 種のみで、汎用 Notice 語彙がない | #1393 |
| RG-7 | image tool result を保持しない | 承認判断の材料が欠ける | Image part は dead_code で tool result からの生成経路がない（CL-6 と同根） | #1388 |
| RG-8 | command exit status を構造化しない | 失敗原因を判断しにくい | ToolOutputSummary は行数・byte 数・error flag・truncation のみで exit status を持たない | #1388 |
| RG-9 | cost を usage に含めない | workflow cost を確認できない | TokenUsage は input / output / total / context window のみで cost・cache 系がない | #1391 |

### ST: 構造要因

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| ST-1 | Codex wire が型安全な contract boundary を持たない | 新 notification / field drift を検出できない | wire は文字列定数 + 生 JSON 値で、契約検証は手動検証の doc 注記のみ。typed 検出がない | #1386 |
| ST-2 | Claude wire が型安全な contract boundary を持たない | 未知 message の無言破棄を検出できない | 生 JSON 値の取り出しベースで、未知 type は fallback 破棄される | #1387 |
| ST-7 | fixture / parity coverage が不足 | provider update の退行を検出できない | record / replay 基盤はあるが、fixture は両 backend とも normal turn 各 1 本のみで coverage が不足 | #1416 |
| ST-8 | frontend が domain state を再構築 | live / reload 差が繰り返される | domain state（turn / permission / error / status）は revision ガード付き mirror だが、activity label 導出・tool pairing / Task 完了推定・streaming text 結合が backend 規則との二重表現として frontend に残る | #1413 |
| ST-9 | invisible wait の診断が限定的 | 停止原因の発見が遅れる | 診断は WaitingPermission 60 秒の log warn のみで、phase 別の無進行診断・利用者可視の診断表示がない | #1410 |

### NF: Storage・provider runtime・lifecycle 保全

| ID | Problem | Impact | 根拠 | Owner |
| --- | --- | --- | --- | --- |
| NF-001 | Claude interrupt 後の poisoned runtime を検出せず同一 process を再利用する | 以後の通常メッセージにも無応答が続く | interrupt は fire-and-forget で control response の成否・process readiness を追跡しない。実セッションで再現確認済み | #1470 |
| NF-002 | Codex dynamic tool request（item/tool/call）へ応答しない | plugin 提案で turn が進行中のまま永久待機する | server request の変換は approval 系のみで、dynamic tool call へ応答を返す経路がない | #1472 |
| NF-003 | recovery の versioned encoding・hash 生成が usecase にあり typed resource が型消去される | schema drift を検出できず、commit 判断が文字列の JSON 再 parse に依存する | usecase が versioned JSON と SHA-256 を生成し構造化 resource を SafeSummary(String) へ格納する。canonicalizer port は存在するが recovery 経路は使用していない | #1525 |
| NF-004 | known persistence field の present-invalid が default 値へ読み替えられる | operation identity・effect owner・deadline が別 authority になる | codec の optional 読出しが field absent と present-invalid を区別しない。型不正な operation kind が正常 default へ補正されるケースを再現確認済み | #1526 |
| NF-005 | OperationRecord が receipt と status の矛盾を受理する | 矛盾 record が exact payload とは別の obligation を通常状態として公開できる | validate の relation 検査が部分的で、receipt と status の identity 不一致（turn / obligation）が write・reload 双方で受理される | #1529 |
| NF-006 | WAL がランタイム中にリセットされず無制限成長する | 実測 17.4GB/4.5h。外部切り詰めを誘発し起動不能に至る | 明示 checkpoint は open 時の 1 回のみで、runtime checkpoint・journal size limit・WAL 観測が存在しない | #1555 |
| NF-007 | projection 1 行の破損で一覧・復旧 query 全体が失敗し、隔離・再構築手段がない | 1 行の破損でアプリが実質使用不能になる | session projection の一覧・owner snapshot・pending 復旧 query が 1 行の decode 失敗で全体 Err になる。quarantine と events からの再構築経路がない。実障害で確認済み | #1556 |
| NF-008 | message projection の行全体リライトと activities 重複導出で書き込みが増幅する | 長い応答で書き込み量が O(L²) となり NF-006 の成長を増幅する | additive merge が全 parse + 全再 serialize で差分経路がなく、activities が parts から重複導出されて blob 外部化が無効化される | #1562 |
| NF-009 | active-turn steer が write-ahead されない | response loss で入力消失または二重適用 | steering 対応 backend では provider steer 成功後に human message を永続化する経路であり、provider 受理直後の切断・crash では input も intent も残らない。production backend は steering を広告していない | #1498 |
| NF-010 | parent turn と background activity が混在 | workflow が workspace 安定前に進む | provider Result で親 Turn が完了した後も background activity が継続し得るが、activity の durable inventory と terminal outcome がなく、Turn 完了だけを根拠に workflow が次の workspace 依存処理へ進む | #1516 |
| NF-011 | Stop deadline の Timeout terminal が workflow turn 完了 handoff を生成しない | workflow node が Running のまま進行せず、再起動時は orphan 中断へ劣化する | workflow handoff の生成は runtime terminal 経路の 1 箇所のみで、Stop の 10 秒 deadline 自己終端は handoff を含まない別 batch を commit する。exit 124 の interrupted terminal は handoff 要求条件（requires_workflow_turn_completion）を満たすが、live 通知も durable outbox entry の存在に依存するため何も配送されない | #1392 |
| NF-012 | queued turn claim が自分の commit 成功を Blocked と誤分類し provider effect を放棄する | durable には Active な turn が provider 未開始のまま残り（phantom turn）、queue 全体が停止して自己修復しない | claim commit の OutcomeUnknown に resolve_commit の失敗が重なると Err で抜け、retry が自己 commit 済み集約への start_queue_head の Rejected(NotQuiescent) を一括で Blocked へ写像して claim を破棄する。Blocked は log のみで、dispatch marker の park が recovery の再駆動も抑止する。immediate send 経路にある commit readback 防御が queued 経路にない | #1404 |
| NF-015 | session projection の snapshot commit が呼び出し元の読みからの不変を保証しない | 並行 commit（terminal・turn 開始・streaming）が巻き戻り、terminal 消失による session 停止や state_revision の逆行が起きる | rename と provider 確立記録は lock なしで projection を読み、commit は worker 内の再読の revision を guard に使うため、読みと commit の間に入った変更を検知せず読んだ時点の reducer_events / meta で上書きする。provider 確立記録は turn 開始直後に detached task で走るため並行 commit と恒常的に重なる | #1571 |
| NF-016 | close と in-flight terminal の競合が同一 turn の二重 terminal を受理する | 1 turn 1 terminal の不変条件が破れる。workflow session では存在しない terminal identity に束縛された pending obligation が復旧 query を恒久に失敗させ、effect admission を全面封鎖する | terminal writer は収束判定と mutation 構築を最初に一度だけ行い、retry loop は additional_mutations を clone で再利用して fresh 状態の上に commit する。TerminalRecord は ON CONFLICT DO NOTHING で黙って捨てられるが、イベントと workflow obligation は成功し、retry 成功経路に再収束判定がない | #1572 |

## 解消済み

原因経路が調査基準の構造に存在しない finding。ID は不変のまま保持する。根拠は現行構造でその問題が成立しない理由である。

| ID | Problem | Impact | 根拠 |
| --- | --- | --- | --- |
| CX-4 | token usage を正しく解釈できない | usage / workflow 集計が誤る | decode は現契約（tokenUsage.total.\* / modelContextWindow）と一致し、flat 形式の後方互換を持つ。0.145.0 実スキーマと突合して確認。cost / cache 系の語彙は RG-9 が所有 |
| SD-1 | resume failure の回復が backend で異なる | Codex Session だけ恒久利用不能になる | 両 backend の resume failure は共通の backend session recovery（identity クリア・context 再注入・accepted turn 引継ぎ）へ合流し、片側だけ回復不能になる経路がない |
| SD-2 | Stop の受理と fallback が異なる | provider により止まらない | Stop の受理保証は backend 非依存の Stop operation が持つ。provider interrupt は handoff にすぎず、10 秒 deadline で Timeout terminal を自前 commit し、共通の staged shutdown が process を終端する |
| SD-3 | malformed / oversized output の扱いが異なる | 一方だけ Session が突然終了する | 不正 JSON 行・oversized 行は共有の stdout line reader / diagnostics（8MB 上限、Json / NonJson / Oversize 分類）が処理し、両 backend とも Session を終了させず継続する |
| OB-1 | early Stop が無言で失われる | 再度 Stop できず実行が続く | Stop は durable operation + ProviderInterrupt obligation として単一 atomic batch で受理される。provider turn identity 確立前の interrupt は予約され確立後に送信、10 秒 deadline の Timeout terminal と startup recovery が喪失窓を残さない |
| OB-2 | send failure 前に composer を clear する | 本文と添付が失われる | composer clear は durable commit 済み Accepted receipt を boundary とし、受理前 rejection・結果不明では本文・添付を保持する |
| OB-3 | queue が memory-only | restart / close で送信済み input が消える | queue item は operation record + obligation（canonical payload）+ session projection として SQLite に atomic 永続化され、startup recovery が復元する。in-memory 側は mirror である |
| OB-5 | Stop 後に queue を自動 drain する | 止めた直後に次の作業が始まる | queue pause は Session 集約が所有し、Stop 受理と interrupted terminal の両方で durable に pause する。drain は pause 中抑止され、再開は明示 resume のみ |
| OB-6 | queue start failure の着地点がない | queue が理由なく停止する | claim 後失敗は error 付き Interrupted(Crash) terminal + queue pause、claim 前失敗は ReconciliationRequired へ durable に着地し、redriver と startup recovery が再駆動する。無言停止経路がない |
| OB-7 | image-only send の backend semantics が違う | 一方だけ送信失敗する | 両 backend の wire 変換が同型の guard（空 prompt + 画像時に text 要素を含めない）を持つ |
| OB-8 | resume recovery で editor context が落ちる | 再試行 turn の判断材料が欠ける | editor context は canonical payload（operation / obligation）に永続化され、recovery resume・restart 復元・queue drain の全再試行経路で TurnInput と system prompt に復元される |
| RT-1 | close / quit が finalization を共有しない | parts、permission、tool が未完了のまま残る | close / quit は同一の lifecycle usecase から Session 集約の terminal 収束（final parts 補完・未完了 tool の失敗確定・未解決 permission の Cancelled 解決）を通る |
| RT-3 | queued turn と human message の lifecycle が分裂 | 返信されない message が残る | human message と queue item は受理時に同一 CAS batch で束縛され、queue 由来 turn も Session 集約の同一遷移・同一 terminal 経路を通る |
| RT-4 | persistence crash 後の自己回復がない | その Session の全 mutation が失敗する | SQLite 単一 authority の下で mutation ごとに独立 CAS であり、一時故障は当該 commit のみの失敗になる。retry + PersistFailure notice + wakeable recovery が自己回復を提供する |
| RT-6 | Idle failure が durable surface に届かない | 理由不明の Error になる | Idle 中の Fatal は SessionErrored として durable commit され、Error part + queue pause + Error state として live / reload 両方に着地する |
| RT-7 | queue recovery failure を握りつぶす | queue が無言停止する | 復元失敗は ReconciliationRequired として durable fence され recovery inventory で可視化される。以後の send は明示拒否、wakeable recovery が恒常再試行する |
| FE-1 | cancelled permission が live では操作可能 | live / reload で dialog が違う | turn 終端時に集約が未解決 permission を一括 Cancelled 解決し、revision 付き state change + part 更新で live 配信される。live / reload とも同一 projector を読み、cancelled part は操作不能の resolved 表示になる |
| FE-2 | crash error が reload 後だけ見える | live では無言停止に見える | crash / turn 失敗 / idle fatal の全てが error part・crash snapshot delta・state change として live 配信される |
| FE-5 | failure banner が Session-scoped でない | 別 Session で消える・混ざる | feedback は Rust-owned の session-scoped state であり、UI も session view 内のみで表示・dismiss する |
| ST-3 | runtime owner と transition が一箇所に混在 | lifecycle 修正の考慮漏れが起きる | 遷移・受理は Session 集約 + runtime 系集約、駆動は orchestration module 群、process-local state は集約への委譲のみで、monolith が存在しない。集約単体テストが遷移・受理・terminal・recovery を検証する |
| ST-4 | persistence failure を握りつぶす | memory と durable state が乖離する | canonical mutation の失敗は受理前 rejection / 受理後 reconciliation へ、runtime projection の失敗は retry + PersistFailure notice + event log 自己修復へ着地する |
| ST-5 | lock ownership が不明確 | 将来 deadlock と停止を招く | transition coordinator / command locks が lock owner を明文化し、invalidation + generation fence により provider I/O 待ちが他の進行を塞がない |
| ST-6 | MessagePart が複数定義 | 語彙拡張が片側だけになる | MessagePart は domain 正本 1 定義 + 明示的境界写像（persistence V1 / protocol DTO / fixture snapshot）の構造である（V-D1） |
| NF-013 | permission response 完了 commit が stale 検出を participant 破棄に読み替える | durable 集約が WaitingPermission に固着し、同 turn の次の permission 要求が拒否され続けて turn が無言でハングする。別 operation の二重応答も admission を通る | provider effect 後の完了は fresh Session を復元して revision-guarded projection participant を再準備する。1 回の stale は再準備後に `PermissionResolved` と reducer projection を同一 batch へ含め、2 回目の stale は operation と pending obligation を `ReconciliationRequired` に原子的に着地させて recovery inventory に残す。並行反映済み・turn 終端後の late result は `PermissionResolved` を追記せず元 operation のみを完了する |
| NF-014 | recovery 解決状態の二重権威で、復旧失敗を経た session が恒久 send 不能になる | backend recovery が一度失敗すると、以後の復旧成功後も send が毎回失敗し、queued send は無言で開始されない | admission と immediate / queued send CAS は、projection・revision・owner-scoped pending obligations を一つの `AgentSessionLifecycleSnapshot` reader query から取得し、公示 flag と obligation view を同じ domain `classify_recovery_fact` へ渡す。event 列からの別分類は存在せず、projection mutation は snapshot revision を commit fence として保持する |

66 ID 外の確認事項: workflow-owned session への WorkflowTurn 送信可否は Session 集約の単一述語（open + recovered + quiescent）が所有し、複数 gate による別々の状態解釈や、直近 terminal の履歴 projection（`SessionState::Done` / `Error`）を送信拒否の根拠にする経路は存在しない（[workflow-ideal-lifecycle.md](../workflow-lifecycle/workflow-ideal-lifecycle.md) W-I5）。集約の admission 単体テストと、workflow turn が provider I/O 前に durable send を一度だけ commit する契約テストで検証されている。

## 検証で却下した historical candidates

調査時に候補となったが、記載された利用者影響を実コードから立証できなかったため active requirement にしない。

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
| 大容量 legacy message の migration 系欠陥（content 喪失・互換縮小・chunk 取得不能・aggregate 移行不能） | legacy migration 経路そのものが現行実装に存在しない（#1499 は legacy file-store 互換機構を持たない） |

## 現行方針

- audit の legacy JSON、file-store、旧 CLI version、過去 code location は historical evidence である。
- active contract は fixed SQLite authority、Rust-owned lifecycle、backend-owned read model、domain 集約による lifecycle 表現（L-P7）である。
- 旧 file-store は historical evidence としてのみ扱い、現行 runtime contract に取り込まない。
- shutdown の旧 page / ref / root / hash 表現と、store / generation / app-data generation identity は削除対象を特定する historical evidence であり、現行 authority または schema 契約ではない。
- 本書の根拠 file 参照は調査時点の historical evidence であり、以後の実装位置を拘束しない。
- OPEN 事項はない。
