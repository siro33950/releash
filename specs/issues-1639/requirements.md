# Context

- 入力: [Issue #1639](https://github.com/siro33950/releash/issues/1639) — [event store] 旧message projectionの残存データ7.2GBでストアが8.7GBに肥大化している（クリーンアップ経路がない）（OPEN）。要求の正本。本文（現状・経緯・影響）と 2026-08-13 の「対応方針（確定）」コメントからなる。
- 補助資料:
  - [Issue #1640](https://github.com/siro33950/releash/issues/1640) — shutdown reconciliation からの復旧不能問題。2026-08-13 のフリーズ障害の隣接 Issue であり、本変更の対象外境界を定める参照。
  - [PR #1621](https://github.com/siro33950/releash/pull/1621) — MS87 Agent TUI 移行の一括統合（MERGED、v0.4.0）。残存データの書き込み元だった旧 message projection 実装を削除した変更。
- 確定済みの背景:
  - local event store は `~/Library/Application Support/com.releash.app/local-event-store.sqlite3`。SQLite（WAL モード）で、書き込みはプロセスが writer lock file を排他保持する。
  - 旧 message projection 実装は PR #1621 で削除済み。現行コードは `message_projection` を読みも書きもしない。残存データへの最終書き込みは 2026-08-12 00:00 で既に停止している。
  - SQLite は行を削除してもファイルサイズが自動では縮まない。物理サイズの回収には VACUUM 等の操作が別途必要である。
  - 前例として 2026-07-27 に WAL が 18GB まで肥大化してストアが破損し手動再構築した経緯があり、再構築後 126MB のストアが 2.5 週間で 8.7GB へ再肥大化した。掃除経路がない限り肥大化は繰り返し発生する。
- 確定済みの制約（Issue #1639 確定コメント）:
  - 対応は 3 層に分ける。未使用テーブルの削除は schema バージョン移行（一回きり）、物理サイズの回収は store 自身が所有する起動時保守パス（常設）、ファイルシステム層の既存 app data GC は変更しない。
  - 物理サイズ回収を既存 app data GC に置くことは不成立と確定済み（責務がファイルシステム GC と異なる・ストア開放後の background 実行では差し替えが成立しない・通常 VACUUM は writer を長時間占有する、の 3 理由）。
  - 物理縮小の発火条件は `freelist_count / page_count >= 25%` かつ freelist が 64MB 相当以上の AND と確定済み。
  - 縮小はストアの正当性の要件ではなく、失敗時は best-effort で旧ファイルのまま起動を継続する。

# Outcome

- 対象者: Releash を継続運用する利用者（現段階では開発者自身を含む全利用者）。
- 現在の問題: ストア 8.7GB のうち 7.2GB が現行コードの誰も読み書きしない死蔵データであり、毎起動のスキーマ検証がその全ページを読むため、起動コストの大半が不要データに費やされている。さらに残存データを掃除する経路も、ファイルの物理サイズを回収する経路も存在せず、削除しても縮まないまま放置され、再肥大化しても回収できない。
- 変更後に実現する状態: 未使用テーブルとその残存データがストアから削除され、肥大化した既存ストアは起動時に物理サイズが回収される。以後空き領域が再肥大化した場合も、常設の保守経路が起動時に自動で回収する。

# Current Behavior

2026-08-13 時点、branch `feat/issues/1639` で確認。コード参照は `src-tauri/src/` からの相対。

- 実測（Issue #1639、dbstat、2026-08-13 時点）: ストア全体 8.7GB。内訳は `message_projection` 7,227MB（31,787 行）、`events` 476MB、`session_projection` 380MB（892 行）、`logical_commits` 124MB（355,348 行）。`message_projection` の中身は旧実装が書いたメッセージ行ごとの累積全文スナップショット（ordinal 1 が 32KB、ordinal 4,599 で 505KB と O(n²) で蓄積。最大セッションは 1 セッションで 4,599 行・1.2GB）。
- `adaptor/gateway/local_event_store/schema.rs` に `message_projection`（:114）、`terminal_records`（:125）、`stop_resolutions`（:135）の DDL と `idx_message_projection_ordinal`（:210）が残る。`CURRENT_SCHEMA_VERSION` は 4（:10）。
- `src-tauri/src` 全体でこの 3 テーブルへの参照は `schema.rs` の DDL とスキーマ検証だけであり（grep で確認）、実 SQL（`FROM` / `INTO` / `UPDATE` / `DELETE`）は 0 件、対応する domain 型も存在しない。`src-tauri/tests/agent_session_tui_acceptance.rs` の atui_050 テスト群が、旧 message projection 境界を現行コードが使えないことを検証している。
- `VACUUM` および freelist を扱うコードは `src-tauri/src` に存在しない（grep 0 件）。ストアの物理サイズを回収する経路はどこにもない。
- store open シーケンスのスキーマ検証（`schema.rs:1003` / `:1126` の `PRAGMA integrity_check` は DB ファイル全ページを読み、`require_foreign_key_integrity`（`:1178`）の `PRAGMA foreign_key_check` は全 FK 行を走査する）が毎起動実行され、死蔵データも毎回その対象になる。一方この検証は open シーケンス内に限られ、運転中には走らない。
- 再現手順: 旧実装（v0.4.0 未満）で workflow Command gate のターミナル出力を伴う運用をしたストアを現行版で開く。実際の出力: 3 テーブルと残存データは削除されず、ファイルサイズも維持されたまま、毎起動の全ページ検証が走り続ける。

# Scope / Non-goals

## Scope

- 未使用 3 テーブル（`message_projection` / `terminal_records` / `stop_resolutions`）と付随 index の schema からの削除。schema バージョン移行として行い、既存ストア上の残存データの削除を含む。
- store 起動時の物理サイズ回収経路（store 自身が所有する保守パス）の新設。

## Non-goals

- 2026-08-13 のアプリフリーズの原因調査・修正。本 Issue の残存データは運転中フリーズの説明にならないことが確定済みで、スコープ外として別途調査する（#1640 / #1641）。
- shutdown reconciliation からの復旧不能問題（#1640）。
- 既存 app data GC（ファイルシステム GC）の変更。
- 未使用 3 テーブル以外の 15 テーブルおよび `store_metadata` のスキーマ・データの変更。
- 起動後（運転中）の物理サイズ回収の実行。

# Requirements

- R-001: 旧 schema バージョンの既存ストアを現行版で開いたとき、未使用 3 テーブル（`message_projection` / `terminal_records` / `stop_resolutions`）とその index、および残存データがストアから削除されている。
- R-002: 移行後および新規作成のストアに未使用 3 テーブルが存在せず、以後の起動で再作成されない。
- R-003: 空き領域が発火条件（`freelist_count / page_count >= 25%` かつ freelist 64MB 相当以上の AND）を満たすストアを開いたとき、起動時にファイルの物理サイズが回収される。
- R-004: 発火条件を満たさないストアの起動では縮小処理が実行されず、発火判定がファイル走査を伴わない（性能）。
- R-005: 物理縮小が失敗した場合（ディスク不足等）、一時ファイルの残骸を片付けた上で、旧ファイルのまま起動が継続する（安全性）。
- R-006: 縮小・差し替えの前後で、使用中テーブルと `store_metadata` のデータが保持され、差し替え後に旧 `-wal` / `-shm` ファイルに由来する不整合が生じない（互換性・安全性）。

# Assumptions / Open Questions

- Assumption: 物理サイズ回収は store open を同期でブロックする。差し替えが安全に成立する区間が open シーケンス中に限られるため非同期化の選択肢はないことが、Issue #1639 確定コメントで受け入れ済みである。
- Open Questions: なし。
