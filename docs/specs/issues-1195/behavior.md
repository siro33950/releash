# Behavior

requirements.md を観測可能な振る舞いとして定義する。本 ISSUE の中核は「webview が常駐保持する message body 量を有界化する内部最適化であり、外部観測可能な振る舞いは変えない」ことなので、Gherkin は退避機構の内部詳細（退避単位が drop か軽量プレースホルダ化か、トリガが非アクティブ化時か上限超過時か未参照時間ベースか、保持上限件数・session 数の具体値）を持ち込まない。それらは `design.md` の裁量で確定する実装詳細であり、ここではユーザーまたは表示系から外部観測できる結果に絞って記述する。

## 用語

- **session**: 1 件の agent チャット会話。Rust 側 session ストアが正本を保持し、webview の保持はそこから再取得可能なキャッシュである。
- **アクティブ session**: 現在 UI に表示中の session（`AgentChatPanel` に開いて見えている会話）。
- **非アクティブ session**: 開いた状態にあるが現在は表示していない session（切り替えて背面に回った会話）。close/delete はされていない。
- **可視範囲**: アクティブ session のうち、現在スクロール位置から表示している message レンジ。これに加え、ストリーミング中・ターン進行中のアクティブなレンジを含む。
- **スクロールバック**: 過去方向へ遡って、初回ページ（最新 50 件）より前の古い message を読み込む操作。`get_session_page(session_id, cursor, limit)` を再供給経路として用いる。
- **退避（eviction）**: 可視範囲外の message body を webview の常駐保持から外すこと。退避しても Rust 正本は不変。
- **再供給（再 hydrate）**: 退避済みレンジが再表示対象になった時点で、`get_session_page` により Rust 正本から再取得して表示を復元すること。
- **有界**: 会話の総 message 数および同時に開いている session 数に対して、webview の常駐 message body 量が線形に増え続けない状態。

## Feature: 開いている agent チャットの常駐メモリ有界化

長い会話のスクロールバックや複数 session の同時オープンによって、webview が保持する message body 量が無制限に増え続ける経路を是正する。退避と再供給により常駐量を有界に保ちつつ、ユーザーから見た表示・スクロールバック・既読履歴・ストリーミング更新の外部観測可能な振る舞いは退避前と同一に保つ。

### Background

```gherkin
Given Releash デスクトップアプリで agent チャット（AgentChatPanel 系）を開いている
And session の message 正本は Rust 側 session ストアが保持している
And webview は初回ロード時に最新 50 件（INITIAL_SESSION_PAGE_LIMIT）で hydrate している
And 過去方向のスクロールバックは get_session_page により追加ページを取得できる
```

## Rule: アクティブ session のスクロールバック累積は上端離脱後に有界へ収束する

長い単一会話を遡っても、webview が常駐保持する message body 量が会話の総 message 数に対して線形に増え続けない。連続した純上方向スクロールバック中（`oldest_visible_index` が 0 近傍）は、いま読み込んだ古い prefix を即退避しないため contiguous な窓が一時的に cap を超えて増大しうる。ユーザーが上端を離れた後（下方向スクロール）または present へ戻った時点で、drop 可能な古い page は退避され、live tail が `RETAINED_MESSAGE_CAP` 以下なら常駐量は cap へ収束する。live tail（最新 page・進行中レンジ）自体が cap を超える場合、その超過分は進行中表示を守るため退避せず、古い page のみ退避する。

```gherkin
Scenario: 長い会話を深くスクロールバックしても常駐 message body 量が有界に保たれる
  Given 多数の message を持つ単一 session を開いている
  When 過去方向へ繰り返しスクロールバックして古いページを読み込み続ける
  Then 連続した純上方向スクロールバック中は contiguous な窓が一時的に cap を超えて増大しうる
  When 上端を離れて古い prefix が可視範囲外になる
  Then drop 可能な古い message body は退避される
  And live tail が RETAINED_MESSAGE_CAP 以下なら webview が常駐保持する message body 量は RETAINED_MESSAGE_CAP へ収束する
  And live tail が RETAINED_MESSAGE_CAP を超える場合は live tail を保持したまま古い page のみ退避される
```

```gherkin
Scenario: スクロールバックで遡った古いレンジへ再び戻ると同一内容が復元される
  Given スクロールバックで古いレンジを表示したのち、さらに別レンジへ移動して当該レンジが退避された
  When 退避された古いレンジへ再びスクロールして戻る
  Then 退避前と同一の message が同一順序で表示される
  And 重複・欠落・順序崩れは発生しない
```

## Rule: 非アクティブ session の常駐量が有界である

