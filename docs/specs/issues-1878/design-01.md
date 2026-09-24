# Design 01

## 開始状態

初回。既存の `design-NN.md` は無い。開始状態の挙動は `docs/specs/issues-1878/requirements.md` の Current Behavior を参照する。

- 差分の基準は base ブランチ `main` からの派生点 `1ad87359`（branch `feat/issues/1878`）。
- 未コミットの変更は未追跡の `docs/specs/issues-1878/`（`requirements.md`、`behavior.md`）だけであり、コードは `1ad87359` のままである。
- この周までに解消・見送りとなった Thread は無い（open Thread なし）。

## 変える部分

- 購読の stream の新設: client ごとに 1 本の stream を作り、その client の全ての購読対象をそこへ載せる。根拠: R-006「一つの client の全ての購読対象は、その client の 1 本の stream で届く」、B-006。ルート: xDS の ADS に合わせて 1 本の stream に全ての対象を載せる。server streaming で作る。message の形は委任。
- 購読の開始と停止の手段の新設: 購読の開始と停止を単発の呼び出しで行えるようにする。根拠: R-007「購読の開始と停止は、単発の呼び出しで行える」、B-007。ルート: 開始と停止は単発の呼び出しで送る。双方向 stream にはしない。
- 購読開始時の配信順序の実装: 購読を開始すると現在の状態を送り、続いて区切りの印を送り、その後は変更だけを送る。根拠: R-004「購読を開始すると、まず現在の状態が届き、続いて区切りの印が届く。区切りの印より後は、変更だけが届く」、B-004。ルート: Kubernetes API の list + watch に合わせる。
- 変更の送り方の選択: 対象ごとに丸ごと送るか差分を送るかを選べる形にし、本 ISSUE で購読へ移す対象は丸ごと送る。根拠: R-005「変更の届け方は、対象ごとに対象を丸ごと送るか差分を送るかを選べる。本 ISSUE で購読へ移す対象は丸ごと送る」、B-005。ルート: 委任。
- 購読の単位の実装: 購読の単位を画面が使う読み取り結果 1 つとし、複数の状態の組み合わせを daemon 側で行う。根拠: R-003「購読の単位は、画面が使う読み取り結果 1 つである。複数の状態の組み合わせは daemon が行い、client は受け取った結果をそのまま表示できる」、B-003。ルート: 委任。
- 対象ごとの版番号: daemon が対象ごとに単調に増える版番号を振り、購読で届く状態・変更・印に載せる。単調性は daemon の一回の起動の中で保つ。根拠: R-008「対象ごとに、daemon が単調に増える版番号を付ける。購読で届く状態、変更、印には版番号が付く。単調性は daemon の一回の起動の中で保たれる」、B-008。ルート: Kubernetes API の `resourceVersion` に合わせる。版番号をどこから作るかは委任。
- 版からの再開: 最後に受け取った版を指定した続きからの再開と、指定した版から再開できない場合および daemon の別の起動で振られた版を指定した場合の最初からのやり直しを実装する。根拠: R-009「つなぎ直すときは、最後に受け取った版を指定して続きから再開できる。指定した版から再開できない場合、および daemon の別の起動で振られた版を指定した場合は、現在の状態が最初から届く」、B-009、B-010。ルート: Kubernetes API の `410 Gone` による再取得に合わせる。
- 購読ごとの送り待ち: 送り待ちを購読ごとに持ち、溢れた購読だけを版からの再開に落とす。同じ stream の他の購読と stream そのものは止めない。根拠: R-010「送り待ちは購読ごとに持つ。ある購読の送り待ちが溢れた場合、その購読だけが版からの再開になる。同じ stream の他の購読と、stream そのものは止まらない」、B-011。ルート: 委任。
- 印（bookmark）の定期配信と、UI 側の無通信検出によるつなぎ直し: 変更が無い間も版番号を載せた印を定期的に流し、UI は一定時間何も届かなければつなぎ直す。根拠: R-011「変更が無い間も、版番号を載せた印が定期的に届く」、B-012。ルート: Kubernetes API の watch bookmark に合わせる。印の間隔と、UI 側で無通信を検出してつなぎ直す判定の実装方法は委任。
- 購読数の上限の不在: 一つの client が持てる購読の数に上限を設けない。開始状態にある変更通知の購読 16 件・監視 64 件の上限（`src-tauri/src/domain/repository/watch_subscriptions.rs:42,59-66`）に相当するものを、新しい仕組みへ持ち込まない。根拠: R-012「一つの client が持てる購読の数に上限を設けない」、B-013。ルート: 委任。
- 重複購読のまとめ: 同じ stream の中で同じ対象を重ねて購読しても、その対象の購読を 1 件にまとめる。根拠: R-013「同じ stream の中で同じ対象を重ねて購読しても、その対象の購読は 1 件にまとまる」、B-014。ルート: 委任。
- 存在しない対象の購読の拒否: 存在しない対象の購読を拒否する。根拠: R-014「存在しない対象の購読は拒否される」、B-015。ルート: 委任。購読対象の識別方法は委任。
- stream 終了時の後始末: stream が終わったら、その client の購読を全て終わらせる。根拠: R-015「stream が終わると、その client の購読は全て終わる」、B-016。ルート: 委任。
- Repository のパス一覧を購読の対象にする: Repository のパス一覧を購読で届ける最初の対象とし、購読開始時に現在の一覧を、以後は変わるたびに変わった後の一覧を送る。根拠: R-016「Repository のパス一覧は購読で届く」、R-001「daemon が持つ状態は購読で届く」、B-017。ルート: 委任。
- Settings の Repositories が一覧を得る経路の切り替え: Repository のパス一覧を表示する画面が、購読で届いた一覧を使うようにする。開始状態では `src/App.tsx:115-116` が `useWorkspaceList` の snapshot（`workspaceList.snapshot.repositories`）から導出して `SettingsModal` の `repoPaths` へ渡している。根拠: R-016、B-017、requirements.md の Scope「Repository のパス一覧を表示する画面（Settings の Repositories）が一覧を得る経路」。ルート: 委任。
- `GetRepoPaths` の削除: Repository のパス一覧を返す単発の呼び出しを削除する。根拠: R-017「Repository のパス一覧を返す単発の呼び出し（`GetRepoPaths`）は無い」、R-001「購読へ移した状態を返す単発の呼び出しは無い」、B-018。ルート: 委任。
- `repo-paths-changed` の削除: Repository のパス一覧が変わったことを知らせる変更通知を削除する。根拠: R-018「`repo-paths-changed` の変更通知は無い」、B-019。ルート: 委任。
- Repository の追加・削除を Workspaces の表示へ反映する経路: `repo-paths-changed` の削除後も、Repository の追加・削除の結果が Workspaces の表示する Repository 一覧へ反映されるようにする。開始状態では `src/hooks/useWorkspaceList.ts:136` が `repo-paths-changed` を受けて `refresh_workspaces` を起こしている。根拠: R-019「Repository の追加・削除の後も、Workspaces は追加・削除の結果を反映した Repository 一覧を表示する」、B-020。ルート: 人間が固定。UI 側で起こす。範囲は `src/App.tsx` と `src/hooks/useWorkspaceList.ts`。粒度は「購読で届いた Repository のパス一覧が変わったら Workspaces 一覧の取り直しを起こす」まで。
- 使われていない読み取りの呼び出し 26 件の削除: `GetCrashReportingEnabled`、`GetFileAtRef`、`GetStagedContent`、`GetBinaryStagedContent`、`GetFileAtBranchBase`、`GetBinaryFileAtBranchBase`、`GetBinaryFileAtRef`、`GetBranchDiffSummary`、`GetHeadDiffFileTreeSnapshot`、`GetRelativePath`、`GetReviewThread`、`GetReviewThreadHistory`、`GetDefaultBranch`、`GetGitStatus`、`GetGitStatusSnapshot`、`GetStatusDiffStats`、`GetStatusDiffStatsSnapshot`、`GetGitLog`、`GetWorktreeDirtyCount`、`GetRepoGitDir`、`ListWorkflowExecutions`、`GetWorkflowExecution`、`GetWorkflowExecutionLog`、`GetWorkflowNodeDetail`、`ResolveWorktreeByExecution`、`ListFacets` を削除する。根拠: R-020「`src` から使われていない読み取りの呼び出しは無い」、B-021。ルート: 委任。26 件の削除と、それに伴って使われなくなるコードの特定・削除の手順は委任。
- 使われなくなるコードの削除: この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードを削除する。対象は Rust、TypeScript、proto、およびそれらだけを対象とするテストである。根拠: R-021「この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。対象は Rust、TypeScript、proto、およびそれらだけを対象とするテストである」、B-022。ルート: 委任。今回変更したファイルの中の未使用コードの特定も委任。

