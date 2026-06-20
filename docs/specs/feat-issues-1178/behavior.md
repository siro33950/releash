# Behavior

requirements.md を観測可能な振る舞いとして定義する。実装の内部経路（bridge / runtime のモジュール名・関数名・内部状態名）は持ち込まず、ユーザーまたは次 turn の送信者が外部から観測できる結果に絞って記述する。

## 用語

- **turn**: ユーザーが 1 メッセージを送ってから、その応答が完了するまでの 1 単位。
- **進捗イベント**: agent の応答が前進していることを示す出力。thinking の更新、ツール実行、progress 通知、応答テキストの増分を含む。
- **stale（無音停止）**: turn が完了せず、かつ進捗イベントも届かない状態が続くこと。
- **stale timeout**: 応答中（Thinking 表示中）の turn を stale と判定するまでの無進捗許容時間。既定 180 秒。
- **permission timeout**: 権限承認待ちの turn を timeout と判定するまでの待機許容時間。既定 300 秒。headless 経路のみ適用。

## Feature: Claude agent turn の無音停止からの回復

Claude（Opus 4.8 等）との通信が応答も終了通知も返さず無音停止しても、agent turn が永続的に Thinking 表示のまま固まらず、一定時間後に失敗として完了し、ユーザーが再試行できる。

### Background

```gherkin
Given Releash デスクトップアプリで Claude と対話している
And agent session が開始している
And ユーザーがメッセージを送信して turn が応答中（Thinking 表示）になっている
```

### Rule: 応答中の turn が無進捗のまま stale timeout を超えると失敗完了する

```gherkin
Scenario: 終了通知も進捗も返らず無音停止した turn が stale timeout 後に失敗完了する
  Given turn が応答中である
  When 進捗イベントが一切届かないまま stale timeout（180 秒）を超過する
  Then 当該 turn は失敗状態として完了する
  And UI は永続 Thinking 表示から復帰する
  And UI に「Claude 応答が停止したため中断した。再試行できる」旨が表示される
  And 同一セッションで次のメッセージを送信できる状態になる
```

```gherkin
Scenario: stale 完了時にそれまでの部分出力が保持される
  Given turn が応答中で、一部のテキストや thinking がすでに表示されている
  When 進捗のないまま stale timeout を超過して turn が失敗完了する
  Then それまでに表示された部分出力は失われず残る
```

### Rule: 進捗イベントが届く間は stale 判定しない（正当に長い thinking を誤中断しない）

```gherkin
Scenario Outline: 進捗イベントが届き続ける限り timeout しない
  Given turn が応答中である
  When stale timeout 未満の間隔で <イベント> が届き続ける
  Then turn は stale と判定されず Thinking 表示を継続する
  And turn は中断されない

  Examples:
    | イベント            |
    | thinking の更新     |
    | ツール実行          |
    | progress 通知       |
    | 応答テキストの増分  |
```

```gherkin
Scenario: 最後の進捗から無進捗時間で stale を判定する
  Given turn が応答中で、過去に進捗イベントを受信している
  When 最後の進捗イベントから新たな進捗が stale timeout を超えて届かない
  Then 当該 turn は stale と判定され失敗完了する
```

```gherkin
Scenario: api/request の補助ログだけでは進捗とみなさない
  Given turn が応答中である
  When 応答や thinking の前進を伴わない補助的なログのみが出力される
  And それ以外の進捗イベントは届かない
  Then 当該ログは進捗とみなされず、stale timeout 超過で turn は失敗完了する
```

### Rule: 終了通知なしに通信が閉じた turn は正常終了扱いにしない

```gherkin
Scenario: 終了通知を受信しないまま通信が閉じても失敗完了する
  Given turn が応答中である
  When 終了通知（result）を受信しないまま agent との通信が閉じる
  Then 当該 turn は正常終了扱いにならず失敗状態として完了する
  And 壊れた接続状態は次の turn に持ち越されない
  And 同一セッションで次のメッセージを送信できる
```

### Rule: stale 検出時に通信プロセスを停止・回復し、壊れた状態を次 turn に残さない

```gherkin
Scenario: stale 検出時に通信を中断し、回復しなければプロセスを停止・再生成する
  Given turn が応答中で stale と判定された
  When 回復のための中断要求を送る
  And 一定時間で turn が解消しない
  Then 通信プロセスは停止される
  And 必要に応じて通信プロセスが再生成される
  And 壊れた接続状態は次の turn に持ち越されない
```

### Rule: 権限承認待ちの timeout は headless 経路のみ適用する

```gherkin
Scenario: headless 経路の権限待ちは permission timeout で打ち切られる
  Given headless 経路で turn が権限承認待ちになっている
  When 承認も拒否もされないまま permission timeout（300 秒）を超過する
  Then 当該 turn は timeout として失敗完了する
  And 同一セッションで次のメッセージを送信できる状態になる
```

```gherkin
Scenario: デスクトップ経路の権限待ちは無期限に待つ
  Given デスクトップ経路で turn が権限承認待ちになっている
  When permission timeout 相当の時間が経過しても承認も拒否もされない
  Then turn は timeout せず権限承認待ちを継続する
```

### Rule: Stop と stale timeout が競合しても二重完了しない

```gherkin
Scenario: ユーザー Stop と stale timeout が同時に発生しても turn は一度だけ完了する
  Given turn が応答中で stale 判定の直前である
  When ユーザーが Stop 操作を行う
  And ほぼ同時に stale timeout が発生する
  Then turn は一度だけ完了する
  And 二重完了・二重エラーは発生しない
  And 同一セッションで次のメッセージを送信できる状態になる
```

### Rule: 正常系の turn 完了挙動は変わらない

```gherkin
Scenario: 終了通知を正常に受信した turn は従来どおり完了する
  Given turn が応答中である
  When 進捗イベントののち終了通知（result）を受信する
  Then 当該 turn は正常完了する
  And stale 判定や中断は発生しない
  And 同一セッションで次のメッセージを送信できる
```

## 仮定

- stale timeout は固定値 180 秒、permission timeout は固定値 300 秒とし、ユーザー設定 UI は設けない（requirements の仮定に準拠）。
- 失敗完了後の回復はユーザーの手動再試行で行い、自動リトライ・自動 resume は行わない。
- 補助的な api/request ログ（応答・thinking の前進を伴わないもの）は進捗イベントに含めない。
- 本振る舞いの対象は Claude Agent SDK 経路に限定する。他の agent backend は対象外。
- permission timeout は headless 経路のみに適用し、デスクトップ経路では権限承認を無期限に待つ。

## Open Questions

なし
