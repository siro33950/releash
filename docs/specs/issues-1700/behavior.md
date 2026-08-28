## B-001: agent が動いている Session Node は青

GIVEN Session を持つ Node が存在する
AND その Node は recovery fence を持たず、失敗しておらず、実行中または承認待ちである
WHEN その Session の agent が動いている
THEN その Node のツリー行は青（実行中）で表示される

## B-002: 次の指示を待って止まっている Session Node は黄

GIVEN Session を持つ Node が存在する
AND その Node は recovery fence を持たず、失敗しておらず、実行中または承認待ちである
WHEN agent が応答を終えて次の指示を待って止まっている
THEN その Node のツリー行は黄（介入が必要）で表示される

## B-003: 追加指示による黄から青への復帰

GIVEN ツリー行が黄で表示されている Session Node が存在する
WHEN ユーザーが追加指示を送り、agent が再び動き出す
THEN その Node のツリー行は青へ戻る

## B-004: 応答終了と再開の反復

GIVEN 黄と青を往復した Session Node が存在する
WHEN agent の応答終了と再開が繰り返される
THEN ツリー行は応答終了のたびに黄になり、再開のたびに青になる
AND 往復の回数に上限はなく、何回目の往復でも同じ結果になる

## B-005: permission の承認待ちは黄

GIVEN Session を持つ Node が存在する
AND その Node は recovery fence を持たず、失敗しておらず、実行中または承認待ちである
WHEN agent が permission の承認を待って止まり、provider の承認ダイアログが表示されている
THEN その Node のツリー行は黄で表示される
AND 承認ダイアログが表示されている間、ツリー行は青にならない

## B-006: 質問への回答待ちは黄

GIVEN Session を持つ Node が存在する
AND その Node は recovery fence を持たず、失敗しておらず、実行中または承認待ちである
WHEN agent が質問への回答を待って止まり、provider の質問が表示されている
THEN その Node のツリー行は黄で表示される
AND 質問が表示されている間、ツリー行は青にならない

## B-007: 承認待ちの Session Node は agent が動いていれば青

GIVEN 承認待ちの Session Node が存在する
AND その Node は recovery fence を持たず、失敗していない
WHEN 承認しないまま追加指示を送り、agent が動いている
THEN その Node のツリー行は青で表示される
AND 承認待ちであることを理由に黄にならない

## B-008: 承認要求の判別手段

GIVEN 承認待ちの Session Node のツリー行が青で表示されている
WHEN その Node の操作可否と Node 詳細を参照する
THEN 承認操作が可能であることが分かる
AND Node 詳細からその Node が承認待ちであることが分かる

## B-009: Stop 信号を受領済みでも agent が動いていれば青

GIVEN 実行中の Session Node が Stop 信号を受領し、Submit 信号を受領していない
AND その Node は recovery fence を持たず、失敗していない
WHEN agent が動いている
THEN その Node のツリー行は青で表示される

## B-010: Stop 信号が未受領でも agent が止まっていれば黄

GIVEN 実行中の Session Node が Stop 信号を受領していない
AND その Node は recovery fence を持たず、失敗していない
WHEN agent が人の回答または次の指示を待って止まっている
THEN その Node のツリー行は黄で表示される

## B-011: 失敗と recovery fence は活動状態より優先される

GIVEN Session を持つ Node が失敗している、または recovery fence を持つ
WHEN その Session の agent が動いている、または止まっている
THEN その Node のツリー行はいずれの場合も赤（失敗）で表示される

## B-012: 完了・中止・停止は活動状態より優先される

GIVEN Session を持つ Node が完了、中止、停止のいずれかに達している
AND その Node は recovery fence を持たず、失敗していない
WHEN その Session の agent が動いている、または止まっている
THEN その Node のツリー行はいずれの場合も緑（動いていない）で表示される

## B-013: 正常終了しなかった Session Node は赤

GIVEN Session を持つ Node が完了に達していない
WHEN その Session の provider プロセスが正常終了せずに終わる
THEN その Node のツリー行は赤で表示される
AND 緑では表示されない

## B-014: 正常終了した Session Node は緑

GIVEN Session を持つ Node が完了に達していない
AND その Node は承認待ちではない
WHEN その Session の provider プロセスが正常終了する
THEN その Node のツリー行は緑で表示される

## B-015: Node の完了に伴って停止した Session Node は緑

GIVEN Session を持つ Node が完了に達する
WHEN その完了に伴って Releash が provider プロセスを停止する
THEN その Node のツリー行は緑で表示される

## B-016: Command Node の色の維持

GIVEN Session を持たない Command Node が存在する
WHEN 変更の前後で同じ Command Node のツリー行を比較する
THEN 実行中の Command Node は変更の前後とも青で表示される
AND 承認待ちの Command Node は変更の前後とも黄で表示される

## B-017: 親行の重大度集約の維持

GIVEN Sequence または Fanout の親行に、自分自身と配下の子の分類結果が存在する
WHEN 変更の前後で同じ組み合わせの分類結果を比較する
THEN 親行は自分自身と配下の子を合わせた重大度順で最も重い分類の色で表示される
AND 集約の結果は変更の前後で変わらない

## B-018: provider による差がない

