# Behavior

FE-5（error banner の session スコープ化）の観測可能な振る舞いを Gherkin で定義する。実装詳細（reducer の action 名や state 構造）は振る舞いに含めず、ユーザーが観測できる banner の表示・残存・クリアの規則に絞る。

## Feature: session スコープの error banner

Agent チャットの error banner は、それを発生させた session の一時通知である。banner は発生元 session のパネルにのみ表示され、他 session の活動によって消えたり、他 session のパネルに混ざったりしない。banner が消えるのは、ユーザーの明示 dismiss と、同一 session における対象操作の成功に限られる。

### Background

```gherkin
Background:
  Given Agent チャットに複数の session が存在する
  And 各 session は自身のパネル（BoundSessionChat / ChatSessionView）で表示される
  And WorkflowView は複数 session のパネルを同時に表示できる
```

## Rule: banner は発生元 session に紐づく

ある操作の失敗で表示された banner は、その操作が属する session に紐づく。banner の表示・残存・クリアは、常にその session を単位として判定する。

```gherkin
Scenario: 操作失敗の banner が発生元 session のパネルに表示される
  Given session A のパネルを表示している
  When session A で操作（例: メッセージ送信）が失敗する
  Then session A のパネルにその失敗を示す banner が表示される

Scenario Outline: 各操作失敗が発生元 session に banner を表示する
  Given session A のパネルを表示している
  When session A で "<操作>" が失敗する
  Then session A のパネルに banner が表示される
  And 他 session のパネルにはその banner が表示されない

  Examples:
    | 操作                     |
    | メッセージ送信           |
    | session の読み込み       |
    | 過去メッセージの読み込み |
    | キューのキャンセル       |
    | session クローズ         |
    | session 復元             |
    | session アーカイブ       |
    | session フォーク         |
    | session タイトル変更     |
    | パーミッション応答       |
    | Agent の変更             |
```

session 一覧・初期化・作成・新規 session への初回送信は、失敗時点で対象 session を持たないため session banner の対象にしない。

## Rule: 表示スコープは発生元 session のパネルに限定される

banner は発生元 session のパネルにのみ表示する。複数 session を同時 mount する WorkflowView の pane grid でも、表示対象外の session の pane に banner が現れない。

```gherkin
Scenario: 別 session のエラーが表示中の session のパネルに混ざらない
  Given session A のパネルと session B のパネルが同時に表示されている
  When session B で操作が失敗し banner が表示される
  Then session B のパネルにのみ banner が表示される
  And session A のパネルには banner が表示されない

Scenario: 同時表示中の複数 session がそれぞれ独立した banner を持つ
  Given session A のパネルと session B のパネルが同時に表示されている
  And session A で操作が失敗し banner が表示されている
  When session B でも別の操作が失敗する
  Then session A のパネルには A の banner が表示されたままである
  And session B のパネルには B の banner が表示される
  And 2 つの banner は互いに独立している
```

## Rule: 他 session の活動では banner が消えない（無言クリアの廃止）

表示中の banner は、他 session の活動（別 session の turn 開始、別 session の読み込み成功、その他 banner と無関係なイベント）では消えない。

```gherkin
Scenario: 別 session の turn 開始で表示中の banner が消えない
  Given session A のパネルに banner が表示されている
  And session B が session A と同一 worktree に属する
  When session B が新しい turn を開始する
  Then session A のパネルの banner は表示されたままである

Scenario: 別 session の読み込み成功で表示中の banner が消えない
  Given session A のパネルに banner が表示されている
  When session B の読み込みが成功する
  Then session A のパネルの banner は表示されたままである

Scenario: banner と無関係なイベントで banner が消えない
  Given session A のパネルに banner が表示されている
  When session A に紐づかない、banner と無関係なイベントが発生する
  Then session A のパネルの banner は表示されたままである
```

## Rule: banner のクリア契機は 2 つに限定される

banner が消えるのは (a) ユーザーの明示 dismiss、(b) 同一 session における対象操作の成功、のみである。

```gherkin
Scenario: 明示 dismiss で banner が消える
  Given session A のパネルに banner が表示されている
  When ユーザーが session A の banner を dismiss する
  Then session A のパネルの banner が消える
  And 他 session のパネルの表示は変化しない

Scenario: 同一 session での対象操作の成功で banner が消える
  Given session A で送信が失敗し banner が表示されている
  When session A で送信が成功する
  Then session A のパネルの banner が消える

Scenario: 別 session での同種操作の成功では banner が消えない
  Given session A で送信が失敗し banner が表示されている
  When session B で送信が成功する
  Then session A のパネルの banner は表示されたままである
```

## Rule: session の除去時に banner エントリを破棄する

session がストアから除去される（close / remove 等）際は、その session の banner も破棄する。閉じた session の banner を残さない。

```gherkin
Scenario: session の除去でその session の banner が破棄される
  Given session A のパネルに banner が表示されている
  When session A がストアから除去される
  Then session A の banner エントリは残らない
  And 他 session のパネルの banner は影響を受けない
```

## Rule: session を持たない処理を active session に混線させない

background refresh や生成前処理の失敗は、実行時にたまたま active だった session の失敗として表示しない。

```gherkin
Scenario: background の session 一覧更新失敗が active session に混ざらない
  Given session A のパネルを表示している
  When background で session 一覧の更新が失敗する
  Then session A のパネルに一覧更新失敗の banner は表示されない

Scenario: session 作成失敗が以前の active session に混ざらない
  Given session A のパネルを表示している
  When 新しい session の作成が失敗する
  Then session A のパネルに session 作成失敗の banner は表示されない

Scenario: 新規 session への初回送信失敗が以前の active session に混ざらない
  Given session A のパネルを表示している
  When session 未指定の初回送信が失敗する
  Then session A のパネルにその送信失敗の banner は表示されない
```

## Rule: turn 由来のエラー表示は banner の対象外（非スコープの明示）

banner は「操作の失敗（送信・切替等）」という session スコープの一時通知に限定する。turn に紐づくエラーは durable な turn の part（正本経路）で表示し、banner はその表示器にしない。

```gherkin
Scenario: turn 内で発生したエラーは banner では表示されない
  Given session A で turn が進行している
  When turn の実行中にエラーが発生する
  Then そのエラーは turn の durable な表示（part）で示される
  And そのエラーは操作失敗 banner としては表示されない
```

## 仮定

- **操作分類・自己回復 policy・notice state は Rust usecase が所有する。** frontend は Rust が返す session 別 snapshot の mirror と描画、dismiss 入力だけを担当する。
- **「対象操作の成功」の粒度**は、banner を発生させた操作と同種の操作が同一 session で成功したときとする（例: 送信失敗 banner は同一 session の送信成功でクリア）。厳密な操作対応表は design で確定する。上記 Scenario では代表例として送信を用いる。
- **同一 session での対象操作の成功による自動クリア**は、Rust の操作 enum が一致した場合に適用する。別 session または別種操作の成功では clear しない。
- **banner の UI 見た目・文面・dismiss 操作の導線**は現状を踏襲し、変更しない。本 Feature はスコープ化とクリア規則のみを対象とする。
- **error メッセージの文面・エラー種別**は現状のものを維持し、追加・変更しない。

## Open Questions

なし
