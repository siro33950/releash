# Agent セッションライフサイクルの理想形（不変条件）

作成日: 2026-07-07

milestone 84「Agentチャット安定化」のドキュメント群:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題点インベントリ（全 66 件、要求リスト）
- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md) — 正規化語彙・データ構造の理想形
- **agent-chat-ideal-lifecycle.md（本書）** — ライフサイクルの理想形（不変条件）
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md) — UI 表示の理想形

本書は「セッション・turn・permission・queue が、どの経路を通っても何を保証するか」を不変条件（invariant）として定義する。監査で確定した lossy-lifecycle / divergent 問題群の解消先。語彙は vocabulary 文書の型を前提とする。

## 設計原則

- **L-P1 (durable-first)**: UI が表示する全状態は durable event → read model から復元可能でなければならない。transient event（Tauri emit）は同じ結果を早く見せるための加速手段であり、正しさに関与しない。
- **L-P2 (終端の明示)**: turn・permission・tool call・queue item は、どの経路（正常・中断・切断・クラッシュ）でも必ず終端状態に到達し、その終端が durable に記録される。「進行中のまま忘れられる」状態を作らない。
- **L-P3 (失敗の着地)**: 内部の失敗（永続化失敗・emit 失敗・backend 応答エラー）は握りつぶさず、`Notice` / `Error` / 構造化ログのいずれかに必ず着地させる。
- **L-P4 (backend 差の吸収)**: 停止・生存確認・頑健性の保証水準は backend に依らず同一とし、差はランタイム層が吸収する。

## 不変条件

### I1: turn 終端保証

あらゆる終了経路 — 正常完了・interrupt・Fatal・session close・backend 切替・アプリ終了 — で、`TurnResult` の durable 記録と streaming parts の Final 化（`FinalPartsRecorded`）が実行される。

- ギャップ: RT-1（close/切替/終了時に finalize も flush もされない）
- 要点: `close_session` / backend 切替 / アプリ終了 hook は「flush → `TurnResult::Interrupted { reason: SessionClosed }` で finalize → runtime close」の順を必ず踏む。backend の終了イベントを捨てない。

### I2: クラッシュ回収

アプリ起動時（およびセッション初回ロード時）に dangling turn（`TurnStarted` があり終端 event が無い）を検出し、`Interrupted { reason: Crash }` で finalize する。permission は `Cancelled(effective=false)`、tool call は `Interrupted` に畳む。

- ギャップ: RT-2（回収経路が無く、スピナー・確認待ちが永久残留）
- 要点: 回収は read model 投影時に lazy に行ってもよいが、結果は durable に書き戻す（reload の度に再判定しない）。

### I3: streaming flush 保証

streaming 中の本文・tool 出力は一定間隔（現行 1s の定期 flush を保証として明文化）で durable 化され、損失窓は最大 flush 間隔に限定される。turn 終端では必ず Final 化する。

- ギャップ: RT-2（クラッシュ時に直前約 1 秒が消える — これは許容損失として明文化）、FE-3（hydration が flush 済みデータを取りこぼす — presentation 側 P1 で解消）
- 要点: 「1 秒」は保証値として定数化し、テストで検証する。

### I4: queue の永続と一貫性

pending queue は durable であり、アプリ再起動・session close・backend 切替を跨いで生存する。queue item の終端は「実行開始 / ユーザー取り消し / 起動失敗」のいずれかで、全て記録・可視化される。

- ギャップ: OB-3 / RT-3（メモリのみで無言消滅）、OB-4（取り消しても human message が復活）、OB-6 / RT-7（起動失敗で無言停止）
- 要点:
  - queue は session state として永続化する（event log の replay 対象にはしない）。
  - 取り消し時は永続化済み human message を「cancelled」としてマークし（削除しない。マークは message read model へ durable に記録し reload 後も保たれる）、復元コンテキストからも除外する（OB-4）。
  - 起動失敗した queue item は `Failed` として queue に残し、ユーザー操作（再試行・取り消し）を待つ。無言のまま次の送信で暗黙復活させない（RT-7 の解消）。

### I5: interrupt 保証

