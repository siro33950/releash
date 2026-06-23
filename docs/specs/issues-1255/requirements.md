# Requirements

## Type

不具合修正。

関連: #1255 / #1178 / #731 / #1192 / #1246

## 背景と目的

AgentChat の Claude backend で、応答中に Stop ボタンを押して停止し、UI が送信ボタンに戻った後に新しいメッセージを送ると、長時間待たされたうえで Stop 前の turn の続きが返ってくることがある。

ユーザーは Stop 直後ではなく、送信ボタンへ戻った後に新しいメッセージを送っている。にもかかわらず、内部的には Stop 済みの Claude SDK session を成功終了扱いで resume して次 turn を投げるため、中断済み turn の状態が transcript / SDK session に残っている場合、新しい入力ではなく Stop 前 turn の続きを返してしまう。

本変更の目的は、Stop を「単なる provider abort」ではなく「turn のインタラプト境界」として扱い、Stop 後の次送信が必ずユーザーの新しい入力への応答として開始されるようにすることである。

## スコープ

- Stop された turn を、通常完了（正常終了）turn と区別して扱う。
- Stop 後の次送信が、停止済み turn の続きを再開せず、新しいユーザー入力への応答として開始されるようにする。
- Stop 以前に発生した遅延イベント（late stream / late result / late turn_complete）を、次 turn の agent message に混入させない。
- Stop 後の状態が UI / SessionStore / AgentStatusCenter で `completed` と誤認されないようにする。
- 上記を Claude backend（claude-sdk-bridge）の経路で成立させる。

## 非スコープ

- Claude 以外の agent backend の Stop / interrupt 挙動の変更（本 Issue は Claude backend を対象とする。下記「仮定」参照）。
- 通常 turn 完了、stale timeout、workflow step 実行といった既存挙動の仕様変更（これらは壊さないことが条件であり、変更対象ではない）。
- Stop 操作自体の UI（ボタン位置・見た目・操作フロー）の変更。
- 関連 Issue（#1178 / #731 / #1192 / #1246）が指す個別問題そのものの修正（本変更で副次的に改善する可能性はあるが、修正完了の責務には含めない）。

## 要求事項

### Stop 後の次送信

- Stop ボタンが送信ボタンに戻った後に新しいメッセージを送った場合、Stop 前 turn の続きを返さないこと。
- Stop 後の次送信は、ユーザーが送った新しい入力への応答として開始されること。
- Stop 後の次送信で、停止済み turn を成功完了として resume したことに起因する長時間待機が発生しないこと。

### Interrupted turn の区別

- Stop された turn を、通常完了 turn と区別できる「interrupted（中断）」境界として扱うこと。
- Stop された turn を、正常完了（成功終了）turn として扱わないこと。
- pending queue / workflow 通知 / status 遷移において、interrupted turn を通常の completed turn と区別すること。

### Late event の遮断

- Stop 以前に発生した late stream / late result / late turn_complete が、次 turn の agent message に混入しないこと。
- Stop 後に到着した遅延 turn_complete が、新しい turn を完了扱いにしないこと。
- Stop 後の次送信が、旧 turn の pending / stream に混入しないこと。

### 状態表示の整合

- Stop 後の状態が UI で `completed` と誤認されないこと。
- Stop 後の状態が SessionStore で `completed` と誤認されないこと。
- Stop 後の状態が AgentStatusCenter で `completed` と誤認されないこと。

### 既存挙動の維持

- 既存の通常 turn 完了挙動を壊さないこと。
- 既存の stale timeout 挙動を壊さないこと。
- 既存の workflow step 実行挙動を壊さないこと。

## 受け入れ基準の概要

- Stop ボタンが送信ボタンに戻った後に新しいメッセージを送っても、Stop 前 turn の続きを返さない。
- Stop 後の次送信が新しいユーザー入力への応答として開始される。
- Stop 以前の late event が次 turn の agent message に混入しない。
- Stop 後の状態が UI / SessionStore / AgentStatusCenter で `completed` と誤認されない。
- 既存の通常 turn 完了、stale timeout、workflow step 実行を壊さない。
- 単体テストまたは統合テストで以下を確認する。
  - Stop 後の late `turn_complete` が新 turn を完了扱いにしないこと。
  - Stop 後の次送信が旧 turn の pending / stream に混入しないこと。
  - interrupted turn 後の Claude bridge / session 再利用ポリシーが期待どおりであること。

## 仮定

- 本変更の対象 backend は Claude backend（`claude-sdk-bridge`）に限定する。Issue の症状・調査メモ・関連箇所がすべて Claude 経路に閉じているため。他 backend への展開が必要な場合は別 Issue で扱う。
- 「Stop 後の次送信が Stop 前 turn の続きを返さない」ことの外形的な判定は、新しいユーザー入力の内容に対応した応答が返ること、および Stop 前 turn の未完了出力が継続されないことで確認できるものとする。
- interrupted turn の境界・late event の fencing・session 再利用ポリシーの具体的な実現方式（turn generation / interrupt seq の導入有無や、停止済み `currentSessionId` の resume 方針など）は本要求では確定させず、`design.md` で決定する。

## Open Questions

なし。
