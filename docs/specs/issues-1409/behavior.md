# Behavior

## Source
- requirements.md

## 仮定

- 可視化は session-scoped バナー＋構造化ログとして観測する。バナーは backend-owned state から供給され、frontend は表示に徹する（requirements A2）。
- リトライ回数・backoff 間隔の具体値は design node で確定するため、本振る舞いでは「短い backoff で有限回リトライ後にエラー伝搬」という観測可能な結果のみを固定する（requirements A4）。
- RT-8 の対象は tool 使用等の durable part を含む turn で、最終永続化の append だけが失敗するケースとする。純テキスト turn は対象外（requirements A5）。
- エラー注入・破損 fixture はテスト専用機構を用い、実 I/O 障害や外部プロセスはテストで発生させない（requirements A6）。

## Behavior

```gherkin
# language: ja
機能: persist 失敗の可視化と event log 自己修復

  背景:
    前提 Agent チャット（Claude / Codex）のセッションが永続化経路を持つ
    かつ セッションのバナー状態は backend が所有し frontend へ供給される

  ルール: 永続化失敗を無言に握りつぶさない（ST-4）

    シナリオ: 一時的な永続化失敗はリトライで回復する
      前提 セッションで永続化操作が発生する
      もし 永続化が一時的に失敗した後、リトライ内で成功する
      ならば その操作は成功として完了する
      かつ ユーザー操作はエラーにならない

    シナリオ: リトライ後も継続する永続化失敗は可視化されエラー伝搬する
      前提 セッションで永続化操作が発生する
      もし リトライを尽くしても永続化が失敗し続ける
      ならば 失敗が呼び出し元へエラーとして伝搬し当該操作がエラー化する
      かつ 失敗が session-scoped バナーとしてユーザーへ可視化される
      かつ 失敗が構造化ログへ記録される

    シナリオ: 永続化失敗が沈黙のまま状態を進めない
      前提 セッションで永続化操作が失敗する
      もし in-memory 状態のみが更新され durable 状態が更新されていない
      ならば その乖離を無言のまま後続処理へ進めない

  ルール: 破損した event log を append 側で自己修復する（RT-4）

    シナリオ: 末尾破損した event log を持つセッションが送信可能へ回復する
      前提 セッションの event log がクラッシュ等で末尾破損（欠け `]`・中途行）している
      もし そのセッションで新たな append（メッセージ送信を含む）を行う
      ならば event log が append 前に修復される
      かつ 以降の append が通常どおり成功する
      かつ 当該チャットでメッセージ送信が再び成功する

    シナリオ: 修復の事実が観測できる
      前提 破損した event log が修復される
      もし 修復が行われる
      ならば 修復の事実が構造化ログへ記録される
      かつ 修復の事実がユーザーへ可視化される

  ルール: 最終永続化失敗時に persist 済み本文を失わない（RT-8）

    シナリオ: 最終永続化の失敗で本文が tool 履歴だけへ置換されない
      前提 turn が Text / Thinking を含む本文を persist 済みである
      かつ その turn が durable part（tool 使用等）を含む
      もし turn 完了時の最終永続化（FinalPartsRecorded の append）が失敗する
      ならば persist 済み本文が projection 由来の tool-only parts で上書きされない
      かつ persist 済み本文を保持したまま再試行する
      かつ reload しても本文が失われない
```

## Open Questions

なし