Stop 操作は常に受理される。backend への interrupt 送出が不可能・無応答でも、猶予（既存 Claude の synthetic abort 10s を両 backend の共通保証にする）の後にランタイムが turn を強制終端する。

- ギャップ: OB-1 / SD-2（Codex は turn_id 未取得窓で無言 no-op、フォールバック無し。frontend も再押下を握りつぶす）
- 要点:
  - turn_id 未取得窓の Stop は「turn_id 取得後に interrupt を送る」予約として保持する。
  - interrupt 後の queue は自動 drain しない（OB-5）。queue を paused 状態にし、再開はユーザー操作（presentation 文書 §queue）。

### I6: ユーザー入力の無損失

送信操作は成功（turn 開始 or queue 追加）以外の結果を持たない。steer 非対応・stall 中・起動失敗のいずれでも、入力テキスト・添付画像は失われない。

- ギャップ: OB-2（stalled turn 中の送信が即エラーになり入力ごと消える）
- 要点: `steer` 未対応 backend への実行中送信は queue へ積む（現行仕様）。stalled 判定中も同じ経路に載せ、エラーにしない。送信 API が失敗した場合も入力欄の内容は保持する（presentation 側 P5）。

### I7: permission の有効性

permission request は「backend が回答を待っている間」だけ Pending である。CLI 側の取り下げ（Claude `control_cancel_request`）・turn 中断・turn 終端で即座に `Cancelled` に遷移し、durable / UI の両方へ反映される。解決の記録は実効性（`effective`）を区別する。

- ギャップ: CL-1（cancel 未処理で効かないダイアログが残り、誤った Allowed/Denied が永続する）、FE-1（interrupt 中に操作可能なダイアログが残る — 表示は presentation 側）
- 要点: `control_cancel_request` を処理して `PermissionRequested` を Cancelled へ更新する。失効後に届いた回答は `effective=false` で記録し、backend へは送らない。

### I8: 状態変更の ack 駆動と失敗の可視化

backend に対する状態変更（set_model / set_permission_mode / interrupt / permission 応答）は、backend の応答（control_response / JSON-RPC response）を検査して初めて確定する。楽観更新で UI と backend 実態を乖離させない。永続化・emit の失敗は握りつぶさず着地させる。

- ギャップ: CL-2（control_response を無言破棄し楽観更新）、CX-6（JSON-RPC error response を warn ログのみで握りつぶし）、ST-4（`let _ =` による persist 失敗の無視）、RT-4（event log が自己修復せず恒久故障）、RT-8（append 失敗時に欠落 parts で上書き）
- 要点:
  - 応答エラー時は state を巻き戻し、`Notice(level=Error)` で通知する。
  - `let _ =` を全廃し、persist 失敗は「リトライ → 失敗継続なら Notice(PersistFailure) ＋ 該当操作のエラー化」に統一する。
  - event log の破損（欠け `]` 等）は読み込み時に自己修復し、修復した事実を Notice に残す（RT-4）。
  - **例外**: `Notice(PersistFailure)` 自体の永続化も失敗し得るため、PersistFailure に限り transient（session バナー）での表示を許容する。durable-first（L-P1）の唯一の例外として presentation 文書 P1 にも明記する。

### I9: resume 回復の統一

backend session の resume 失敗（mismatch・thread 消失）は、両 backend とも同一の回復経路を通る: `BackendSessionCleared`（語彙 V-D9 で配線）→ resume metadata クリア → 新規 establish → `Notice` で「文脈が引き継がれない」ことを通知。セッションが恒久的に死ぬ・無言で文脈が消える、のどちらも許さない。

- ギャップ: SD-1（Claude は無言自動復旧・Codex は恒久死）、OB-8（requeue で editor_context が脱落）
- 要点: requeue する turn input は editor_context を含め完全に保全する。

### I10: backend stdout の頑健性統一

両 backend の stdout 読み取りは共通の保証を持つ: 非 JSON 行は skip してカウント（Fatal にしない）、1 行サイズ上限超過は種別推定つきで破棄を可視化（`Notice(OversizeDropped)`）、いずれもセッションを殺さない。