session を切り替えて複数を開いても、背面に回った非アクティブ session の body が常駐し続けず、同時保持量が session 数に対して線形に増え続けない。

```gherkin
Scenario: 複数 session を切り替えて開いても常駐 message body 量が有界に保たれる
  Given 複数の session を順に開いて切り替えて使っている
  When 開く session 数を増やしていく
  Then webview が常駐保持する message body 量は同時に開いた session 数に比例して増え続けない
  And 表示していない非アクティブ session の message body は退避される
```

```gherkin
Scenario: 退避された非アクティブ session に戻ると表示が復元される
  Given ある session を表示後に別 session へ切り替え、元の session が非アクティブとして退避された
  When 退避された元の session へ表示を切り替えて戻る
  Then 切り替え前と同一の表示・スクロール可能な履歴が復元される
  And 重複・欠落・順序崩れは発生しない
```

## Rule: 退避は close/delete を伴わず、再供給は正本から行う

退避は webview 内部のキャッシュ解放であって session の終了ではなく、再供給は Rust 正本の既存 paging API のみを用いる。

```gherkin
Scenario: 退避は session の close/delete を引き起こさない
  Given アクティブ／非アクティブを問わず session の一部または全部の body が退避される
  Then 当該 session は close/delete されず、開いた状態を保つ
  And Rust 側の session 正本は退避によって変化しない
```

```gherkin
Scenario: 再供給は既存の get_session_page のみで完結する
  Given 退避済みレンジが再表示対象になった
  When 退避済みレンジを復元する
  Then 再取得は get_session_page（session_id, cursor, limit）経由で行われる
  And 新規の read API・永続データ型・プロトコル型は追加されない
```

## Rule: ストリーミング中・ターン進行中の表示更新が退行しない

退避対象はアクティブな進行中レンジを含めない。応答途中の表示やターン完了の確定が退避/仮想化によって壊れない。

```gherkin
Scenario: ストリーミング中のアクティブ session が退避対象にならない
  Given アクティブ session で turn が応答中（ストリーミング中）である
  When 退避処理が走りうるタイミングになる
  Then 進行中の turn とその可視レンジは退避されない
  And ストリーミングの逐次表示更新は従来どおり反映される
```

```gherkin
Scenario Outline: ターン進行に伴う表示更新が退避導入後も不変である
  Given アクティブ session で turn が進行している
  When <更新> が発生する
  Then 退避導入前と同一に表示へ反映される

  Examples:
    | 更新                        |
    | ストリーミング途中のメッセージ更新 |
    | 新規メッセージの追加          |
    | ターン完了の確定             |
```

## Rule: 既読・履歴の外部観測可能な振る舞いが退避前と同一である

退避はあくまで内部最適化であり、ユーザーから見える既読履歴・スクロール体験は退避前後で区別できない。

```gherkin
Scenario: 退避と再供給を経ても既読履歴が変化しない
  Given session の一部レンジが退避され、のちに再供給された
  When 当該レンジを表示する
  Then 退避前と同一の既読／履歴状態が再現される
  And ユーザーは退避が起きたことを表示上で区別できない
```

## 仮定

- session ストア（Rust）が message の正本であり、webview の保持は再取得可能なキャッシュである。退避した body の再供給に整合性問題は生じない（#1213 の `get_session_page` が runtime streaming overlay・token usage metadata を含めて正本を返す前提）。
- 退避の単位（body 全体 drop か軽量プレースホルダ化か）、退避トリガ（非アクティブ化時の即時／保持 message 数・session 数の上限超過時／未参照時間ベース）、および保持上限の具体値は、本 behavior の不変条件（有界化・表示不変・再供給整合）を満たす範囲で `design.md` の裁量により確定する。本 behavior はこれらの内部詳細に依存しない。
- 退避判定・再供給に必要なロジックは `rust-first-logic` 方針に従って配置し、フロントは表示・入力・invoke・表示用フォーマットに徹する。フロントに新たなビジネスロジックを増やさない。
- 退避/再供給は #1213 が確定した paging 契約（`initialPage` / `get_session_page` / cursor / `INITIAL_SESSION_PAGE_LIMIT`）の上に構築し、ページング方式の再設計・二重実装を生まない。
- 対象は agent チャット（`AgentChatPanel` 系）であり、remote UI（`src/remote/`）は本 ISSUE の主対象外。
- 「有界」の計測手順・観測点・具体的な保持上限値は `design.md` で定義する。本 behavior は「総 message 数・同時 session 数に対して線形に増え続けない」という外部観測可能な性質のみを固定する。

## Open Questions

なし
