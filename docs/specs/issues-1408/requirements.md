# Requirements

## Type

信頼性改善（backend 間の非対称な stdout 処理の共通化）。

agent session の backend（Claude / Codex）が CLI プロセスの stdout を読み取る経路を、共通の行読み取り部品へ集約する。現状は Claude と Codex で堅牢性が非対称であり、Codex は stdout に非 JSON 行が 1 行混ざるだけでセッションが即死する。この非対称を解消し、両 backend が同じ規約で「行サイズ上限」「非 JSON 行の扱い」「破棄・skip の可視化」を持つようにする。

関連: #1408（本 ISSUE、L7）/ milestone 84「Agentチャット安定化」Phase 0（依存なし）/ 監査問題 **SD-3** / ライフサイクル理想形 **I10** / 親方針: `.claude/rules/rust-first-logic.md`、CLAUDE.md「full-retention 設計を避ける」。

## 背景と目的

Releash の agent session は Claude / Codex の CLI を子プロセスとして起動し、その stdout を行単位で読んで wire message へ decode している。監査（SD-3）により、この stdout 読み取りの堅牢性が両 backend で大きく非対称であることが確定している。

### 現状のコード調査（実コードで確認済み）

- **Claude は堅牢**:
  - 非 JSON 行は warn ログを出して skip し、読み続ける（`claude/process.rs:181-186` の `next_stdout_item`）。
  - 1 行のサイズ上限 `MAX_CLAUDE_STDOUT_LINE_BYTES = 8 * 1024 * 1024`（`claude/process.rs:22`）を持ち、上限超過行はバッファに保持せず読み捨てて `ClaudeStdoutLine::Oversize`（`ClaudeStdoutItem::OversizeDropped { bytes }`）にする（`claude/process.rs:211-266`, `:26-28`）。
  - 破棄発生時は runtime state の `oversize_dropped_count` を加算し（`claude/session.rs:403-406`）、Error part 相当の可視化とカウント表示（「サイズ超過破棄 N 件」）を行い、セッションを継続する（`claude/session.rs:345-358`, `:546-556`）。
  - この skip / oversize 挙動はテストで意図的仕様として固定されている（`claude/process.rs:487-534`）。
- **Codex は脆い**:
  - stdout 読み取りは `BufReader::lines()`（`codex/app_server.rs:108`）で、1 行のサイズ上限が無い。巨大な item 行（aggregatedOutput 等）を丸ごとメモリに蓄積しうる。
  - `decode_jsonrpc_line` が非 JSON 行を `Err("invalid app-server JSON-RPC: ...")` にし（`codex/app_server.rs:138-140`）、`next_json` がそのまま伝播する（`codex/app_server.rs:115-126`）。
  - read loop の Err 分岐が Fatal を送って break → `process.shutdown()` で app-server を kill するため、runtime は turn を Crash として complete しセッション実体が終了する（監査 SD-3 記載、`codex/session.rs:411-428` / `runtime/usecase.rs:2536-2564`）。

結果として、「プロトコル外の 1 行」という同じ事象が Claude では無害な警告または 1 件破棄の通知で済むのに対し、Codex では致命的クラッシュ（turn=Crash、セッション終了）になる。CLI や注入環境が stdout に警告等を 1 行出すだけで Codex チャットが突然死する。

本変更の目的は、この非対称を解消し、両 backend の stdout 読み取りに共通の堅牢性契約（サイズ上限・非 JSON 行の扱い・破棄/skip の可視化）を与えることである。

### 正本ドキュメントとの関係

- 監査 **SD-3**（`specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md:409`）が本 ISSUE の問題定義。
- ライフサイクル **I10**（同 `agent-chat-ideal-lifecycle.md:108`）が最終的な理想形。I10 は「共通の分類契約」を掲げ、`Notice(Diagnostic / UnsupportedMessage)` への着地や、未分類・malformed・oversize を `ProtocolIncompatible` として fail-closed で新規 turn を block する将来像を含む。
- ただし `Notice` 語彙（**S5 #1393**）と `ProtocolIncompatible` / schema・wire 基盤（**F 系 / D1 #1445**）は後続 Phase の成果物であり、本 ISSUE（Phase 0・依存なし）はそれらに依存できない。ISSUE 本文も「S5 で `Notice(OversizeDropped/UnsupportedMessage)` に接続。それまでは構造化 warn ログ＋Error part 相当の既存可視化を維持」と明記している。したがって本 Spec は、共通部品の導入と Codex 即死の解消までを担い、Notice / ProtocolIncompatible への着地は後続 ISSUE に委ねる。

