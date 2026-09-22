# Design 01

## 開始状態

差分の基準は base branch `main` の `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)`、作業 branch は `feat/issues/1826`。未コミットの変更は `docs/specs/issues-1826/` の `requirements.md` と `behavior.md` だけで、コードは未変更である。先行条件の #1839 はこの基準 commit へ取り込み済みである。

`docs/specs/issues-1826/` に既存の `design-NN.md` は無く、初回の周である。実装の状態は `requirements.md` の Current Behavior を参照する。この周までに解消・見送りとなった Thread は無い。

## 変える部分

- 実行木 Archive 操作の統一: Workflow として起動した実行木と単独 Session として起動した実行木を、execution tree を対象とする同じ Archive 操作へ通す。単独 Session の provider session ID の有無による Archive 不可・Delete 確認への分岐は Archive 経路から外し、画面、Connect、GC、worktree 削除から同じ backend 操作を利用する。根拠: R-001、R-006、B-001、B-009。ルート: 固定（「固定するルート」1、4）
- Archive と Abort の順序: 実行木の状態にかかわらず Archive を受理し、終了していない場合は既存の Abort が成功した後にだけ Archive の事実を記録する。Abort が完了しなければ Archive 済みにせず、終了済みの場合は終了状態を変えない。根拠: R-002、B-002〜B-004。ルート: 固定（「固定するルート」2）
- Archive / Restore の正本の統一: Archive 状態、Archive 時刻、理由、および Restore を root の事実ログから導出し、Workspace tree と履歴も同じ事実を読むようにする。`workflow_execution_archives.json` の repository と同ファイルへの通常の読み書きを廃止する。根拠: R-003、R-011、B-005、B-016。ルート: 固定（「固定するルート」3、7）
- 既存 Archive 記録の移行: `workflow_execution_archives.json` の既存記録を、Archive 状態、時刻、理由を保った事実ログへ移す。移行対象が終了していない場合は Abort 成功後にだけ Archive を記録し、移行後は旧ファイルを Archive の正本として読まない。根拠: R-004、B-006。ルート: 固定（「固定するルート」3）
- 画面の Archive 提示と確認: Archive されていない実行木には状態にかかわらず Archive を提示し、終了していない実行木だけ、Archive により Abort されることを画面で確認する。確認を取り消した場合は何も実行せず、終了済みの場合は確認なしで共通 Archive 操作を呼ぶ。バックエンドには確認状態や確認手順を追加しない。根拠: R-005、R-006、B-007〜B-009。ルート: 固定（「固定するルート」4）
- GC による消失 worktree の Archive: 既存 GC が取得する Git worktree 一覧と同じ経路で、一覧から消えた worktree に属する実行木を理由 `worktree_removed` で共通 Archive 操作へ渡す。一覧にある worktree は対象外とし、フォルダの有無は参照せず、対象リポジトリを読めない場合はそのリポジトリの実行木を Archive しない。根拠: R-007、R-008、B-010〜B-012。ルート: 固定（「固定するルート」5）
- Releash 内の worktree 削除前の Archive: worktree に属する実行木を理由 `worktree_removed` で共通 Archive 操作により片付け、終了していない実行木の Command プロセス停止を含む Archive 完了後に Git worktree とフォルダの削除へ進む。根拠: R-009、B-013。ルート: 固定（「固定するルート」6）
- Restore と単独 Session の再開の分離: Archive 済みの実行木の Restore は Archive を解除する事実だけを記録し、終了状態を変えずプロセスを起動しない。現在の単独 Session Restore が同時に行う provider process の起動を外し、Restore 後の再開は既存の手動 Resume 操作だけが行う。根拠: R-011、B-016、B-017。ルート: 固定（「固定するルート」7）

## 固定するルート

1. Workflow として起動した実行木と単独 Session として起動した実行木の Archive は、両者を実行木対象の同じ Archive 操作へ通す。単独 Session も Session Node 一つを root とする実行木であるため。関係: R-001、B-001
2. 終了していない実行木の Archive は、Archive 操作の内側で既存の Abort を実行し、Abort 成功後にだけ Archive する。Archive 済みかつ実行中の状態を不可能にするため。関係: R-002、B-002、B-004
3. Archive および Restore の永続記録は事実ログへ統一し、`workflow_execution_archives.json` の既存記録を移行して同ファイルへの読み書きを廃止する。workflow state の正本を事実ログへ揃えるため。関係: R-003、R-004、B-005、B-006
4. 実行中の実行木を画面から Archive する際の確認は画面だけに置き、バックエンドの Archive 操作には確認機構を持たせない。画面以外の入口でも追加入力なしに同じ操作を実行するため。関係: R-005、R-006、B-008、B-009
5. GC の worktree 消失判定は、既存 GC が付属データの削除判定に使う Git worktree 一覧と同じ経路を使い、フォルダの有無を参照しない。リポジトリを読めない場合は何もしない。Git の状態を正とするため。関係: R-007、R-008、B-010〜B-012
6. Releash 内の worktree 削除では、理由 `worktree_removed` の Archive を Git worktree およびフォルダ削除より先に完了する。フォルダが消える前に実行中 Command プロセスを停止するため。関係: R-009、B-013
7. Archive 済み実行木の Restore は Resume を呼ばず、単独 Session のプロセス再開は既存の手動 Resume 操作へ委ねる。Restore 時の自動再開を行わないため。関係: R-011、B-016、B-017

固定したルート以外の型・関数・モジュール配置、処理分割、テスト設計は委任する。

## 変えないもの

- Abort が worktree フォルダの存在を要求せず、Command の Retry が実行先フォルダの存在を要求する #1839 後の条件。今回の Archive は既存の Abort を利用し、Retry の条件は変更しない（R-010、B-014、B-015）
- Node の状態、Resume / Retry、起動時の自動再試行など #1839 で確定・実装済みのライフサイクル。理由は Archive と Restore に必要な変更以外を #1839 の範囲へ戻さないため
- 起動時の Workflow 状態の常駐・復元方法と worktree 単位の起動排他（#1840）、実行木の自然完了の事実化と保存済み定義を読めない実行の扱い（#1836）。理由は各 Issue が所有するため
- worktree フォルダの削除方法と、削除を画面の待機対象から外す変更（#1845）。今回変更するのは削除前に Archive を完了する順序までである

## 未確定・リスク

なし。
