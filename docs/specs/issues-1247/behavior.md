# behavior — issues-1247

`AgentSessionEventLog` / Projector による Agent session 保存正典の振る舞い定義。

本変更は内部アーキテクチャの再定義であり、外部から観測可能な振る舞い（UI 表示・session 一覧・streaming 体験・workflow 完了判定の結果）は不変である（requirements R6）。
そのため本ドキュメントの振る舞いは、UI 操作ではなく「durable event 列を正典として read model が決定的に導出される」というビジネスルールを、観測可能な結果（read model の状態）として定義する。

Gherkin 中の用語:

- **durable event**: replay 可能で session の事実を構成する最小語彙（`AgentSessionEvent`）。
- **live-only delta**: 表示中のみ意味を持ち replay されない逐次差分（text / tool-input / reasoning の delta）。
- **read model**: event 列から projection で導出される派生状態。本変更では少なくとも message page・session status・workflow turn-complete input を含む。
- **session status**: 永続側 `SessionState`（Active / Idle / Done / Error / Closed / Archived）に加え、runtime 由来の遷移状態（streaming 中・permission 待ち）を含む実行フェーズ。

---

## Feature: durable event 列を正典とした read model の projection

session の事実を durable event 列として append し、message page・session status・workflow turn-complete input を event 列から projection で導出する。read model は event 列の射影であり、event 列を唯一の正典とする。

### Background:
```gherkin
Given Agent session の保存正典が durable event 列（AgentSessionEvent）である
And read model（message page・session status・workflow turn-complete input）は event 列の projection として導出される
And streaming の逐次差分は live-only delta として扱われ replay されない
```

---

### Rule: durable event を append すると read model が event 列から導出される

#### Scenario: prompt 投入から turn 完了までの event 列を message page へ projection する
```gherkin
Given 空の event 列を持つ Agent session
When prompt 投入・tool 呼び出しの成功・turn 完了を表す durable event を順に append する
Then message page は event 列を projection した結果として導出される
And message page には投入された prompt と tool 呼び出しの結果が含まれる
And read model に event 列に存在しない message は現れない
```

#### Scenario: 同一の event 列からは常に同一の read model が導出される
```gherkin
Given turn を 1 つ含む durable event 列
When event 列から read model を 2 回 projection する
Then 2 回の projection 結果は一致する
```

#### Scenario: live-only delta は read model の正典に含まれない
```gherkin
Given durable event 列と、それに付随する live-only delta（text / tool-input / reasoning の逐次差分）
When event 列のみから read model を再構築する
Then live-only delta を適用しなくても read model は完全に再構築できる
And live-only delta は durable event 列の一部として永続化されない
```

---

### Rule: session status は永続状態と runtime 遷移状態の双方を event 列から projection する

#### Scenario Outline: 実行フェーズを event 列から projection で導出する
```gherkin
Given session に <events> を表す durable event が append されている
When session status を projection する
Then session status は <status> となる

Examples:
  | events                            | status        |
  | turn 開始（user prompt 投入）      | streaming 中  |
  | tool 呼び出しに対する permission 要求 | permission 待ち |
  | turn 完了（中断・エラーなし）       | Idle          |
  | session のクローズ                 | Closed        |
```

#### Scenario: runtime 由来の遷移状態が event 列のみから判定できる
```gherkin
Given streaming 中・permission 待ちを表す durable event が append された session
When runtime memory を参照せず event 列のみから session status を projection する
Then streaming 中・permission 待ちの遷移状態が判定できる
And runtime memory が唯一の保持者である状態は存在しない
```

---

### Rule: abort / timeout / bridge crash 時に partial 状態を残さない（finalization）

#### Scenario Outline: 異常終端時に未完了の turn / tool call / permission を終端 event で閉じる
```gherkin
Given turn が進行中で、実行中の tool call と未解決の permission が存在する session
When <trigger> が発生する
Then 進行中の turn を終端する durable event が付与される
And 実行中の tool call を終端する durable event が付与される
And 未解決の permission を終端する durable event が付与される
And read model に partial な turn / tool call / permission は残らない

Examples:
  | trigger      |
  | abort（interrupt） |
  | timeout       |
  | bridge crash  |
```

#### Scenario: 異常終端後の read model から完了判定が一意に定まる
```gherkin
Given abort により終端 event が付与された session
When read model から turn の完了状態を判定する
Then turn は未完了（partial）ではなく終端済みとして判定される
And 完了判定が runtime state と session JSON のどちらを見るかで揺れない
```

---

### Rule: reconnect 時に read model を二重適用しない

#### Scenario: 同一 event 列の再 projection で message が重複しない
```gherkin
Given turn を含む durable event 列から projection 済みの read model
When UI reconnect により read model を event 列から再構築する
Then message は二重適用されず、再構築前と同じ内容になる
And 累積 parts の重複適用が発生しない
```

#### Scenario: abort / crash / reconnect の各シナリオで read model が決定的に再構築できる
```gherkin
Given abort / crash / reconnect のいずれかを経た session の durable event 列
When event 列から read model を再構築する
Then read model は event 列から決定的に再構築される
And partial 残存・二重適用は発生しない
```

---

### Rule: 既存の observable behavior を保持する（互換 projector）

#### Scenario: event 列駆動へ置き換えても既存の永続構造へ projection できる
```gherkin
Given event 列駆動で処理される Agent session
When read model を永続化する
Then 既存の session 永続構造（meta.json / messages/{seq}.json / index.json）へ projection される
And 既存の session 永続構造と矛盾するフォーマット変更は発生しない
```

#### Scenario: event 列駆動への置き換え後も UI 表示・session 一覧・streaming・workflow 完了判定の結果が変わらない
```gherkin
Given 既存の処理経路が durable event の append を介して read model へ反映される構造
When 同一の入力（prompt・tool 呼び出し・turn 完了）を与える
Then UI 表示内容は置き換え前と同じである
And session 一覧の結果は置き換え前と同じである
And streaming の見え方は置き換え前と同じである
And workflow 完了判定の結果は置き換え前と同じである
```

---

## 仮定（Assumptions）

- **B1.** 本ドキュメントの Scenario は read model（observable な派生状態）に対する振る舞いを定義し、durable event の具体的な variant 名・projector の内部構造・モジュール配置は実装仕様（`design.md`）に委ねる。requirements の R1〜R7 に含まれる関数名・型名（`accumulate_sdk_message` 等）は Gherkin 本文に持ち込まない。
- **B2.** 「streaming の見え方が変わらない」は cumulative snapshot 前提（#1214 の seq delta 移行は Non-goal）のもとでの不変を意味する。
- **B3.** session status の Examples（streaming 中 / permission 待ち / Idle / Closed）は代表例であり、`SessionState` の全状態（Active / Idle / Done / Error / Closed / Archived）と runtime 遷移状態の網羅的な対応表は `design.md` で確定する。
- **B4.** finalization の終端 event の具体的な variant（どの event でどう閉じるか）は `design.md` で定義する。本ドキュメントは「partial が残らない」という結果のみを規定する。
- **B5.** workflow turn-complete input の projection は、turn 完了を表す durable event の有無から導出され、runtime state（`BridgeState::Streaming` 相当）の直接参照を正典としない。

## Open Questions

なし（requirements の Open Questions はすべて解消済み。behavior レベルで新たに人間の判断を要する未確定点は生じていない）。