## スコープ

- **R1（共通 line reader の新設）** `infrastructure/agent_session/` に、両 backend が使う共通の行読み取り部品を新設する。少なくとも次を持つ:
  - 1 行のサイズ上限（既存 Claude の 8MB を踏襲し、共通定数として定義する）。
  - 上限超過行はバッファに保持せず読み捨て、oversize を上位へ通知する。
  - 非 JSON 行は skip し、skip 件数をカウントする。
  - 破棄（oversize）・skip の発生を呼び出し側へ通知する hook / 戻り値を持つ（S5 で `Notice` へ接続できる形にしておく。それまでは構造化 warn ログ＋既存の Error part 相当の可視化を維持する）。
- **R2（Codex への適用）** `codex/app_server.rs` の read_line 経路を共通部品へ置き換える。非 JSON 行での「decode 失敗 → Fatal → セッション即死」を、skip＋カウントによる継続へ変更する。ただし JSON-RPC の整合性が必要なため、応答待ちの request に対応する行（応答必須の JSON-RPC response）だけは失敗を伝搬してよい。
- **R3（Claude への適用）** `claude/process.rs` / `claude/session.rs` の既存 skip・8MB 破棄経路を共通部品へ移す。破棄時に行種別の推定（先頭 bytes から type を覗く等）を付けて可視化を改善する。既存の意図的仕様（skip / OversizeDropped / カウント表示）は回帰させない。
- **R4（fixture）** 非 JSON 行・巨大行を混ぜた fixture を追加し、両 backend でセッション継続を固定する回帰テストを持つ。fixture は F1（wire record/replay 回帰テスト基盤、#1383）に追加する方針だが、F1 未整備の場合の代替配置は design.md で決める。

## 非スコープ

- `Notice` 語彙（`Notice(OversizeDropped / UnsupportedMessage / Diagnostic)`）の導入と、それへの着地。→ **S5 #1393**。
- `ProtocolIncompatible` / fail-closed による新規 turn block・durable 化・reconciliation。→ D1 #1445 / F 系 / 後続 ISSUE。I10 の「未分類・malformed・oversize は fail-closed」への完全収束は本 ISSUE では扱わない（後述 Open Questions Q1）。
- wire message の typed 化・converter の語彙変更（**F4 #1386 / F5 #1387 / F6 #1388** 以降）。挙動等価の read 経路共通化に閉じ、message の意味変換は変更しない。
- Codex の stall 誤検知・生存 signal（**SD-4 / S1 #1389**）、tool 出力 delta 写像（SD-5）等、stdout 読み取り以外の SD 問題。
- runtime / usecase 側の turn 状態機械そのものの再設計。共通部品からの通知を受ける最小限の配線に留める。
- frontend への新規ロジック追加。可視化は既存の Rust-owned read model 経由に留める。

## 要求事項

- **R1**: `infrastructure/agent_session/` に共通の stdout 行読み取り部品が存在し、両 backend がそれを使う。部品は「1 行サイズ上限（共通定数）」「上限超過行の読み捨て」「非 JSON 行の skip とカウント」「破棄/skip の呼び出し側通知」を提供すること。
- **R2**: Codex の stdout に非 JSON 行（警告・診断ログ等）が 1 行以上混ざっても、セッションが即死せず継続すること。従来 Fatal で落ちていた経路が skip＋カウントに変わること。ただし応答必須の JSON-RPC response の decode 失敗は従来どおり失敗として扱ってよい。
- **R3**: Codex の stdout 1 行に対し、Claude と同じ 1 行サイズ上限が効き、上限超過行でメモリが無制限に肥大しないこと。
- **R4**: Claude の既存挙動（非 JSON 行 skip、8MB 超破棄、`oversize_dropped_count` によるカウント表示、Error part 相当の可視化、セッション継続）が回帰しないこと。
- **R5**: 破棄（oversize）・skip の発生件数が、両 backend で可視化（少なくとも構造化 warn ログとカウント）されること。可視化の source of truth は Rust 側 runtime state / read model にあり、frontend は表示のみに留まること。
- **R6**: Claude の破棄時に、破棄行の種別推定を付与して可視化が改善されること（先頭 bytes からの type 推定等、手段は design.md で決める）。
- **R7**: 共通部品の破棄/skip 通知経路が、後続の S5（`Notice`）へ無改造に近い形で接続できる拡張点を持つこと（本 ISSUE では `Notice` に接続しない）。
- **R8**: 非 JSON 行・巨大行を混ぜた fixture により、両 backend でセッションが継続することが自動テストで固定されること。
- **R9**: full-retention / full-recompute 経路を新設しないこと。巨大行は保持せず読み捨て、カウント等の delta / summary のみを state に持つこと。

