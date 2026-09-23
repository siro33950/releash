# Design 01

## 開始状態

- 差分の基準は branch `feat/issues/1864` の `f3e55b13`（`origin/main` と同一）。コードの未コミット変更はない
- 既存の Design はないため初回とし、開始状態の挙動は `requirements.md` の Current Behavior を参照する
- この周までに解消・見送りとなった Thread はない

## 変える部分

- 読み側の祖先受理条件: 読み側 `execution_worktree_path`（`src-tauri/src/adaptor/gateway/workflow/worktree_context.rs`）が、祖先の Node が Sequence / Fanout でないことを理由に `Corrupt("worktree ancestor is not a composite")` を返す判定をやめ、delegate 親 Session を含む祖先経路でも作業場所の継承を続けられるようにする。根拠: R-001「delegate 親子関係を含む祖先経路でも、Session の作業場所を読み側で解決できる」、B-001「祖先が合成子でないことを理由とする読み取りエラーにならない」。この判定が Session 通知の受理失敗（R-003 / B-003）と Fanout の完了待ち（R-004 / B-004）の起点であり、読み側と実行側の一致（R-002 / B-002）もこの変更で満たす。ルート: 委任
- Session 祖先を破損として拒否する既存テストの期待: `src-tauri/src/adaptor/gateway/workflow/worktree_context_test.rs` の `test_実効cwd_祖先の欠落と循環と別木と不正定義はcorruptになる` にある `leaf-parent`（祖先 Node の kind を Session に差し替えて `Corrupt` を期待する）ケースを、要求に合わせて改める。根拠: R-006 の拒否対象が循環した祖先関係と別の実行木の Node を含む祖先関係だけになり、B-006 に Session 祖先が含まれないため。ルート: 委任
- delegate 構造の回帰テスト追加: delegate 親 Session を祖先に持つ Session について、作業場所の解決と Session 開始・終了通知の受理を確認するテストを追加する。根拠: R-001 / R-002 / R-003（B-001 / B-002 / B-003）。開始状態のテストは Sequence または Fanout の直下に Session を置く fixture だけで、この経路を通らない。ルート: 下記「固定するルート」

## 固定するルート

- 回帰テストの対象構造を固定する。範囲: `impl`（Sequence）→ `implement`（Session）→ `completion.delegate` → `inner_check`（Sequence）→ `inner_review`（Fanout）→ `reviewer`（Session）という delegate 構造で、作業場所の解決と Session 開始・終了通知の受理を確認する。粒度: 検証対象の構造と確認項目まで（配置先・命名・組み立て方は指定なし）。理由: 「Command のみの検証ではこの不具合を検出できない」ため
- 上記以外に固定する実装上の指定はない。読み側 `execution_worktree_path` の受理条件をどう書き換えるか、判定をどの関数・どの層に置くかは委任。回帰テストの配置先、テスト名、fixture の組み立て方、モック方針も委任

## 変えないもの

- 読み側が読み取りエラーとして拒否し続ける対象は、循環した祖先関係と別の実行木の Node を含む祖先関係だけにする。祖先の Node 種別や親子関係の種別による拒否を読み側に持たせない。理由: 実行側 `ExecutionTree::execution_worktree_path`（`src-tauri/src/domain/workflow/entities/workflow_execution/worktree.rs:4-25`）は祖先の Node 種別を判定しておらず、読み側だけ厳しくすると同じ事実に対して起動は成功・読み取りは失敗という食い違いが別条件で残り、R-002 と齟齬が出るため
- 実行側（domain、`workflow_host`、`control_plane`）の作業場所の導出は変更しない。理由: 今回の不具合が読み側の受理条件に限られ、実行側は同じ構造で正しく解決しているため
- 既に終了が記録されず停止している実行木を先へ進めるための専用の復旧経路は作らない。理由: 未完了かつプロセス不在の Session Node に対する Resume が既存の一般操作として存在し（`src-tauri/src/usecase/workflow/control_plane.rs:446-512`、`docs/specs/issues-1839` R-003）、読み側の解決を直せば delegate 部分木の Session にも使えるため

## 未確定・リスク

- R-003 / R-004 を満たすことの確認が、読み側の解決経路の確認に留まる。Issue の調査時点で修正後の再実行は行われておらず、delegate 部分木の Session で Stop が記録されない要因が読み側の受理条件のほかに残っていないことは、実行を通しては未確認である。想定が外れた場合、読み側を直しても Fanout が完了待ちのまま残る
- 自動判断した箇所はない。`requirements.md` の Assumptions は「なし」であり、未決のまま残した要求もない
- `[DEFERRED]` で人間へ渡した件はない
