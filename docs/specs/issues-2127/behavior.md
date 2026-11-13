## B-001: 未解決の Thread がカードで出る

GIVEN ファイルの行に、Comment を 2 件持つ未解決の Thread が付いている
WHEN 開発者がそのファイルをファイルタブの Diff 表示で開く
THEN Thread が付いた範囲の最後の行の下に、カードが出る
AND カードには、2 件の Comment が古い順に並ぶ

## B-002: File 表示でも未解決の Thread がカードで出る

GIVEN ファイルの行に、未解決の Thread が付いている
WHEN 開発者がそのファイルをファイルタブの File 表示で開く
THEN Thread が付いた範囲の最後の行の下に、カードが出る

## B-003: 人が書いた Comment の見出し

GIVEN 未解決の Thread に、人が 25 分前に書いた Comment がある
WHEN その Thread のカードが出る
THEN その Comment に、書き手の名前の頭文字のアイコン、書き手の名前、「25 分前」、本文が出る

## B-004: agent が書いた Comment の見出し

GIVEN 未解決の Thread に、agent が 3 分前に書いた Comment がある
WHEN その Thread のカードが出る
THEN その Comment に、agent を表す絵のアイコン、書き手の名前、「3 分前」、本文が出る
AND 書き手の名前の右に、その agent の model が出る

## B-005: Comment の本文が Markdown として整形される

GIVEN 未解決の Thread に、見出し、箇条書き、コード、強調を Markdown で書いた Comment がある
WHEN その Thread のカードが出る
THEN 本文は、見出し、箇条書き、コード、強調が整形されて出る
AND Markdown の記号は、書かれた文字のままでは出ない

## B-006: カードに行の場所と差分が出ない

GIVEN ファイルの行に、未解決の Thread が付いている
WHEN その Thread のカードが出る
THEN カードには、ファイルの path、行番号、差分は出ない

## B-007: 未解決の Thread のカードに返信の入力欄と「解決にする」が出る

GIVEN ファイルの行に、未解決の Thread が付いている
WHEN その Thread のカードが出る
THEN カードの下端に、「返信を書く…」と案内する入力欄と、「解決にする」ボタンが出る

## B-008: ⌘Return で返信を送る

GIVEN 未解決の Thread のカードの入力欄に、開発者が文を書いている
WHEN 開発者が ⌘Return を押す
THEN その文が、その Thread の最後の Comment としてカードに出る
AND 入力欄は空に戻る

## B-009: Return は改行になる

GIVEN 未解決の Thread のカードの入力欄に、開発者が文を書いている
WHEN 開発者が Return だけを押す
THEN 入力欄の中で改行する
AND 返信は送られない

## B-010: 「解決にする」で解決済みの 1 行に替わる

GIVEN 未解決の Thread のカードが出ている
WHEN 開発者が「解決にする」を押す
THEN その Thread は解決済みになる
AND カードは、解決にした人の名前、解決にした時刻の相対表示、「表示」ボタンを持つ 1 行に替わる

## B-011: 解決済みの Thread が 1 行で出る

GIVEN ファイルの行に、siro33950 が 1 時間前に解決にした Thread が付いている
WHEN 開発者がそのファイルをファイルタブの Diff 表示で開く
THEN Thread が付いた範囲の最後の行の下に、「siro33950 が解決にしました · 1 時間前」と「表示」ボタンを持つ 1 行が出る
AND その Thread の Comment は出ない

## B-012: File 表示でも解決済みの Thread が 1 行で出る

GIVEN ファイルの行に、解決済みの Thread が付いている
WHEN 開発者がそのファイルをファイルタブの File 表示で開く
THEN Thread が付いた範囲の最後の行の下に、解決にした人の名前、解決にした時刻の相対表示、「表示」ボタンを持つ 1 行が出る

## B-013: 「表示」で解決済みの Thread の Comment が出る