- ギャップ: SD-3（Codex は非 JSON 行 1 行で即死・サイズ上限なし。Claude は 8MB 破棄が type 不明のまま）
- 要点: 読み取りラッパを infrastructure 共通部品にし、両 backend で使う。

### I11: 生存シグナルと stall 判定

「進行中」の判定は backend からの進行シグナル（parts delta / KeepAlive / status 変化）の有無で行い、シグナル要件は backend 別に定義する。長考（reasoning）中も何らかのシグナルが届くこと（Codex は reasoning delta 購読 — CX-3 の解消 — で充足）。stall 診断は「実際に進行が観測できない」場合のみ発火する。

- ギャップ: SD-4（Codex reasoning 中の無シグナルで stall 誤検知）、ST-9（permission 待ち以外の不可視停止に診断が無い、閾値 60s）
- 要点: stall 診断を全 phase（Streaming / WaitingPermission / Interrupting）に拡張し、`Notice(Diagnostic)` として durable に残す。

### I12: エラーの着地保証

backend プロセス死・turn 失敗・Fatal は、発生時点で durable な `Error` part または `Notice` として着地し、live UI にも即時反映される（表示規則は presentation P3）。workflow への turn 完了通知は構造化された失敗理由（`TurnError`）を運ぶ。

- ギャップ: RT-6（Idle 中 Fatal が痕跡ゼロ）、FE-2（crash/timeout が live で無言 — 表示側）、RT-5（workflow に exit 1 しか伝わらない）

### I13: 排他とロックの規約

session runtime lock の保持中に、別 session の lock・長時間 await（backend I/O）を行わない。lock の取得順序と保持範囲を規約として明文化し、prune はランタイムハンドル取得に依存しない方式にする。

- ギャップ: ST-5（二段ロックと prune skip）、ST-3（巨大 module 内で lock 範囲が追えない）

## turn 状態機械（明示化）

ST-3 の解消として、phase 遷移を規範として一覧化する（実装は本表と 1:1 対応のモジュールに分解する）:

| phase | 入力イベント | 遷移先 | 必須アクション |
|---|---|---|---|
| Idle | start_turn | Streaming | TurnStarted durable 化、queue から取った場合は item 終端記録 |
| Idle | Fatal | Idle | Notice(Error) durable 化（I12、RT-6） |
| Streaming | PartsMerged / TokenUsageUpdated | Streaming | merge → 定期 flush（I3） |
| Streaming | PermissionRequested | WaitingPermission | permission part durable 化＋state change emit |
| Streaming | TurnCompleted | Idle | finalize（Final 化→TurnResult 記録→workflow 通知→queue 評価） |
| Streaming | interrupt 要求 | Interrupting | backend interrupt 送出 or 予約（I5） |
| Streaming | Fatal / stream 終了 | Idle | Interrupted{Crash} で finalize（I1） |
| WaitingPermission | respond_permission | Streaming | effective 判定→backend 送出→PermissionResolved 記録 |
| WaitingPermission | permission cancel（CLI 取り下げ） | Streaming | Cancelled(effective=false) へ更新（I7） |
| WaitingPermission | interrupt 要求 | Interrupting | pending permission を Cancelled(effective=false) へ畳む（I7）＋backend interrupt 送出 or 予約（I5） |
| WaitingPermission | TurnCompleted | Idle | 未解決 permission を Cancelled に畳んで finalize（現行踏襲） |
| Interrupting | TurnCompleted / 猶予超過 | Idle | finalize。猶予超過時は強制終端＋Interrupted{Timeout} |
| （全 phase） | close_session / app quit | Idle | I1 の手順（flush → finalize → close） |

queue は phase と直交する永続 sub-state: `items: [{ input, status: Queued/Starting/Failed/Cancelled }]` ＋ `paused: bool`（I4 / I5）。

## シナリオ別保証（受け入れ基準の骨子）

