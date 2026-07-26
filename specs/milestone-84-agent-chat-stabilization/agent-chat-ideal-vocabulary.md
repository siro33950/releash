# Agent チャット正規化語彙の理想形

作成日: 2026-07-07
更新日: 2026-07-24

本書は milestone 84 で利用者と workflow が観測する Agent チャットの product / domain vocabulary を定義する。Rust の型定義、保存形式、SQL schema、transport field、実装手順は定義しない。

関連正本:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題と影響
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) — lifecycle の不変条件
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md) — 表示先と利用者操作
- [close-quit-decision-table.md](close-quit-decision-table.md) — close / quit surface の意味
- [Issue #1499 Primary Spec](../../docs/specs/issues-1499/requirements.md) — 恒久 SQLite store と operation safety

## 優先関係

#1499 は固定 path の SQLite store を直接 create / openし、正常稼働時の唯一の persistence authority とする。変更前の file-store data は vocabulary の入力でも互換対象でもない。

Phase 0、F2、F3、D3などの計画labelと、廃止したfile-storeの物理用語は現行runtime vocabularyではない。通常の SQLite schema evolution、configuration compatibility、watch subscription initialization はそれぞれ別概念であり、legacy-data migration と呼ばない。

## 設計原則

- **V-P1（lossless）**: provider が利用者判断に必要な情報を送った場合、未知のものを含め、無言で捨てたり別の意味へ潰したりしない。
- **V-P2（single meaning）**: 同じ概念は backend、live / reload、Session / Workflow で同じ意味を持つ。
- **V-P3（explicit terminal）**: turn、tool、permission、queue、operation は進行中と終端を区別し、結果不明を成功や未開始へ推測しない。
- **V-P4（durable authority）**: transient delivery は表示を速めるだけで、受理事実や完了事実の正本にならない。
- **V-P5（Rust ownership）**: state transition、capability、available action、安全な failure 分類は Rust が所有する。
- **V-P6（selected / effective separation）**: 利用者が選んだ値、provider に反映済みの値、反映結果不明を区別する。
- **V-P7（bounded read model）**: 通常 surface は現在必要な state と bounded history だけを持ち、全履歴を保持しない。
- **V-P8（semantic transport）**: public interface は意味を lossless に運び、domain 型や persistence 表現をそのまま公開しない。

## Product / domain vocabulary

### MessagePart

会話中に利用者が観測できる最小の意味単位。domain に一つだけ存在し、provider、SQLite、Tauri、WebSocket、frontend はこの意味へ写像する。

| Term | Meaning |
| --- | --- |
| Text | 利用者または Agent の発話本文 |
| Thinking | provider が公開を許可した reasoning / thinking |
| ToolCall | 一つの tool 実行と、その進行・結果 |
| TodoList | Agent が示した計画と進行状況 |
| Notice | 警告、設定、rate limit、reroute、compaction などの運用通知 |
| Error | turn または operation に結び付く失敗 |
| Permission | 利用者回答を必要とする要求と、その決定 |
| Task | subagent / background task の進行 |
| Image | 会話または tool result に含まれる画像 |

### ToolCall

ToolCall は tool の種類、入力の安全な要約、進行状態、出力、終了理由を一つの意味として扱う。pending、running、succeeded、failed、denied、timed out、interrupted を区別する。command の exit status、web search の query、tool result の text / image は利用可能な範囲で保持する。

provider 固有の raw message や表示名から frontend が状態を推測しない。unknown tool は内容を落とさず、generic な tool として表示できる。

### TodoListItem

Todo item は内容、pending / in progress / completed の状態、provider が提供する priority を持つ意味である。Claude と Codex で同じ表示と lifecycle を持つ。

### Notice

Notice は会話の失敗とは限らない operational information である。少なくとも warning、configuration warning、protocol incompatibility、model reroute、rate limit、MCP status、compaction を区別する。

durable Notice は transcript から再取得できる。state transition 前に command 自体が失敗したことを知らせる transient feedback は Notice と混同しない。

### SessionOperationFeedback

Session-scoped command の未解決 failure を表す。対象 operation、safe description、現在利用できる解決操作を持つ意味であり、別 Session の operation で消えない。成功履歴や transcript message ではない。

### Send operation

通常 send の一回の利用者 intent。caller が保持する stable identity、immutable acceptance receipt、進行する execution status から成る。

- receipt は「どの input が受理され、turn または queue のどちらへ結び付いたか」という不変事実である。
- status は provider start、running、reconciliation、failure、terminal など受理後の進行である。
- response loss と restart は同じ operation へ戻る。
- same identity / different input は conflict であり、新しい send ではない。

### Stop operation

一つの target turn を止める intent。Accepted 後は terminal winner、superseded、結果確認必要のいずれかへ収束し、queue を pause する。Stop の public deadline は 10 秒である。

### Session lifecycle operation

normal Session close、open Session archive、closed Session archive、backend switch の backend operation。view close とは別であり、same action の再要求は同じ進行へ join する。

### Application quit

全 graceful quit surface を一つの application shutdown flight へ正規化した intent。最初に受理された exit / restart intent が flight を所有し、後続要求は同じ結果へ join する。public deadline は 15 秒である。

startup failure 中の cooperative exit は normal application quit operation ではなく、durable progress を持たない process-local exit である。

### Durable obligation

受理済み operation に残る未完了 work を、restart 後も同じ意味で監督するための domain concept。owner、purpose、現在状態、安全な observation、利用可能な action を持つ。

provider 作用の結果を確認できない場合、obligation は成功または未開始を推測せず reconciliation を表す。local publication など外部作用でない work も、その意味を明示して扱う。

### Pending recovery

未完了 obligation の利用者可視 view。Session、Workflow、closed history、unowned runtime、application shutdown の正しい owner surface へ表示する。current collection と、過去の shutdown が固定した historical collection を混同しない。

### Recovery action

Rust が安全性を判定して提示する、pending recovery を前進させる操作。同じ action identity の response loss / restart は同じ result へ戻る。安全な再実行または authoritative readback を証明できない action は提示しない。

利用者へ提示できる action の意味は次に閉じる。

| Action | Meaning |
| --- | --- |
| Read again | 外部作用を開始せず、同じ identity の現在の根拠を再取得する |
| Retry same effect | backend が安全と証明できる場合だけ、元と同じ effect identity を再試行する |
| Use observed result | authoritative に確認できた結果を canonical outcome として採用する |
| Cancel if safe | effect が開始されていないと確認でき、対象が取消可能な場合だけ取消す |
| Keep for manual resolution | 推測で前進させず、未解決状態を維持する |

action result の意味は `Pending`、`Succeeded`、`Confirmed no effect`、`Ambiguous`、`Cancelled before effect`、`Unchanged` に閉じる。`Ambiguous`を成功または無作用へ読み替えない。

### Safe operation failure

利用者が次の行動を判断できる安全な failure。failure kind、retryable かどうか、安全な説明、correlation を意味として持つ。private input、secret、filesystem path、SQL、provider raw payload は含めない。

### PermissionRequest

provider が利用者回答を待つ一つの要求。command approval、file change、question、MCP elicitation などを同じ product concept として扱う。question identity、secret / multi-select / free-form の性質、回答の実効性を失わない。

Allowed、Denied、Cancelled と、回答は保存されたが provider への実効性を確認できない状態を区別する。取り下げ済み要求を操作可能なまま残さない。

### TurnResult

一つの turn の terminal result。normal completion、failure、interrupt、crash と、provider が示す refusal / limit などの stop reason を区別する。workflow はこの意味を同じまま受け取る。

### TokenUsage

provider が報告した input、output、context、cost の利用実績。ReasoningEffort や budget とは別概念である。provider が確認できない値を zero や推測値として表示しない。

### AgentRuntimeEvent

provider adapter から domain lifecycle へ渡る意味的 event。text、thinking、tool、permission、notice、usage、turn result、liveness を backend 間で正規化する。

unknown content は安全な Notice または protocol incompatibility へ着地する。応答が必要な control message を無応答で破棄しない。

### Agent mode

利用者が選ぶ execution behavior。`Ask / Edit / Plan / Auto / Bypass` の排他的な五つを product vocabulary とする。

- Auto の判断主体は provider classifier / reviewer であり、Releash は結果を監査する。
- Bypass は Rust-owned policy check と execution-scoped confirmation を必要とする。
- どの mode も workflow の human checkpoint を迂回しない。

### Agent Goal

Session の継続的な completion condition と status。workflow task とは別概念で、Session ごとに current Goal は最大一つである。set、edit、pause、resume、clear、completed、failed、blocked と provider capability / effect を利用者に示す。

### ReasoningEffort

model の応答・推論強度を調整する behavioral signal。UI 名は「工数（推論レベル）」とする。provider / model が提示する option、default、反映時点を使い、selected、effective、unknown を区別する。

TokenUsage、cost、時間、turn 数、token / cost / time budget、厳密な上限を意味しない。

### Agent session configuration

provider、model、Agent mode、ReasoningEffort の selected / effective state。Goal は別 aggregate である。configuration と Goal の action 可否、provider capability、pending / reconciliation は Rust が評価する。

workflow template、launch 時に解決した configuration、実行 Session の configuration、queue item が固定した configuration は別 scope であり、暗黙に相互上書きしない。

### Local atomic event transaction

一つの利用者操作として不可分な domain event と state change が、一つの結果として可視になるという意味。SQL statement、table、commit order は vocabulary に含めない。

### Read model

surface が現在描画するための backend-owned state。live と reload で同じ意味を持ち、bounded history と direct operation lookup を使う。frontend は read model の mirror であり、transient event から別の domain state を作らない。

### Startup outcome

SQLite の create / open、必要な schema evolution、validation の結果。Ready または normal workbench を開けない safe failure として Rust が決める。

未完了の初回作成は、それを証明する initial-create evidence がある場合だけ同じ fixed path で再試行できる。既存の空 file を証拠なしに未初期化と推測しない。初期化済み store または初回作成残骸と証明できない既存 file の検証 failure は、自動的に空 store へ置換しない。旧 file-store は startup outcome の入力ではない。

safe failure の利用者可視分類は次に閉じる。内部 error、path、SQL は分類名や説明へ流さない。

| Classification | Meaning |
| --- | --- |
| Store in use | 別 process が fixed store の writer ownership を保持している |
| Storage unavailable | filesystem、permission、capacity などにより一回の store attempt を完了できない |
| Unsupported runtime | bundled SQLite runtime が必要条件を満たさない |
| Unsupported store version | store は識別できるが、この build が安全に扱える schema version ではない |
| Initialization state invalid | 既存 file を未完了初回作成と証明できず、安全に create / open できない |
| Store validation failed | 初期化済み store の metadata、key、integrity invariant を検証できない |
| Schema evolution failed | supported schema evolution の結果を検証できない |

safe startup failure は classification、safe description、correlation、次回launch時の扱い、利用可能action `Quit`だけを持つ。`Quit`はapplication shutdownではなく、durable stateを作らないprocess-local actionである。startup attemptのpath・lock・busy wait・retry回数と、Rust interfaceの正確な契約はPrimary Specを正本とする。

### Application shutdown state

一つの application quit flight の current / history summary と ordered target detail を表す domain state。保存、保存後検証、effect gate、current / history read、target pagination、recovery は同じ canonical shutdown identity と revision を参照する。page file、reference、root hash、current recovery collection はこの state の代替 authority ではない。利用者可視のstatusは次の意味に閉じ、内部処理段階をそのまま公開しない。

| Status | Meaning |
| --- | --- |
| Preparing | まだshutdown effectを開始せず、安全に開始できるかを確定している |
| In progress | shutdown effectを開始した、または開始結果を確認中である |
| Completed | 全targetのterminal resultが確定した |
| Failed | effect開始前に安全に失敗し、理由を表示できる |
| Cancelled | effect開始前に安全に中止した |
| Needs resolution | 未完了または結果不明のtargetを同じflightで解決する必要がある |

## 設計判断

- **V-D1**: `MessagePart` は domain に一つだけ置き、persistence / public representation は明示的な境界写像にする。
- **V-D2**: tool use と result を一つの ToolCall lifecycle として扱い、status、output、exit reason を保持する。
- **V-D3**: todo は pending / in progress / completed と priority を持つ backend 共通語彙にする。
- **V-D4**: operational Notice は transcript から復元可能にし、SessionOperationFeedback とは分離する。
- **V-D5**: transient retryable error と terminal error を区別し、解決済み error を恒久 failure として残さない。
- **V-D6**: permission / question / elicitation は exact response semantics と実効性を保持する。
- **V-D7**: TurnResult は completion、failure、interrupt、crash、stop reason と利用可能な stats を失わない。
- **V-D8**: TokenUsage は実績として扱い、ReasoningEffort と分離する。
- **V-D9**: AgentRuntimeEvent は provider 差を domain semantics へ正規化し、未知 control message を fail closed にする。
- **V-D10**: configuration、Goal、workflow template、resolved launch configuration、queue item が固定した configuration の scope を分け、selected / effective / pending / reconciliation を明示する。
- **V-D11**: local atomic event transaction は domain-visible な all-or-nothing 結果を定義し、物理 schema を正本にしない。
- **V-D12**: provider wire は実行中 protocol と検証済み contract の互換性を確認し、drift を `ProtocolIncompatible` として閉じる。
  - **V-D12a**: Codex adapter は公式 protocol contract に追随できる typed boundary を持つ。
  - **V-D12b**: Claude adapter は SDK contract に追随できる typed boundary を持つ。

## トレーサビリティ

| Problem group | Canonical vocabulary |
| --- | --- |
| CL / CX の無言破棄 | AgentRuntimeEvent、Notice、PermissionRequest、TurnResult |
| SD の backend 差 | ToolCall、Stop operation、TokenUsage、Agent mode |
| OB / RT の入力・lifecycle 喪失 | Send operation、Durable obligation、Pending recovery |
| FE の live / reload 差 | Read model、SessionOperationFeedback |
| RG の語彙不足 | ToolCall、TodoListItem、TurnResult、Notice、TokenUsage |
| ST の構造要因 | V-D1、V-D9、V-D11、V-D12 |
| #1499 | operation、obligation、startup outcome、application shutdown state |
