# Goal: #1328 bash を command node に移行

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1328 --repo siro33950/releash` で issue 本文を読むこと。

`command: "<shell>"` kind block を実行可能にする。旧 `type: bash` は #1322 で schema からは消えているため、本 goal は command 実行系の実装が中心。

## 実装内容

1. **command 実行 infra**: `infrastructure/`（または既存 process 実行基盤）に、shell command を `/bin/sh -c` で worktree を cwd として実行し、exit_code / stdout / stderr / duration(ms) を取得する実装を追加する。出力サイズは既存の output 制限機構（output_limit.rs）に準じて制限する。timeout / retry / 並列度はスコープ外（構文予約もしない）。
2. **標準結果 Artifact（D3）**: command NodeExecution は常に予約 field `ok` / `exit_code` / `stdout` / `stderr` / `duration` を持つ Artifact を産出する。`ok = (exit_code == 0) && (artifact 未指定 || Contract validation 成功)`（P11）。
3. **artifact: 指定時**: stdout を JSON parse → #1325 の Contract 検証 → 成功時は Contract field を予約 field と単一名前空間に合成して Artifact 保存。予約名を宣言する Contract は load 時 Diagnostic（D3）。parse / validation 失敗時は ok=false の標準結果のみで完了し、rules（P11 の catch-all）で fix node へ遷移できる。
4. **runtime 統合**: engine の kind 分岐に command 実行を実装（現行 bash は runtime 未実装。transition.rs で Bash が UnexpectedNodeType 扱いになっている分岐を置換）。標準結果は **ArtifactProduced event（contract は artifact: 無しなら null、value = 予約 field ∪ Contract fields）** として記録し、NodeCompleted は result_summary / token_usage に留める（design.md §9 の最終形）。projection / UI / CLI で読めるようにする。
5. **cancellation**: command は process group を作って spawn し、abort / アプリ終了時に kill する（design.md §8.1 手順 5。既存 child process の staged shutdown 機構を再利用）。kill された NodeExecution は完了させず ExecutionAborted / ExecutionInterrupted に帰着させる。
6. **routing**: command result（ok）と Artifact field の両方で `when` / `switch` 分岐できること（#1327 の rules に接続。**`ok` は artifact 有無を問わず常に routing 可能な Boolean**。design.md §4 / §6 R4・R6）。
7. **built-in / example 整合**: built-in に bash 由来 node があれば command 記法へ移行。full-pipeline.yml の `run_tests` / `judge` / `list_threads` 相当の command node パターン（標準結果 routing / stdout-JSON Artifact / inputs 参照）を fixture で再現して検証する。

## 削除対象

- `type: bash` の残骸（validation の MissingCommand / EmptyCommand 等は新 command kind の検査として再配置）
- runtime の UnexpectedNodeType(Bash) 分岐

## テスト

- `ok` / `exit_code` / `stdout` / `stderr` / `duration` の標準結果（成功 / 非 0 exit / stdout 巨大時の制限）。
- exit code routing（`when { on: ok }`）。
- stdout-JSON の Contract 検証成功 → Contract field で routing、検証失敗 → ok=false で fix node へ route。
- 予約 field 名を宣言する Contract の Diagnostic。
- 旧 `type: bash` が受理されないこと（#1322 の regression test の維持確認）。
