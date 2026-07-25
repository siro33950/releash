# Agent チャット表示の理想形

作成日: 2026-07-07
更新日: 2026-07-24

本書は backend-owned state の何を、どの surface に、どう表示し、利用者が何を操作できるかを定義する。frontend の component、hook、reducer、DTO、pagination 実装は定義しない。

関連正本:

- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md)
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md)
- [close-quit-decision-table.md](close-quit-decision-table.md)
- [Issue #1499 Primary Spec](../../docs/specs/issues-1499/requirements.md)

## #1499 との境界

正常な SQLite startup 後は normal workbench を直接表示する。SQLite の create / open / schema evolution に失敗した場合は、Rust が返す safe startup failure と終了操作だけを表示し、Session、transcript、normal mutation、旧 file-store data、migration phase / progress を表示しない。

startup failure 中の Quit は shutdown progress を作らず、15 秒以内の process-local exit として扱う。SQLite が利用可能な場合だけ normal application shutdown surface を使う。

## 表示原則

- **P1（live / reload 等価）**: transient update は表示を速めるだけで、reload 後の read model と異なる意味を作らない。
- **P2（primary surface 一つ）**: 同じ state の primary surface を一つにし、他 surface は要約だけを表示する。
- **P3（無言遷移の禁止）**: terminal、failure、permission 取消、queue pause、configuration / Goal change は利用者に見える変化を伴う。
- **P4（scope 一致）**: Session、turn、application の failure と action を正しい scope だけに表示する。
- **P5（入力保全）**: durable acceptance 前に本文や添付を消さない。
- **P6（監査可能性）**: 後から、何を実行し、なぜ止まり、何が未解決かを read model と history から確認できる。

## Surface 定義

| ID | Surface | Responsibility |
| --- | --- | --- |
| S1 | transcript | message parts、turn terminal、durable Notice、permission history |
| S2 | activity | current tool / task progress の要約 |
| S3 | todo | current plan / todo の常時表示 |
| S4 | permission | pending request への回答と実効性 |
| S5 | composer / queue | input、send receipt / status、queue state、Stop |
| S6 | Session banner | Session-scoped feedback、recovery、protocol incompatibility |
| S7 | usage | token、context、cost の実績 |
| S8 | Session badge | Running、Waiting、Paused、Recovery、Error の要約 |
| S9a | launch configuration | new Agent の draft、preflight、launch progress |
| S9b | Session configuration | selected / effective config、Goal、pending / reconciliation |
| S9c | workflow Agent configuration | template、inherit / override、resolved preview |
| S10 | application shutdown | current quit flight、target summary、safe action |
| S11 | startup failure | safe failure、retry guidance、Quit |

## 語彙と表示先

| Vocabulary | Primary | Display rule |
| --- | --- | --- |
| Text | S1 | streaming と final を同じ message として表示 |
| Thinking | S1 | streaming 中は展開、完了後は折りたたみ。provider 共通 |
| ToolCall | S1 | running / succeeded / failed / denied / timed out / interrupted を区別し、text / image output と exit status を表示 |
| Task | S1 / S2 | child thinking、tool、unpaired result、全文を展開可能にする |
| TodoList | S3 | pending / in progress / completed と priority を表示 |
| Notice | S1 | warning、reroute、rate limit、MCP、compaction を error と区別 |
| Error | S1 | retryable / resolved / terminal を区別 |
| Permission | S4 | pending、responding、resolved、cancelled、reconciliation を区別 |
| TurnResult | S1 | normal、failed、stopped、crash、refusal / limit を区別 |
| TokenUsage | S7 | usage 実績として表示し、ReasoningEffort と混ぜない |
| Send operation | S5 | immutable receipt と進行 status を分け、Accepted 後の failure でも receipt を維持 |
| Stop operation | S5 | 10 秒以内の terminal または reconciliation として表示 |
| Session lifecycle | S6 / S8 | 10 秒以内の completion または同じ operation の reconciliation |
| Pending recovery | owner surface / S6 / S10 | safe observation と Rust-owned action だけを表示 |
| Configuration / Goal | S9b | selected、effective、pending、unknown、available action を分離 |
| Application shutdown | S10 | first intent、vocabularyが定めるcurrent status、target summary、unresolved result |
| Startup outcome | S11 | safe failure と Quit のみ。migration progress は表示しない |

## Composer と send

- send attempt ごとに本文と添付を保持する。
- Accepted receipt を受け取った場合だけ、その attempt の本文と添付を clear する。
- 受理前 rejection、payload conflict、結果不明では保持する。
- Accepted 後の reconciliation / failure で入力を復活または自動再送しない。
- response loss 後は同じ operation identity の readback / retry を使う。
- 待機中に利用者が追加した新しい入力を、古い attempt の成功で消さない。

## Tool / todo / Notice

- ToolCall は実行中と result 到着を別状態として表示する。output delta を完了と見なさない。
- 画像を返す tool result は利用者が同じ判断材料を確認できるよう表示する。
- Web search は検索 query と利用可能な result summary を表示する。
- Todo は provider 共通の進行表現とし、in progress を強調する。
- transient retry 中の error を恒久的な赤 error として残さない。
- operational Notice は severity と scope に応じて transcript / banner のどちらかを primary にし、二重の別 record を作らない。

## Permission UX

- pending request は一つだけ描画し、同じ request を transcript と dialog に二重表示しない。
- provider 取消、Stop、turn terminal を受けたら直ちに操作不能へ変える。
- response 送信中は回答内容を保持し、provider 実効性を確認できるまで成功表示しない。
- secret answer は masked input とし、plaintext を history、log、feedback に出さない。
- resolved chip は Allowed / Denied / Cancelled と説明・理由を表示する。
- exact response を安全に再利用できない場合だけ、再入力が必要であることを明示する。

## Turn、Stop、queue

- terminal は live で即時に表示し、reload 後にだけ error が現れる状態を作らない。
- Stop は「停止中」を無期限に表示せず、10 秒以内に terminal または同じ Stop identity の reconciliation へ移る。
- queue item は queued、starting、paused、failed、cancelled、needs resolution を区別する。
- Stop、close、archive、backend switch、quit、failure、crash 後は paused を表示し、明示 resume まで自動開始しない。
- cancelled message は transcript に mark して残す。

## Agent 実行設定 UX

- Agent mode は `Ask / Edit / Plan / Auto / Bypass` の排他的 selector とする。
- Auto は provider reviewer の範囲、Bypass は危険性と追加確認を表示する。どちらも workflow checkpoint を越えない。
- ReasoningEffort は「工数（推論レベル）」として S9b に表示し、S7 の使用量・cost と分離する。
- selected と effective / unknown、反映時点、unsupported reason を同時に理解できるようにする。
- Goal は current status、pending transition、evidence、provider strategy / effect を表示する。
- launch draft、Session configuration、workflow template は見た目を再利用しても state と command を共有しない。
- protocol incompatibility は新規 send を無効化し、利用者が確認できる safe reason を表示する。

## Feedback と recovery

- command feedback は Session-scoped に表示し、別 Session の activity で消さない。
- 同じ failure identity の解決または明示 dismiss だけが対象 entry を更新する。
- feedback capacity 到達時も既存 feedback の閲覧、dismiss、safe resolution を利用できる。
- pending recovery は current resource の表示と、過去の shutdown に記録された collection を区別する。
- recovery action は vocabulary が定める意味のうち backend が提示したものだけを有効にし、frontend に generic retry を追加しない。
- action response loss は同じ action identity の進行を表示し、別 action を自動生成しない。

## Close / quit UX

- view close は対象 view だけを閉じ、backend progress を合成しない。
- active Session close / open archive は final parts、SessionClosed、queue pause を表示する。
- Idle close / archive は synthetic terminal を表示しない。
- backend switch は old backend の結果確認後だけ new effective backend を表示する。結果不明では old backend と queue pause を維持する。
- application quit は S10 に一つの flight を表示し、複数 surface の要求を別 progress にしない。
- 15 秒で未完了 target がある場合は、exit / restart と restart 後に確認できる recovery を表示する。
- shutdown summary と detail が一致しない場合、成功や `None` を表示せず safe internal failure とする。

## Startup failure UX

- S11 は normal workbench と排他的に表示する。
- Rust が返す `Store in use`、`Storage unavailable`、`Unsupported runtime`、`Unsupported store version`、`Initialization state invalid`、`Store validation failed`、`Schema evolution failed` のいずれか、allow-listed safe description、correlation、次回launch時の扱いをそのまま表示し、raw database error、path、SQL を表示しない。
- 初回 create の中断を証明できる場合だけ、次回起動で同じ path の初期化を再試行することを説明する。既存の空 file だけから retryable と表示しない。
- 初期化済み store または初回作成残骸と証明できない file の検証 failure は、自動的に削除・再初期化しないことを明示する。
- 利用可能な操作は Rust が返す Quit 一件に限定し、Retry / Reset / Import / Open Workbench、durable quit operation、shutdown / migration progress を合成しない。
- Quit の重複clickは同じprocess-local one-shotへjoinし、frontendで別flightや成功状態を作らない。

## frontend 実装境界

frontend は read model の表示、入力受付、backend command 呼出しだけを行う。次を frontend に置かない。

- terminal winner、retryability、recovery action、安全な failure の判定
- provider / model capability、mode / Goal / effort の写像
- current shutdown と history detail の整合性判断
- startup failure と normal admission の判断
- raw error string による分類

## トレーサビリティ

| Problem group | Presentation |
| --- | --- |
| FE-1〜FE-7 | P1〜P6、permission、feedback、Task、usage |
| OB-1〜OB-8 | composer、Stop、queue |
| CL / CX / RG | ToolCall、TodoList、Notice、TurnResult |
| #1445〜#1451 | S9a〜S9c |
| #1499 | send、recovery、close / quit、startup failure |

## 設計判断

- **P-D1**: usage indicator は composer 近くに常設し、詳細は on demand にする。
- **P-D2**: durable Notice と transient SessionOperationFeedback を分離する。
- **P-D3**: cancelled queue message は transcript に mark して残す。
- **P-D4**: launch、Session、workflow の configuration surface を分ける。
- **P-D5**: mode は五値の排他的 selector とする。
- **P-D6**: ReasoningEffort と TokenUsage / cost を視覚・意味の両方で分離する。
- **P-D7**: Goal は Session configuration surface の独立 projection として扱う。
- **P-D8**: SQLite startup failure は専用 surface に safe failure と Quit だけを表示する。