| シナリオ | 保証 |
|---|---|
| 正常完了 | TurnResult(Completed{stop_reason}) が durable。reload 後も同一表示（P1） |
| ユーザー Stop | 最悪 10s で Idle。queue は paused。入力欄・queue は無損失 |
| streaming 中に tab close / backend 切替 / アプリ終了 | 再オープン時: 本文は flush 済みまで表示、turn は Interrupted{SessionClosed}、スピナー・permission 残骸なし |
| クラッシュ → 再起動 | dangling turn が Crash で回収済み。損失は最大 1s の本文のみ（I2/I3） |
| resume 失敗 | 両 backend とも新規 establish ＋ Notice。以後のターンは正常。editor_context 保全（I9） |
| queue に 2 件積んで再起動 | queue が復元され、明示操作で実行再開できる（I4） |
| permission 待ち中に interrupt | ダイアログは即 Cancelled 表示。誤記録なし（I7） |
| モデル/モード切替の失敗 | UI は旧値のまま、Notice(Error) が出る（I8） |
| stdout に非 JSON 行 | 両 backend ともセッション継続＋カウント可視化（I10） |

## backend 差の吸収規約

| 関心事 | 保証水準 | Claude | Codex |
|---|---|---|---|
| interrupt | 常に受理・最悪 10s で終端（I5） | synthetic abort 踏襲 | turn_id 予約＋猶予強制終端を追加 |
| 生存シグナル | 長考中もシグナル継続（I11） | thinking delta ＋ keep_alive | reasoning delta 購読を追加（CX-3） |
| stdout 頑健性 | 非 JSON 行 skip ＋サイズ上限＋可視化（I10） | 既存を可視化強化 | 共通ラッパ導入で即死を解消 |
| steer | 非対応は queue へフォールバック（I6） | queue | queue（将来 turn/steer 対応時に置換） |
| resume 失敗 | Cleared → 再 establish ＋ Notice（I9） | 無言復旧を通知付きに | 恒久死を回復経路に接続 |

## トレーサビリティ（本書が解消する問題）

| 問題 ID | 不変条件 / 節 |
|---|---|
| RT-1 | I1 |
| RT-2 | I2, I3 |
| RT-3, OB-3 | I4 |
| OB-4 | I4（cancelled マーク） |
| OB-5 | I5（queue paused） |
| OB-6, RT-7 | I4（起動失敗の可視化） |
| OB-1, SD-2 | I5 |
| OB-2 | I6 |
| OB-8 | I9 |
| CL-1 | I7 |
| CL-2, CX-6 | I8（ack 駆動） |
| ST-4, RT-4, RT-8 | I8（失敗の着地・自己修復） |
| SD-1 | I9 |
| SD-3 | I10 |
| SD-4, ST-9 | I11 |
| RT-6, RT-5 | I12 |
| FE-1, FE-2, FE-3（backend 側の裏付け） | I7, I12, I3 |
| CX-2 | I7 の変種: elicitation は「応答義務のある要求」として permission 経路に載せる（無応答ハングの解消。語彙は V-D6 の Question を流用） |
| ST-3 | turn 状態機械の明示化 |
| ST-5 | I13 |

## 設計判断

- **L-D1**: queue の永続先は session state（event log の replay には混ぜない）。取り消し・pause を上書きで表現でき、履歴汚染がないため。
- **L-D2**: interrupt の強制終端猶予は 10s（既存 Claude synthetic abort の実績値を共通保証に昇格）。
- **L-D3**: crash 回収は「検出は lazy・記録は durable」。起動時の全セッション走査はしない（開いたセッションから回収）。
- **L-D4**: queue 取り消しは human message の削除ではなく cancelled マーク。履歴の誠実性と OB-4（復元コンテキスト除外）の両立。
- **L-D5**: interrupt 後の queue は paused（自動 drain 禁止）。「止めたのに続く」（OB-5）の解消を優先し、再開は明示操作。

## 確定事項（2026-07-07 レビューで確定）

1. **I5 / L-D5**: interrupt 時は queue を**常に paused** にする（選択式は不採用）。再開は queue chips の明示操作。
2. **L-D4**: 取り消した queue メッセージは **cancelled マークで transcript に残す**（非表示は不採用）。復元コンテキストからの除外は共通実施。
3. **I2**: crash 回収は**中断チップのみ**（起動時のバナー・ダイアログ通知は出さない）。古いセッションの一斉回収も静かに行う。
