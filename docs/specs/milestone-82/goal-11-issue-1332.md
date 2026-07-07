# Goal: #1332 CLI/API command boundary 新語彙化

まず `docs/specs/milestone-82/goal-common.md` を読み、そこに書かれた必読ドキュメント・設計判断・横断ルール・品質ゲートに従うこと。`gh issue view 1332 --repo siro33950/releash` で issue 本文を読むこと。

最小 local API（D1）を新設して CLI をそれ経由に移行し、外部 command boundary を新語彙で統一する。

## 実装内容

1. **最小 local API（D1）**: Tauri アプリ内に localhost バインドの HTTP サーバを新設する（axum 等を dependency 追加可）。design.md §11 の endpoint 表が正。
   - 127.0.0.1 の ephemeral port で listen し、起動時に data dir へ discovery ファイル（`local-api.json` { port, token, pid }、権限 0600）を書く。全リクエストに bearer token 検証。
   - endpoint は typed command / query に 1:1 で対応させ、Tauri command と同じ usecase を呼ぶ controller（adaptor/controller）として実装する。business logic を API 層に持たない。
   - 対象: start（created_from の自己申告 field 含む）/ executions 一覧 / status / logs / approve / abort / output submit・validate・get。approve / output submit は同名 NodeExecution 並走時に `node_execution_id` でアドレスする（design.md §5/§11。session への env `RELEASH_NODE_EXECUTION_ID` 注入が既定値）。
2. **CLI の API 経由化**:
   - `releash workflow start <workflow-name> "<request>"` を新設。workflow-name は WorkflowDefinition.name で解決（P12。名前重複は Diagnostic）。自由文字列は request Artifact になる（#1326 の経路）。
   - `releash workflow executions` / `status <execution-id>` / `logs <execution-id>` / `approve <execution-id> --node <node-name> [--node-execution <id>] [--comment]` / `abort <execution-id>` / `output submit|validate|get <execution-id> --node <node-name> [--node-execution <id>] --type <contract> ...` を正とする（design.md §12。`--node-execution` は同名並走時のみ必須、session 内では env から自動解決）。
   - mutation（start / approve / abort / output submit）は API 必須とし、**pending file 経路（workflow_pending/、pending_command.rs、CliMutationRequested / CliMutationRejected event、pickup 機構）を撤去する**。app 未起動時の mutation は「アプリ起動が必要」エラーにする。
   - read-only（executions / status / logs / output get・validate）は API 経由を正とし、app 未起動時のみ file-direct 読みを最小 fallback として残す。
   - `runs` / `run_id` / `--step` / `Reject` の CLI 語彙を全廃する（#1324/#1331 の残存確認含む）。
3. **agent 向け文面の更新**: CLI help の system prompt 注入（cli/mod.rs render_long_help）、contract.rs 系 repair prompt、prompt_rendering.rs、instructions facet 本文が指示する CLI 語彙を新 command 形（executions / execution-id / --node / --type）に更新する。
4. **同一 boundary の検証**: UI（Tauri command）/ CLI（local API）/ Agent action が同じ typed command（usecase 層）に落ち、別々の状態遷移ロジックを持たないことをテストで固定する。`releash task ...` は追加しない（P6）。

## 削除対象

- pending file 機構一式（workflow_pending ディレクトリ、pending_command.rs、workflow_io.rs の enqueue、CliMutationRequested / CliMutationRejected event、dispatch/pickup 経路）
- CLI の `runs` / `run_id` 引数 / `--step` / 旧 subcommand 形
- 旧 CLI 語彙を含む repair prompt / help / facet 文面

## テスト

- workflow start（name 解決 / request Artifact 化 / 重複名 Diagnostic）、executions / status / logs、approve / abort、output submit・validate・get（--node / --type）の end-to-end（API 経由）。
- token 不一致 / discovery ファイル不在（app 未起動）の挙動（mutation はエラー、read は file-direct fallback）。
- UI / CLI / Agent action が同じ typed command boundary を通る境界テスト。
- 旧 `runs` / `run_id` / `--step` / pending file 経路が存在しないこと（regression / grep 検査）。