GIVEN Claude Code の Session を持つ Node と Codex CLI の Session を持つ Node が存在する
WHEN 両者の agent が同じ活動状態にあり、Node の状態も等しい
THEN 両者のツリー行は同じ色で表示される
AND B-001 から B-015 の結果は双方の provider で同じように成立する

## B-019: 実行木上の位置と起点による差がない

GIVEN Session を持つ Node が実行木の root である場合と child である場合が存在する
AND その実行木が workflow の実行として起こされた場合と Session の起動として起こされた場合が存在する
WHEN agent が同じ活動状態にあり、Node の状態も等しい
THEN いずれの組み合わせでもツリー行は同じ色で表示される
AND B-001 から B-015 の結果はいずれの組み合わせでも同じように成立する

## B-020: provider の承認判定への非介入

GIVEN permission の承認を要求する Session Node が存在する
WHEN Releash が agent の活動状態を観測する
THEN permission の承認可否は provider 自身が決める
AND provider の承認 UI は現行どおり表示される
AND Releash が承認や回答を代行しない

## B-021: 活動状態の事実は遷移ごとに 1 件

GIVEN Session の agent が活動状態を遷移させる
WHEN 一連の観測の後に記録された活動状態の事実を数える
THEN 事実の件数は実際に起きた活動状態の遷移の回数と等しい
AND 同一の活動状態を繰り返し観測しても事実は追記されない

## B-022: 再起動後の色の再現

GIVEN 活動状態が記録された Session Node が存在する
WHEN アプリケーションを再起動して Workspace ツリーを表示する
THEN 各 Session Node のツリー行の色は、記録済みの活動状態から B-001 から B-015 と同じ規則で導かれる色になる

## B-023: 完了判定の維持

GIVEN 実行中の Session Node が存在する
WHEN Submit 信号と Stop 信号の受領状況、および agent の活動状態が変化する
THEN その Node が完了と判定されるのは Submit 信号と Stop 信号の両方が揃ったときだけである
AND agent の活動状態は完了判定を変えない

## B-024: 操作可否と resume 不能理由の維持

GIVEN 同じ Workspace と実行木の状態が存在する
AND その Session Node の Node 状態の導出は変更の前後で同じである
WHEN 変更の前後で利用可能な操作と resume 不能理由を比較する
THEN approve、retry、stop、abort、archive の各操作可否は変わらない
AND resume の可否は B-029 が定める場合を除いて変わらない
AND resume 不能理由の内容は変わらない

## B-025: 活動状態は操作可否に影響しない

GIVEN Session を持つ Node が存在する
WHEN Node の状態は変わらないまま agent の活動状態だけが遷移する
THEN approve、retry、stop、resume、abort、archive の各操作可否は変わらない
AND resume 不能理由の内容は変わらない

## B-026: AgentSession の lifecycle 判定の維持

GIVEN AgentSession が存在する
WHEN archive、restore、delete、GC、initial instruction の受理可否が判定される
THEN 判定は現行どおり open、paused、archived の lifecycle に従う
AND agent の活動状態は判定を変えない

## B-027: local API と CLI の応答維持

GIVEN 同じ要求を local API または CLI へ送る
WHEN 変更の前後で応答を比較する
THEN local API の応答は変わらない
AND CLI の応答は変わらない

## B-028: 活動が観測される前の Session Node は黄

GIVEN Session を持つ Node が存在する
AND その Node は recovery fence を持たず、失敗しておらず、実行中または承認待ちである
WHEN その Session が起動してから、まだ agent の活動が一度も観測されていない
THEN その Node のツリー行は黄で表示される
AND 停止した provider プロセスを resume した直後も黄で表示される

## B-029: 正常終了しなかった Session Node は resume できる

GIVEN Session を持つ Node の provider プロセスが正常終了せずに終わっている
WHEN その Node の操作可否を参照する
THEN resume が可能である
AND 実行木が workflow の実行として起こされた場合と Session の起動として起こされた場合のいずれでも resume が可能である
WHEN resume を要求する
THEN 既存の provider session を使った provider プロセスの復旧が実行される
AND provider プロセスの復旧が成立した場合にだけ Node は実行中になる
AND provider プロセスを復旧できない場合は resume が失敗し、Node は失敗状態のまま残る
AND provider が動作中の Paused Session Node は provider を再起動せず実行中へ戻る

## B-030: AgentSession の応答から `activity` が消える

GIVEN AgentSession の詳細を要求する
WHEN 変更の前後で応答を比較する
THEN 変更後の応答は terminal 出力の recency から導出する `activity` を含まない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-004 |
| R-004 | B-005, B-006 |
| R-005 | B-007, B-008 |
| R-006 | B-009, B-010 |
| R-007 | B-011, B-012 |
| R-008 | B-002, B-013, B-014, B-015 |
| R-009 | B-016 |
| R-010 | B-017 |
| R-011 | B-018 |
| R-012 | B-019 |
| R-013 | B-020 |
| R-014 | B-021 |
| R-015 | B-022 |
| R-016 | B-023 |
| R-017 | B-024, B-025 |
| R-018 | B-026 |
| R-019 | B-027 |
| R-020 | B-028 |
| R-021 | B-029 |
| R-022 | B-030 |
