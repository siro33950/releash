## B-001: ツリーの全階層の既定展開

GIVEN Sequence、Fanout の入れ子と、delegate の子を持つ Session が実行木にある
WHEN 開発者が Swift.app の Node 面でその実行木を初めて表示する
THEN ツリーの開閉矢印で開閉する全ての行が既定で展開される
AND 行を開く操作をしなくても、全階層の子が表示される

## B-002: 過去の試行の初期表示

GIVEN Retry 前の履歴を持つ Node が実行木にある
WHEN 開発者が Swift.app の Node 面でその実行木を初めて表示する
THEN 過去の試行の履歴は非表示である

## B-003: 過去の試行の表示切り替え

GIVEN delegate の子と Retry 前の履歴を持つ Session が Node 面にある
WHEN 開発者が「過去の試行」ボタンを操作する
THEN 過去の試行の履歴の表示と非表示が切り替わる
AND delegate の子の開閉状態は変わらない

## B-004: ツリーの開閉と履歴の独立

GIVEN delegate の子と Retry 前の履歴を持つ Session が Node 面にある
WHEN 開発者がその行のツリーの開閉矢印を操作する
THEN delegate の子が開閉する
AND 過去の試行の履歴の表示と非表示は変わらない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003, B-004 |
