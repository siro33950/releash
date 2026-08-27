## B-001: 単独 Session が応答終了で介入待ちになる

GIVEN Workspace で開始した単独 Session の実行木が実行中で、Workspace ツリーの当該行が青（Active）である
WHEN agent が応答を終えて入力待ちになり、provider が completion signal（Stop）を通知する
THEN その Session node の completion signal 状態が `StopReceived` になる
AND Workspace ツリーの当該行が黄（Attention）になる

## B-002: completion signal の記録が木の形と起こされ方に依存しない

GIVEN 実行中の単独 Session の root Session node と、実行中の workflow 実行木の child Session node がそれぞれ存在する
WHEN それぞれの provider が同じ completion signal（Stop）を通知する
THEN どちらの Session node も completion signal 状態が `StopReceived` になる
AND どちらの行も黄（Attention）になる

## B-003: 単独 Session の completion signal が再起動をまたいで保たれる

GIVEN 単独 Session が Stop を受けて当該行が黄（Attention）になっている
WHEN アプリケーションを再起動して同じ Workspace を開く
THEN その Session node の completion signal 状態は `StopReceived` のままである
AND 当該行は黄（Attention）のままである

## B-004: 単独 Session の node execution と AgentSession の対応

GIVEN 単独 Session が開始されている
WHEN Workspace ツリーの当該行を参照する
THEN その行は Session node の node execution と、対応する AgentSession を示す
AND アプリケーション再起動後も同じ対応が示される

## B-005: 単独 Session の詳細から実行木と node execution を取得できる

GIVEN 単独 Session が開始されている
WHEN その AgentSession の詳細を外部インターフェースから取得する
THEN 応答には、その Session が属する実行木の識別子と node execution の識別子が含まれる

## B-006: 単独 Session の archive / restore / delete が受理される

GIVEN archived でない単独 Session が存在する
WHEN 利用者が archive を要求する
THEN 要求は受理され、その Session は archived になる
AND archived になった Session に対する restore と delete の要求も受理される

## B-007: 単独 Session が GC の対象になる

GIVEN GC の条件を満たす単独 Session が存在する
WHEN GC が実行される
THEN その Session は GC の対象として扱われ、削除される

## B-008: workflow が起こした実行木の Session の archive / restore / delete / GC が拒否される

GIVEN workflow の実行として起こされた実行木の Session が存在する（その Session node が木の root であるか child であるかを問わない）
WHEN archive、restore、delete、GC のいずれかを要求する
THEN 要求は拒否される
AND その Session の状態は変わらない

## B-009: workflow 内 Session が Stop だけを受けた状態

GIVEN workflow 実行木の Session node が実行中で、Submit を受けていない
WHEN provider が completion signal（Stop）を通知する
THEN その node の completion signal 状態は `StopReceived` になり、当該行は黄（Attention）になる
AND その node の completion 条件は満たされず、node は完了しない

## B-010: Submit と Stop が揃ったとき completion 条件が満たされる

GIVEN workflow 実行木の Session node が Stop を受けて `StopReceived` である
WHEN その node が Submit を受ける
THEN その node の completion 条件が満たされる
AND `completion: Approval` を指定していない node は完了する

## B-011: completion に Approval を指定した node は承認待ちになる

GIVEN `completion: Approval` を指定した workflow 実行木の Session node が Stop を受けて `StopReceived` である
WHEN その node が Submit を受ける
THEN その node は承認待ちになる

## B-012: 実行木の一覧が起こされ方で分かれる

GIVEN 同じ worktree に、workflow の実行として起こされた実行木と、Session の起動として起こされた実行木がそれぞれ存在する
WHEN workflow 実行の一覧と workspace の session 一覧をそれぞれ取得する
THEN workflow 実行の一覧には workflow の実行として起こされた実行木だけが現れる
AND workspace の session 一覧には Session の起動として起こされた実行木だけが現れる

## B-013: Session の起動として起こされた実行木は explicit retry を受理しない

GIVEN Session の起動として起こされた実行木の Session node が片側の completion signal を受けている
WHEN Workspace ツリー、local API、または Tauri command からその node execution の Retry を要求する
THEN Workspace ツリーに Retry 操作は表示されず、直接の要求も拒否される
AND 新しい attempt は作られず、実行木と AgentSession の状態は変わらない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006, B-007, B-008 |
| R-007 | B-009, B-010, B-011 |
| R-008 | B-012 |
| R-009 | B-013 |