GIVEN 解決済みの Thread が 1 行で出ている
WHEN 開発者が「表示」を押す
THEN 1 行の下に、その Thread のすべての Comment が、未解決の Thread のカードと同じく、書き手のアイコン、書き手の名前、書いた時刻の相対表示、Markdown として整形した本文とともに古い順に出る
AND 「表示」は「隠す」に替わる
AND 返信の入力欄と「解決にする」ボタンは出ない

## B-014: 「隠す」で 1 行に戻る

GIVEN 解決済みの Thread を、開発者が「表示」で開いている
WHEN 開発者が「隠す」を押す
THEN Comment は消え、1 行だけが出る
AND 「隠す」は「表示」に替わる

## B-015: 「Thread作成」で行の下に Thread を作るカードが出る

GIVEN 開発者がファイルタブで行を選び、帯に「Thread作成」が出ている
WHEN 開発者が「Thread作成」を押す
THEN シートは開かない
AND 選んだ範囲の最後の行の下に、「コメントを書く…（Markdown）」と案内する入力欄、「キャンセル」ボタン、「Thread を作る」ボタンを持つカードが出る
AND 開発者が入力欄を押さずに打った文字が、入力欄に入る

## B-016: 「Thread を作る」で Thread ができる

GIVEN Thread を作るカードの入力欄に、開発者が文を書いている
WHEN 開発者が「Thread を作る」を押す
THEN 選んだ範囲に、その文を最初の Comment とする Thread ができる
AND Thread を作るカードは消える
AND できた Thread が、選んだ範囲の最後の行の下に、未解決の Thread のカードで出る

## B-017: 「キャンセル」で Thread を作るカードが消える

GIVEN Thread を作るカードが出ている
WHEN 開発者が「キャンセル」を押す
THEN Thread を作るカードは消える
AND Thread はできない

## B-018: 本文の画像がファイルタブに出る

GIVEN 未解決の Thread に、URL で画像を書いた Comment がある
WHEN その Thread のカードがファイルタブに出る
THEN その画像が、Comment の本文の中に出る

## B-019: 本文の画像が Thread 面に出る

GIVEN Thread に、URL で画像を書いた Comment がある
WHEN 開発者が Thread 面でその Comment を表示する
THEN その画像が、Comment の本文の中に出る

## B-020: ファイルタブのリンクが既定のブラウザで開く

GIVEN ファイルタブのカードに、リンクを書いた Comment が出ている
WHEN 開発者がそのリンクを押す
THEN リンク先が既定のブラウザで開く

## B-021: Thread 面のリンクが既定のブラウザで開く

GIVEN Thread 面に、リンクを書いた Comment が出ている
WHEN 開発者がそのリンクを押す
THEN リンク先が既定のブラウザで開く

## B-022: 本文の HTML と script がファイルタブで動かない

GIVEN 未解決の Thread に、返信を送る script を含む HTML を書いた Comment がある
WHEN その Thread のカードがファイルタブに出る
THEN その script は動かない
AND その Thread に返信は加わらず、Thread は未解決のままで、新しい Thread もできない

## B-023: 本文の HTML と script が Thread 面で動かない

GIVEN Thread に、script を含む HTML を書いた Comment がある
WHEN 開発者が Thread 面でその Comment を表示する
THEN その script は動かない
AND その Thread に返信は加わらず、Thread の解決の状態は変わらず、新しい Thread もできない

## B-024: model が記録されていない agent の Comment の見出し

GIVEN 未解決の Thread に、model が記録されていない agent の Comment がある
WHEN その Thread のカードが出る
THEN その Comment に、agent を表す絵のアイコン、書き手の名前、書いた時刻の相対表示、本文が出る
AND 書き手の名前の右に、model は出ない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-004, B-024 |
| R-003 | B-005 |
| R-004 | B-006 |
| R-005 | B-007 |
| R-006 | B-008, B-009 |
| R-007 | B-010 |
| R-008 | B-011, B-012 |
| R-009 | B-013, B-014 |
| R-010 | B-015 |
| R-011 | B-016 |
| R-012 | B-017 |
| R-013 | B-018, B-019 |
| R-014 | B-020, B-021 |
| R-015 | B-022, B-023 |