R-002「状態を変える操作と、client の入力に対する計算は単発の呼び出しで行える」は、開始状態の単発の呼び出しで既に満たされており、変更しない。

## 固定するルート

- 購読の仕組みは、一般的な作り（Kubernetes API の list + watch、xDS の ADS）に合わせる。範囲は購読の仕組み全体。粒度は下敷きにする設計の指定であり、型やモジュール配置までは指定しない。理由は正本 Issue が方針として挙げているため。関係する要求は R-004（最初の状態と区切りの印）、R-008・R-009（版番号と再開）、R-011（bookmark）、R-006（1 本の stream に全ての購読対象を載せる = ADS）。
- 今は server streaming で作り、購読の開始と停止は単発の呼び出しで送る。範囲は購読の transport の形。粒度は stream の向きと開始・停止の送り方の指定であり、message の形までは指定しない。理由は、双方向 stream への載せ替えをネイティブ UI（#78）の B13 以降に行うと決めているため。関係する要求は R-006、R-007。
- R-019（Repository の追加・削除が Workspaces の表示へ反映される）は UI 側で起こす。範囲は `src/App.tsx` と `src/hooks/useWorkspaceList.ts`。粒度は「購読で届いた Repository のパス一覧が変わったら Workspaces 一覧の取り直しを起こす」まで。理由は、R-019 が #1885 で Workspaces を購読へ移すまでの一時的な要求であり、そこでそのまま外れる場所に置くため。また、R-018 で消す通知経路を daemon 側に作り直さないため。関係する要求は R-018、R-019、受入条件は B-020。

上記以外の実装上のルートは指定されていない。購読の仕組みの内部設計（型、モジュール配置、proto の形、購読対象の識別方法）、版番号の生成方法、購読ごとの送り待ちと溢れたときの再開の実装方法、印の間隔と UI 側の無通信検出の判定、削除の手順と未使用コードの特定は、いずれも委任である。

## 変えないもの

- `ListBranchesWithStatusSnapshot` は削除しない。理由は、`src/components/workspace/CreateWorktreeModal.tsx:125` から使われており、正本 Issue が削除対象を選んだ条件（`src` から呼ばれていない）に合わないため。購読への移設は #1885 が扱う。
- デッドコードの削除について `behavior.md` に受入条件を追加しない。既存の B-022 の範囲を広げるに留める。理由は、人間が「振る舞いには書かなくていい」と決めたため。

## 未確定・リスク

なし。この周で自動判断した箇所、未決のまま残した要求、`[DEFERRED]` で人間へ渡した件は、いずれも無い。
