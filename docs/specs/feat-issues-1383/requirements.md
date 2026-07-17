# Requirements — F1: wire record/replay 回帰テスト基盤

対象 issue: [#1383](https://github.com/siro33950/releash/issues/1383)（milestone 84「Agentチャット安定化」／ Phase 0 ／依存なし）
解消する監査問題: **ST-7（前半）** — 再発検出基盤の欠落（wire fixture replay テストが無い）

## 背景と目的

### 背景

agent chat（Claude / Codex セッション）の wire メッセージを domain event / read model へ変換する経路には、現状 inline `json!` literal の単体テストしか存在しない。

- Claude: `convert_claude_message`（`infrastructure/agent_session/claude/convert.rs:71`）が `&Value` を受け取り `ClaudeConversion { events: Vec<AgentRuntimeEvent>, auto_responses: Vec<Value> }` を返す。テストは 18 件すべて inline `json!`。
- Codex: `convert_jsonrpc_message`（`infrastructure/agent_session/codex/convert.rs:34`）が `&Value` を受け取り `Vec<AgentRuntimeEvent>` を返す。テストは 22 件すべて inline `json!`。
- `AgentRuntimeEvent` は `domain/agent_session/gateway.rs:57` に定義。durable 化された `AgentSessionEvent` は `event_log/projector.rs` の `project()` により `SessionReadModel`（messages / status / workflow_turn_complete / tool_retries）へ射影される。

監査 ST-7 が指摘するとおり、CLI のバージョンアップや将来の変更で「無言破棄・扱いの差」が再発したとき、それを機械的に検出する手段が無い。監査で確定した多くの問題（CX-1 の wire 形式誤り、CX-4 のフィールド名不一致、CX-9 の dead code 化）は、実 wire ログを流すテストがあれば検出できたものである。

### 目的

実セッションの wire ログ（Claude の stream-json 行 / Codex の JSON-RPC 行）を fixture として `convert → AgentRuntimeEvent → 永続 event → projector → read model` に流し、出力を golden ファイルと比較する回帰テスト基盤を用意する。

これにより:

- **挙動等価の証明**: F4/F5（typed wire 置換）は「golden 不変」で変換挙動の等価を証明する。
- **意図した差分のレビュー**: F6 以降（語彙変更）は「golden 差分が設計どおりか」をレビュー可能にする。
- **milestone 84 全体の安全網**: 本 issue は Phase 0 の最初に実施し、以降の全 issue の回帰検出土台になる。

wire 層の規範（`agent-chat-ideal-vocabulary.md` §12 / V-P1）に従い、content-plane の変換先の無い既知/未知メッセージ件数と control-plane の未対応件数を、将来の parity テスト（ST-7 後半 = T1 #1416）が別々に検証できる観測点をこの基盤で用意する。

## スコープ

1. **fixture 置き場と実 wire ログの格納**
   - `src-tauri/src/infrastructure/agent_session/fixtures/{claude,codex}/` を新設し、実セッションから採取した wire ログ（Claude=stream-json 行、Codex=JSON-RPC 行）を JSONL として格納する。
   - 秘匿情報（絶対パス・トークン・認証情報・メッセージ本文など）はマスキングして格納する。
   - fixture は少なくとも受け入れ基準を満たす通常 turn を含む（後述）。

2. **record tap（開発時採取機構）**
   - Claude / Codex が共有する stdout line reader の生行分類境界に、環境変数ゲート `RELEASH_WIRE_RECORD=<dir>` の tap を追加し、各 backend process の生成時に対応する recorder を接続する。
   - 既定（環境変数未設定）では無効で、本番挙動・パフォーマンスに影響しない。
   - 採取した生行を `<dir>` 配下へ 1 行 1 メッセージの JSONL で書き出す。マスキング適用の有無は設計で定める（後述）。

3. **replay テスト（convert → AgentRuntimeEvent の golden 比較）**
   - fixture を 1 行ずつ `convert_claude_message` / `convert_jsonrpc_message` に流し、出力イベント列を `serde_json` 化して golden ファイルと比較するヘルパを実装する。
   - Claude 側は `AgentRuntimeEvent` 列に加え `auto_responses`（control-plane の自動応答）も golden に含める。
   - convert は stateful（`ClaudeConvertState` / `CodexConvertState`）であり、fixture を通す間 state を引き継ぐ。
   - 新規依存を増やさず自作比較で行い、`UPDATE_GOLDEN=1` で golden を更新できる。

4. **read model までの統合 golden**
   - 同じ fixture を `AgentRuntimeEvent → 永続 event → event_apply → projector（`project()`）` まで通し、`SessionReadModel` のスナップショットも golden 比較する cross-layer テストを crate 最外周の `test_support` に追加する。

5. **golden 更新手順のドキュメント**
   - fixture ディレクトリに README を置き、record tap による採取手順・マスキング方針・`UPDATE_GOLDEN=1` による golden 更新手順を記載する。

6. **CI 実行**
   - replay / 統合 golden テストが `cargo test`（CI）で実行される。

## 非スコープ

- **監査 ST-7 後半に属する他テスト**: E2E turn ライフサイクルテスト、cross-backend parity テストは本 issue の対象外（T1 #1416）。本基盤はそれらが再利用できる形にとどめる。
- **wire 変換の挙動変更・語彙変更**: convert / projector の出力を変える修正（CX-1 の wire 形式修正、CX-4 フィールド名、語彙拡張など）は F4/F5/F6 以降の各 issue の scope。本 issue は現状挙動を golden として固定するだけで、変換ロジックは変更しない。
- **`codex/permission.rs` の誤挙動固定テスト（CX-1）の是正**: 誤った現行仕様を固定しているテストの修正は CX-1（F6 系）で扱う。
- **frontend / playwright（`pnpm test:integration`）への追加**: agent chat の E2E は対象外。
- **fixture の網羅的拡充**: 監査付録 A のすべての wire type を網羅する fixture 収集は行わない。受け入れ基準に挙げた代表 turn を満たす範囲にとどめ、拡充は各 issue の必要に応じて追加する。

## 要求事項

1. **fixture 基盤**: `infrastructure/agent_session/fixtures/{claude,codex}/` に、実セッション由来の wire ログを JSONL で格納できる。秘匿情報はマスキングされている。

2. **record tap**: `RELEASH_WIRE_RECORD=<dir>` 設定時のみ、Claude / Codex の stdout 読み取り経路で受信した wire 行を `<dir>` に採取できる。未設定時は無効で本番挙動に影響しない。

3. **replay golden（convert 層）**: fixture を convert に通して得た `AgentRuntimeEvent` 列（Claude は `auto_responses` 含む）を golden と比較し、不一致で fail する。`UPDATE_GOLDEN=1` で golden を更新できる。

4. **統合 golden（read model 層）**: 同じ fixture を projector まで通した `SessionReadModel` スナップショットを golden と比較し、不一致で fail する。

5. **自作比較・依存追加なし**: golden 比較は新規 crate 依存を追加せず自作する（既存 `cli/review.rs` の golden 前例と同方針）。

6. **代表 turn の網羅**: Claude fixture は text / thinking / tool_use / permission / result を含む通常 turn を、Codex fixture は agentMessage / commandExecution / requestApproval / turn completed を含む通常 turn を対象に、それぞれ replay テストが通る。

7. **ドキュメント**: fixture ディレクトリの README に golden 更新手順・採取手順・マスキング方針を記載する。

8. **CI 統合**: 上記テストが `cargo test` で実行され、CI（`.github/workflows/ci.yml`）で走る。

9. **挙動不変**: 本 issue の変更で convert / projector の出力（＝生成される golden）を変えない。record tap 追加は既存の読み取りループの挙動を変えない。

## 受け入れ基準の概要

- [ ] Claude fixture（text / thinking / tool_use / permission / result を含む通常 turn）で replay テストが通る。
- [ ] Codex fixture（agentMessage / commandExecution / requestApproval / turn completed を含む通常 turn）で replay テストが通る。
- [ ] convert 層 golden（`AgentRuntimeEvent` 列 ＋ Claude `auto_responses`）と read model 層 golden（`SessionReadModel`）の両方が比較対象になっている。
- [ ] `RELEASH_WIRE_RECORD=<dir>` で実セッションから fixture を採取でき、未設定時は本番挙動に影響しない。
- [ ] golden 更新手順（`UPDATE_GOLDEN=1` 等）が fixture ディレクトリの README に記載されている。
- [ ] replay / 統合 golden テストが `cargo test`（CI）で実行される。
- [ ] `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

以下は本 issue と正本文書から判断できる範囲で置いた仮定。設計・実装で覆る場合は該当箇所を更新する。

1. **spec-id / ディレクトリ**: ブランチ `feat/issues/1383` に対応し、spec ディレクトリは `docs/specs/feat-issues-1383/` とする。

2. **record tap は本 issue の成果物に含める**: 受け入れ基準の「golden 更新手順を README に記載」は採取機構の存在を前提とするため、`RELEASH_WIRE_RECORD` tap を production コードとして実装・マージする。ただし既定無効・非破壊とする。

3. **tap の挿入位置**: `stdout_line_reader.rs` が上限内の生行を `Json` / `NonJson` へ分類する直前に挿入し、Claude `process.rs` / Codex `app_server.rs` から backend 別 recorder を接続する。パース失敗行も記録し、8MB 超過行は shared reader が本文を保持しないため挙動不変を優先して記録対象外とする。

4. **golden 形式**: golden は fixture ごとに、convert 層（events ＋ auto_responses）と read model 層（SessionReadModel）を `serde_json` の pretty JSON で保存する。golden ファイルは fixture と同じディレクトリ配下に置く。

5. **マスキング方針**: 絶対パス・ホームディレクトリ・トークン / API キー・メッセージ本文などの秘匿値を安定なプレースホルダへ置換する。構造（type / subtype / フィールド構成 / イベント順序）は保持し、変換挙動の検証に必要な形状は壊さない。マスキングは fixture 格納時に確定させ、リポジトリにはマスク済みのみを commit する。

6. **`UPDATE_GOLDEN` 環境変数**: golden 更新は `UPDATE_GOLDEN=1` を用いる（`cli/review.rs` に UPDATE_GOLDEN の前例は無いため本 issue で新規に導入する）。record tap は既存の環境変数取得方法に倣い `std::env::var_os` を用いる。

7. **AgentRuntimeEvent → 永続 event の写像**: convert が返す `AgentRuntimeEvent` を durable な `AgentSessionEvent` へ写す際は、production の building block と turn 完了順序（最終 parts の `FinalPartsRecorded`、続いて terminal event）を統合 golden で再利用する。

8. **依存追加なし**: `insta` / `goldenfile` 等の golden crate は導入しない（現状 `Cargo.toml` に無く、issue も自作比較を指定）。

## Open Questions

なし。