## 受け入れ基準の概要

- **Rust test**
  - Codex: 非 JSON 行を含む stdout でセッションが継続し、skip がカウントされる（R2 / R5）。
  - Codex: 1 行サイズ上限超過行が読み捨てられ、後続行の処理が継続する（R3）。
  - Codex: 応答必須の JSON-RPC response の decode 失敗が従来どおり失敗として扱われる（R2 の例外条件）。
  - Claude: 既存の skip / OversizeDropped / カウント表示テストが回帰しない（R4）。
  - Claude: 破棄時に行種別推定が付与される（R6）。
  - 共通部品の単体テスト（サイズ上限・非 JSON skip・通知 hook）（R1 / R7）。
- **fixture / 回帰**
  - 非 JSON 行・巨大行を混ぜた fixture で両 backend のセッション継続が固定される（R8）。
- **ISSUE 記載の受け入れ基準**
  - [ ] codex stdout に非 JSON 行が混ざってもセッションが継続する（R2）。
  - [ ] 破棄・skip がカウント可視化される（R5）。
  - [ ] 巨大行でメモリ肥大しない（上限が両 backend に効く）（R3）。
- **品質ゲート**: `pnpm lint` / `pnpm test` / `pnpm build`（frontend 変更がある場合）/ `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- 本 ISSUE は milestone 84 Phase 0（依存なし）であり、`Notice` 語彙（S5 #1393）・`ProtocolIncompatible`・schema/wire 基盤（F 系 / D1 #1445）には依存しない。よって本 ISSUE の可視化は、既存の構造化 warn ログ＋Error part 相当＋カウント表示を維持し、`Notice` / `ProtocolIncompatible` への着地は後続 ISSUE で行う（ISSUE 本文の明記に基づく）。
- 共通行読み取り部品の配置は `infrastructure/agent_session/`（例: 新規 module）とし、Claude 既存の `MAX_CLAUDE_STDOUT_LINE_BYTES = 8MB` を共通定数の初期値として踏襲する。定数名・module 名は design.md で確定する。
- Codex 側は現状 `BufReader::lines()` を使っており文字列ベースだが、Claude 側はバイト境界でサイズ上限を扱う（8MB を超える行のメモリ肥大を防ぐため）。共通部品はバイト境界の読み取りを基準とし、Codex もバイトベースへ寄せる前提とする（UTF-8 変換は decode 前の分類後に行う）。詳細は design.md で決める。
- Codex の「応答必須 request に対する行だけ失敗を伝搬」の判定（in-flight request id の追跡）は、共通部品ではなく Codex 側の呼び出し層で行う前提とする。共通部品は「1 行が JSON か非 JSON か」「サイズ上限」だけを返し、JSON-RPC 整合性の判断は backend 側に置く。
- 破棄・skip カウントは runtime state の既存 `oversize_dropped_count` 相当を拡張して持ち、full-retention を増やさない。

## 確定事項（旧 Open Questions）

- **Q1（確定: 案 A）**: 本 ISSUE における「非 JSON 行の扱い」は **案 A（ISSUE 本文準拠 / 暫定 skip＋継続）** で確定する。非 JSON 行は種別を問わず skip＋カウントして継続し、Codex 即死を解消する。I10 が求める「未分類・malformed・oversize は `ProtocolIncompatible` として fail-closed で新規 turn を block」は本 ISSUE では実装せず、S5（#1393）/ `ProtocolIncompatible`（後続 Phase）導入時に後続 ISSUE で収束させる。この方針は Phase 0・依存なしという層化、および ISSUE 本文・受け入れ基準（「非 JSON 行が混ざってもセッションが継続する」）に整合する。

## Open Questions

なし。
