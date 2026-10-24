## B-001: 復元後の会話のスクロール

GIVEN Codex がマウス報告を有効にし、会話のスクロールを受け付けている Session がある
WHEN ネイティブ版が snapshot から Session を復元し、利用者がマウスホイールまたはトラックパッドで会話をスクロールする
THEN Codex は設定していた報告方式と座標形式でスクロール操作を受け取り、会話の表示がスクロールする
AND 入力欄はその操作によって送信履歴へ置き換わらない

## B-002: 復元後の alternate scroll の維持

GIVEN Codex が alternate screen を使用し、マウス報告を無効、alternate scroll を無効に設定している
WHEN ネイティブ版が snapshot から Session を復元し、利用者がホイール操作をする
THEN Codex に上下キーの入力は送られない

## B-003: 復元後の貼り付け方式

GIVEN Codex が bracketed paste の有効・無効を設定している
WHEN ネイティブ版が snapshot から Session を復元し、利用者が内容を貼り付ける
THEN Codex は復元を挟まない場合と同じ bracketed paste の設定に従った入力を受け取る

## B-004: 復元後の focus 通知

GIVEN Codex が focus 通知の有効・無効を設定している
WHEN ネイティブ版が snapshot から Session を復元し、利用者がフォーカスを移す
THEN Codex への通知の有無と内容は復元を挟まない場合と同じになる

## B-005: 復元後のキー入力方式

GIVEN Codex が kitty keyboard の入力方式を設定している
WHEN ネイティブ版が snapshot から Session を復元し、利用者がキー操作をする
THEN Codex は復元を挟まない場合と同じ方式のキー入力を受け取る

## B-006: 画面ごとのキーボード設定への復帰

GIVEN Codex が通常画面と alternate screen でそれぞれキーボード設定を保存し、入力方式を変更している
WHEN snapshot からの復元後に Codex が画面を切り替え、保存していた設定へ戻し、利用者がキー操作をする
THEN Codex はその画面で復元を挟まない場合と同じ方式のキー入力を受け取る

## B-007: 復元後の入力設定の変更

GIVEN Codex が設定した入力方式を snapshot から復元した Session がある
WHEN Codex が対象の入力方式を変更または解除し、利用者が対応する操作をする
THEN Codex が受け取る入力または通知は、復元を挟まず同じ設定変更を受信した場合と同じになる

## B-008: 保存された状態からの入力方式の復元

GIVEN Codex の入力設定を本変更後に受信して保存した端末状態、またはその設定の復元に必要な情報が残る旧保存状態がある
WHEN その保存状態から作り直した snapshot で Session を復元し、利用者がマウス操作、貼り付け、フォーカス移動またはキー操作をする
THEN Codex が受け取る入力または通知は、その保存状態までの出力を継続して受信した場合と同じになる
AND その後の入力設定の変更または解除も、復元を挟まない場合と同じように反映される

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004 |
| R-004 | B-005, B-006 |
| R-005 | B-007, B-008 |
