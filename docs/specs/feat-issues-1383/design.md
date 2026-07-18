# Design — F1: wire record/replay 回帰テスト基盤

対象 issue: [#1383](https://github.com/siro33950/releash/issues/1383)（milestone 84 Phase 0 / ST-7 前半）
参照: [`requirements.md`](./requirements.md) / [`behavior.md`](./behavior.md)

## 概要

agent chat（Claude / Codex）の wire メッセージ → `AgentRuntimeEvent` → 永続 event → projector → `SessionReadModel` の変換経路について、実 wire ログ由来の fixture を golden と突き合わせる回帰テスト基盤を新設する。あわせて、fixture 採取のための record tap（環境変数ゲート、既定無効・非破壊）を production コードへ追加する。

本 issue は「現状挙動を golden として固定する」ことだけを行い、convert / projector の出力（生成される golden の内容）は変えない。

方針の骨子:

- **record tap**: Claude / Codex が共有する `stdout_line_reader.rs` の生行分類境界に、`RELEASH_WIRE_RECORD=<dir>` 設定時のみ生 wire 行を process-owned writer thread の bounded channel へ `try_send` する tap を挿入する。各 process は backend 別 recorder を shared reader の生成時に渡す。既定では `std::env::var_os` が `None` を返し、channel 初期化もファイル I/O も行わない。reader 終了時は recorder を明示的に shutdown して queue を drain / join する。
- **fixture 置き場**: `infrastructure/agent_session/fixtures/{claude,codex}/<turn名>/` に、マスク済み wire ログ（`wire.jsonl`）と golden 2 種を co-locate する。
- **replay テスト（convert 層）**: fixture を `convert_claude_message` / `convert_jsonrpc_message` に流し、出力を golden と比較する。
- **統合テスト（read model 層）**: crate 最外周の `test_support` で、同じ fixture を convert → 永続 event（既存の building block と production の完了順序を再利用したテスト reducer）→ `project()` まで流し、`SessionReadModel` を golden と比較する。
- **golden 比較は自作**: 新規 crate 依存を追加せず、`UPDATE_GOLDEN=1` で更新できる共通ヘルパを実装する。

## 変更対象

### 新規

| パス | 役割 |
|---|---|
| `src-tauri/src/infrastructure/agent_session/wire_record.rs` | record tap 本体（env ゲート・追記・エラー握りつぶし） |
| `src-tauri/src/infrastructure/agent_session/fixtures/mod.rs` | `#[cfg(test)]` fixture ローダ・golden 比較ヘルパ・replay ドライバ |
| `src-tauri/src/infrastructure/agent_session/fixtures/snapshot.rs` | domain 型へ serde を持ち込まない convert golden 用 snapshot DTO |
| `src-tauri/src/infrastructure/agent_session/fixtures/README.md` | 採取手順・マスキング方針・golden 更新手順 |
| `src-tauri/src/infrastructure/agent_session/fixtures/claude/normal_turn/wire.jsonl` | Claude 代表 turn のマスク済み wire ログ |
| `src-tauri/src/infrastructure/agent_session/fixtures/claude/normal_turn/convert.golden` | Claude convert 層 golden |
| `src-tauri/src/infrastructure/agent_session/fixtures/claude/normal_turn/read_model.golden` | Claude read model 層 golden |
| `src-tauri/src/infrastructure/agent_session/fixtures/codex/normal_turn/wire.jsonl` | Codex 代表 turn のマスク済み wire ログ |
| `src-tauri/src/infrastructure/agent_session/fixtures/codex/normal_turn/convert.golden` | Codex convert 層 golden |
| `src-tauri/src/infrastructure/agent_session/fixtures/codex/normal_turn/read_model.golden` | Codex read model 層 golden |
| `src-tauri/src/test_support/agent_session_wire_replay.rs` | convert から projector までを跨ぐ read model 統合 golden テスト |

### 変更

| パス | 変更内容 |
|---|---|
| `infrastructure/agent_session/stdout_line_reader.rs` | 生行の分類直前に optional recorder tap を挿入 |
| `infrastructure/agent_session/claude/process.rs` | Claude recorder を shared reader の生成時に接続 |
| `infrastructure/agent_session/codex/app_server.rs` | Codex recorder を shared reader の生成時に接続 |
| `infrastructure/agent_session/mod.rs`（or 該当 `mod` 宣言箇所） | `wire_record` / `fixtures` module 宣言を追加 |
| `test_support/mod.rs` | crate 最外周の read model 統合 golden テスト module を追加 |
| `usecase/agent_session/event_log/projector.rs` | read model DTO の golden JSON 化に必要な `Serialize` を test build 限定で追加 |

CI（`.github/workflows/ci.yml`）は既存の `cargo test` 経路で新テストを実行するため変更不要。

## アーキテクチャと責務分割

### record tap（`wire_record`）

- `infrastructure` 層に閉じた薄い副作用ユーティリティ。domain / usecase には依存しない。
- 公開 API（crate 内）:
  - stdout process の生成時に `WireRecorder::from_env(WireBackend)` を生成し、shared reader へ渡す。`RELEASH_WIRE_RECORD` が未設定なら reader は recorder を保持せず、行コピーも行わない。
  - 設定時は生 wire 行を所有権ごと writer thread へ渡す。channel は 256 件、queue が所有する wire 本文は合計 16 MiB を上限とし、どちらかを超えた行は `try_send` 前後で warning を残して best-effort drop する。
  - writer thread が `<dir>/<backend>.jsonl` を一度 open して追記を直列実行するため、stdout を読む Tokio worker では directory 作成・file open・write を行わず、disk I/O 待ちもしない。
  - recorder は stdout process が所有する。session の close / process 交換は child を停止した後に reader task を join し、reader task は recorder の shutdown で queue を drain、file を flush、writer thread を join してから終了する。
  - writer 起動・channel 超過または切断・すべての I/O エラーは `log::warn!` で記録し、呼び出し元の戻り値・制御フローに影響を与えない（非破壊）。
- 記録対象は**生 wire 行**。マスキングは行わない（後述のとおり fixture 化時に別途適用）。採取先 `<dir>` は開発者ローカルの一時ディレクトリを想定し、リポジトリには commit しない。

### fixture / replay ハーネス（`fixtures`、`#[cfg(test)]`）

- fixture ディレクトリを走査し、`wire.jsonl` を 1 行ずつ読む。
- convert ドライバ:
  - Claude: `ClaudeConvertState::new(None, ClaudeWireMode::…)` を 1 turn 分だけ生成し、各行を `convert_claude_message` に流して `events` と `auto_responses` を蓄積する（state は行をまたいで引き継ぐ）。
  - Codex: `CodexConvertState::default()` を 1 turn 分生成し、各行を `convert_jsonrpc_message` に流す。
- convert 層 golden の生成物と、read model 層テストへ渡す `Vec<AgentRuntimeEvent>` の両方をここで供給する。read model 層テスト（crate root の `test_support`）からは、この module の `#[cfg(test)] pub(crate)` ドライバを呼んで convert 結果を得る（convert を二重実行しない）。
- golden 比較ヘルパ `assert_golden(path, actual: &str)`:
  - `UPDATE_GOLDEN` が設定されていれば `path` を `actual` で上書きして pass。
  - 未設定なら既存 golden を読み、不一致なら最初の相違行を含めて `panic!`。golden 不在時は「`UPDATE_GOLDEN=1` で生成せよ」と案内して fail。

### read model 統合（crate root の `test_support`）

- convert が返す `Vec<AgentRuntimeEvent>` を、**テスト reducer** で `Vec<AgentSessionEvent>` に写し、`project()` に渡して `SessionReadModel` を得る。
- テスト reducer は production の `apply_runtime_event`（`runtime/usecase.rs`）を IO 抜きで縮約したもので、`event_log` の既存 building block（`TurnEventLog::begin_turn` / `append_part_events` / `finalize_turn`）をそのまま使う。projection ロジックは複製しない。
- `PartsMerged` は production と同じ `merge_part` で最終 parts を累積し、terminal event の直前に `FinalPartsRecorded` を追加する。これにより projector の最終置換経路と parts の merge・順序を検証する。
- convert は infrastructure、reducer + project は usecase にあるため、複数レイヤーを知る orchestration は crate 最外周の `#[cfg(test)] test_support` に置く。usecase の単体テストから infrastructure への逆依存は作らない。

## データモデルまたは型

### fixture ディレクトリ構成

```
infrastructure/agent_session/fixtures/
  README.md
  claude/
    normal_turn/
      wire.jsonl          # マスク済み生 wire 行（stream-json）
      convert.golden      # convert 層 golden（events + auto_responses）
      read_model.golden   # read model 層 golden（SessionReadModel）
  codex/
    normal_turn/
      wire.jsonl          # マスク済み生 wire 行（JSON-RPC）
      convert.golden
      read_model.golden
```

- 1 fixture = 1 ディレクトリ。golden は fixture と同じディレクトリに co-locate する。
- 命名は `<backend>/<turn名>/`。代表 turn は `normal_turn` とする。

### record tap 型

```rust
pub(crate) enum WireBackend { Claude, Codex }

impl WireBackend {
    fn file_name(self) -> &'static str { … } // "claude.jsonl" / "codex.jsonl"
}

pub(crate) struct WireRecorder { … }

impl WireRecorder {
    pub(crate) fn from_env(backend: WireBackend) -> Self;
    pub(crate) fn record(&self, raw_line: Vec<u8>);
    pub(crate) async fn shutdown(&mut self);
}
```

### golden テキスト形式

- 各 golden は `serde_json::to_string_pretty` による pretty JSON として保存する（末尾改行あり、決定的）。
- convert 層: `line_index` ごとに `events` と（Claude は）`auto_responses` を並べた構造を JSON 化する。fixture module 内で `#[derive(Serialize)]` した snapshot DTO（`ReplayLine` / `RuntimeEventSnapshot` と内包型）へ明示変換する。
- read model 層: `SessionReadModel` を JSON 化する。
- domain 型は転送形式から独立させ、`serde` を追加しない。convert golden の JSON 表現は fixture module 内の snapshot DTO だけが所有する。
- read model は usecase の内部 DTO であるため、`SessionReadModel` と内包する projector DTO には `#[cfg_attr(test, derive(serde::Serialize))]` を追加し、golden 用の JSON 表現を test build に限定する。すでに `Serialize` を持つ `ChatMessage` 等はそのまま利用する。

## 処理フロー

### 採取（開発時）

1. 開発者が `RELEASH_WIRE_RECORD=/tmp/wire cargo run …`（もしくは実バイナリ）で実セッションを実行。
2. shared `StdoutLineReader` が上限内の wire 生行を分類する直前に、process-owned `WireRecorder` が生行を bounded channel へ `try_send` し、専用 thread が `/tmp/wire/{claude,codex}.jsonl` に追記。queue 超過時は本来の stdout 処理を止めず、その行だけを warning 付きで drop する。
3. session / reader の終了時に recorder が queue を drain し、file flush と writer join を完了する。
4. 開発者が README の手順でマスキングを適用し、`wire.jsonl` として fixture ディレクトリへ配置。
5. `UPDATE_GOLDEN=1 cargo test` で golden を生成。差分を確認し commit。

### replay（convert 層 / CI・開発時）

1. `wire.jsonl` を 1 行ずつ読み、JSON へパース。
2. backend 対応の convert に順に流し、`events`（+ Claude `auto_responses`）を蓄積。
3. テキスト化して `convert.golden` と `assert_golden` で比較。

### 統合（read model 層 / CI・開発時）

1. 上と同じ convert 結果（`Vec<AgentRuntimeEvent>`）を取得。
2. テスト reducer で `Vec<AgentSessionEvent>` に写像:
   - 冒頭で `TurnStarted`（固定 `turn_id` / message id / `at`）を発行。
   - `PartsMerged(parts)` → durable part event を `append_part_events` しつつ、domain の `merge_part` で最終 parts を累積。
   - `PermissionRequested(req)` → production と同じ `DomainMessagePart::Permission` として最終 parts へ merge し、`append_part_events` で `PermissionRequested`（resolved 状態なら `PermissionResolved` も）を生成。
   - `TokenUsageUpdated` → 保持し `TurnCompleted` に反映。
   - `TurnCompleted(result)` → 累積済み最終 parts を `FinalPartsRecorded` として追加後、completed terminal event または `finalize_turn` を追加。
   - `SessionEstablished` / `SlashCommandsUpdated` / `PermissionModeChanged` / `KeepAlive` / `BackendSessionCleared` は durable content を持たないため read model には寄与しない（no-op、記録のみ必要ならコメントで明示）。
3. `project()` で `SessionReadModel` を得てテキスト化し、`read_model.golden` と比較。

### tap 挿入点の詳細

- **共通境界** (`stdout_line_reader.rs`): 上限内の生行を `Json` / `NonJson` へ分類する直前に、active recorder がある場合だけ行をコピーして `record` へ渡す。これにより正常行とパース失敗行の両方を採取する。
- **backend 接続** (`claude/process.rs` / `codex/app_server.rs`): process 生成時に backend 別 `WireRecorder` を shared reader へ渡し、reader task 終了時に明示 shutdown する。`Oversize` は shared reader が生バイトを保持しないため「可能な範囲」で扱い、挙動不変を優先して記録をスキップする。

いずれも tap は戻り値・分岐を変えない挿入のみで、既存の読み取りループ挙動を変更しない。

## エラー処理

- **record tap**: writer thread 起動・channel 上限超過または切断・ディレクトリ作成・ファイル open・write・flush / join の失敗はすべて `log::warn!` で記録して継続。通常の記録は bounded `try_send` のみとし、tap が原因で stdout 消費やセッションが停止・変質してはならない（要求 9 / behavior「挙動は変わらない」）。終了時だけは queue を drain して採取済み末尾を確定する。
- **golden 比較**: 不一致は最初の相違を含めて `panic!`（test fail）。golden 不在は「`UPDATE_GOLDEN=1` を実行せよ」と案内して fail。
- **fixture 不整合**: `wire.jsonl` のパース失敗など fixture 自体の破損はテストを明示的に fail させる（golden 差分と区別できるメッセージにする）。
- **UPDATE_GOLDEN**: 設定時は比較せず上書きして pass（意図した golden 更新経路）。

## テスト方針

本 issue の成果物の中核がテストであり、以下を追加する。

- **replay golden（convert 層）**: Claude / Codex の代表 turn fixture で `convert.golden` と一致することを検証する `#[test]`。
  - Claude 代表 turn: text / thinking / tool_use / permission / result を含む。
  - Codex 代表 turn: agentMessage / commandExecution / requestApproval / turn completed を含む。
- **統合 golden（read model 層）**: 同 fixture を projector まで流し `read_model.golden` と一致することを検証する crate 最外周の `#[test]`（`test_support/agent_session_wire_replay.rs`）。
- **tap gating の unit test**: `wire_record` を直接呼び、
  - 環境変数未設定時にファイルが作られないこと、
  - 設定時に writer channel の順序どおり `<dir>/<backend>.jsonl` へ 1 行 1 メッセージで追記されること、
  - queue の件数上限・byte 上限を超えた行が enqueue されないこと、
  - shutdown 後に enqueue 済みの末尾まで読み出せること、
  を `tempdir` で検証する（外部プロセスは起動しない）。
- **golden ヘルパの unit test**: `assert_golden` の一致 / 不一致 / `UPDATE_GOLDEN` 上書き / golden 不在時の挙動を `tempdir` で検証する。
- 品質チェック: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を通す。

## リスクと代替案

- **テスト reducer と production 経路の乖離**: 統合 golden の reducer は `apply_runtime_event` の縮約であり、両者がずれると「read model の現状挙動を固定する」意図が損なわれる。緩和策として、reducer は projection を複製せず既存 building block（`merge_part` / `append_part_events` / `finalize_turn`）を再利用し、production と同じ `FinalPartsRecorded` → terminal event の順序を固定する。将来 T1（#1416）で production 経路を直接通す E2E に置き換える余地を残す。
- **層またぎテストの依存**: convert と projector を跨ぐ orchestration は複数レイヤーを知る必要がある。内側の usecase test module には置かず、crate 最外周の `test_support` に閉じることで production とテストの双方で逆依存を作らない。
- **golden の脆さ**: golden は決定的でなければならない。convert / read model の出力に非決定要素（HashMap 順序・タイムスタンプ・生成 id）が混ざると偽陽性になる。緩和策として reducer 側の `at` / message id は固定値、wire 由来の id はマスク済み安定値を使う。出力に map 由来の順序依存が無いことを実装時に確認する。
- **tap による本番影響**: env 未設定時に channel 初期化も I/O も行わないこと、設定時は process-owned writer thread へ所有権を渡して stdout loop から blocking I/O を隔離し、失敗を握りつぶすことで非破壊を担保する。ホットパスに残る処理は byte reservation と bounded `try_send` だけであり、queue 超過時も stdout loop を待たせない。
- **マスキングの網羅性**: マスク漏れで秘匿情報が commit されるリスク。緩和策として README にマスキング対象チェックリストを明記し、commit 前レビューで確認する（自動マスキングツールの整備は本 issue 非スコープ）。
- **snapshot DTO の保守**: `AgentRuntimeEvent` の変種追加時は fixture module の snapshot DTO も明示更新が必要になる。domain へ転送形式を持ち込まない代わりに、網羅的な `match` をコンパイル時の更新点として使い、golden の JSON 表現を fixture 側へ閉じる。

## 仮定

`requirements.md` / `behavior.md` の仮定に加え、設計上以下を置く。いずれも本文中で扱いを明示済み。

1. **record tap のファイル分割**: 採取先は backend ごとに `<dir>/claude.jsonl` / `<dir>/codex.jsonl` の 1 ファイルへ追記する。複数セッション同時採取時の行 interleave は開発時採取（単一セッション想定）では問題にしない。
2. **tap は生 wire を採取しマスキングは行わない**: マスキングは fixture 化時に別途適用する（要求仮定 5）。採取先ディレクトリはリポジトリ外・gitignore 対象とし、マスク前データを commit しない運用とする。
3. **代表 turn は各 backend 1 fixture（`normal_turn`）**: 受け入れ基準の要素を 1 turn で満たす。拡充は後続 issue の必要に応じて追加する（要求 非スコープ）。
4. **統合 golden の写像はテスト reducer で行う**: `AgentRuntimeEvent → AgentSessionEvent` の pure な production 共有関数は存在しない（`apply_runtime_event` は runtime 状態に密結合）ため、既存 building block と production の `FinalPartsRecorded` → terminal event 順序を再利用したテスト reducer を crate 最外周に置く。
5. **Oversize / drop 行の採取は best-effort**: Claude の oversize 行は生バイトを保持しないため採取をスキップする。パース失敗行は生行を保持するため採取する（behavior「可能な範囲で残す」）。
6. **CI 変更不要**: 新テストは通常の `cargo test` で実行され、既存 CI 経路に含まれる。
7. **golden は serde_json pretty JSON**: 要求仮定 4 どおり JSON を採用する（人間の合意により確定）。domain の転送形式非依存を保つため convert 出力は fixture 専用 snapshot DTO、read model は test build に限って usecase DTO 自身を serialize する。

## Open Questions

なし。
