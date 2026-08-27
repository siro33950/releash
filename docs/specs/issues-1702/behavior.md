## B-001: 周回をまたぐ fanout 子 Node の識別子一意性（item が定義済みの fanout）

GIVEN item が定義済みの fanout を配下に持つ合成子が、後方辺で再入される workflow が実行されている
WHEN 同じ fanout 定義が 2 回以上の周回で実行され、各周回で fanout 子 NodeExecution が開始される
THEN その Worktree の Workspace tree に含まれる全 Node の識別子が互いに異なる
AND 異なる周回で開始された同じ fanout 子は、互いに異なる識別子を持つ

## B-002: 周回をまたぐ fanout 子 Node の識別子一意性（item が実行時に決まる fanout）

GIVEN item が実行時に決まる fanout を配下に持つ合成子が、後方辺で再入される workflow が実行されている
WHEN 同じ fanout 定義が 2 回以上の周回で実行され、各周回で fanout 子 NodeExecution が開始される
THEN その Worktree の Workspace tree に含まれる全 Node の識別子が互いに異なる
AND 異なる周回で開始された同じ fanout 子は、互いに異なる識別子を持つ

## B-003: 周回をまたぐ fanout を含む Worktree の Workspace tree 読み出し

GIVEN 後方辺で同じ fanout を 2 回以上通過した WorkflowExecution を含む Worktree がある
WHEN その Worktree の Workspace tree を読み出す
THEN 読み出しは成功し、Workspace node id の衝突を理由に `corrupt_stored_state` で拒否されない
AND UI は当該 Worktree の Node を「Node unavailable」として表示しない

## B-004: 周回をまたぐ fanout を含む Worktree の Node 詳細読み出し

GIVEN 後方辺で同じ fanout を 2 回以上通過した WorkflowExecution を含む Worktree がある
WHEN その Workspace tree に現れる Node の詳細を読み出す
THEN 読み出しは成功し、Workspace node id の衝突を理由に `corrupt_stored_state` で拒否されない

## B-005: 周回ごとの fanout インスタンス配下への fanout 子の出現

GIVEN 後方辺で同じ fanout を 2 回通過した WorkflowExecution がある
WHEN その Workspace tree を読み出す
THEN 1 周目に開始された fanout 子 NodeExecution は、1 周目の fanout インスタンスの配下に Node として現れる
AND 2 周目に開始された fanout 子 NodeExecution は、2 周目の fanout インスタンスの配下に Node として現れる
AND 各周回の fanout 子は、他の周回の fanout 子と同一の Node に統合されない

## B-006: 衝突を含む既存 fact log の読み出し

GIVEN 変更前に記録され、fanout 子 Node の識別子衝突を生じさせていた fact log（報告実例: execution `97c31282-c12a-4163-b6f8-6735b78c73cf`）が保存されている
WHEN データ移行を行わずに、その Worktree の Workspace tree を読み出す
THEN 読み出しは成功する
AND 読み出しによって fact log は書き換えられない

## B-007: 同一 fact log からの導出の再現性

GIVEN 追記が発生していない fact log を持つ Worktree がある
WHEN その Worktree の Workspace tree を複数回読み出す
THEN 各回で導出される Workspace tree が同一である
AND 各回で fanout 子 Node の識別子が同一である

## B-008: 承認待ち Node への応答

GIVEN 後方辺で fanout を 2 回以上通過した WorkflowExecution を含む Worktree に、承認待ちの Node がある
WHEN 利用者が UI からその Node を選択して承認応答を行う
THEN 承認応答は対象の NodeExecution に対して受理される
AND Workspace node id の衝突を理由に失敗しない

## B-009: 待機中の Session Node への回答

GIVEN 後方辺で fanout を 2 回以上通過した WorkflowExecution を含む Worktree に、待機中の Session Node がある
WHEN 利用者が UI からその Session Node へ回答を送る
THEN 回答は対象の Session へ届く
AND Workspace node id の衝突を理由に失敗しない

## B-010: Workspace node id の衝突以外を要因とする読み出し拒否の維持

GIVEN Workspace node id の衝突以外の要因で Workspace tree の不変条件を満たさない保存状態がある
WHEN その Worktree の Workspace tree を読み出す
THEN 読み出しは `corrupt_stored_state` で拒否される

## B-011: 同一 Worktree の複数 WorkflowExecution にまたがる fanout 子 Node の識別子一意性

GIVEN fanout を含む同じ workflow が、同じ Worktree で 2 回以上実行されている
WHEN その Worktree の Workspace tree を読み出す
THEN Workspace tree に含まれる全 Node の識別子が互いに異なる
AND 異なる WorkflowExecution で開始された同じ fanout 子は、互いに異なる識別子を持つ

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-011 |
| R-002 | B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006, B-007 |
| R-005 | B-008, B-009 |
| R-006 | B-010 |
