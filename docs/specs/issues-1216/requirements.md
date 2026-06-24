# Requirements

## Type

性能・起動経路改善（runtime lifecycle のリファクタリング）

対象 Issue: #1216「Startup orphan cleanup を non-blocking service 化する」

正本ドキュメント: `docs/releash-performance-architecture-audit.md`（M3 / 項目 4）
マイルストーン: 性能・メモリ効率改善（#80）
実装順: 7-2（Runtime lifecycle / startup）。#1215（runtime cap）と合わせて扱う。

## 背景と目的

`docs/releash-performance-architecture-audit.md` の M3「Runtime lifecycle を締める」の項目 4 として、`cleanup_orphan_processes` を startup の blocking path から外すことが挙げられている。

現状の確認できている挙動は次のとおり:

- アプリ起動時の Tauri `setup` クロージャ（`src-tauri/src/lib.rs` 422-433 付近）で、`cleanup_orphan_processes` を別スレッドで `spawn` した直後に `.join()` しており、**cleanup の完了まで setup が同期的にブロックされる**。`setup` 完了が遅れる分、visible startup（first window ready）が cleanup によって遅延しうる。
- `cleanup_orphan_processes`（`src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` 1066-1167）は、`~/.releash` 配下の `pids` ディレクトリの `*.pid` を走査し、孤児プロセスグループに `SIGTERM` → 最大 **2 秒 sleep** → 生存していれば `SIGKILL` を送る。`.pid` ファイル数や孤児プロセスの終了待ちに比例して所要時間が伸びる（最悪、孤児 1 群あたり 2 秒の sleep が直列に積み上がる）。
- 既存コメント（`lib.rs` 422-423）は「`init_agent_sessions()` より前に完了しなければ、新規 spawn したプロセスを誤って kill しうる」と明記している。`init_agent_sessions`（`bridge_common.rs` 8552 付近）は Tauri command で、frontend から worktree 単位で呼ばれ、session / bridge プロセスの起動につながる。つまり cleanup と新規 spawn の間には**順序保証が必要**だが、現状は「setup 内で cleanup を join し切る」ことに依存しており、command 経由の新規 spawn との間に明示的な ordering 機構はない。
- cleanup は `unix` 限定（`#[cfg(unix)]`）。プロセスの生存判定は `libc::kill(pid, 0)` / `libc::killpg(pgid, 0)`、所有者の同一性は `owner_app_pid` + `owner_start_time`（PID 再利用検出）で判定する。判定不能時は保守的に skip する既存方針（issue #1024）がある。
- cleanup は `log::info!` / `log::warn!` で実行内容を出力するが、cleanup 全体の実行状態・失敗・対象数を**構造化された観測点**としては公開していない。

本 Issue の目的は次の 3 点を満たすこと:

1. orphan process cleanup を startup の blocking path（first window ready を遅延させる経路）から外す。
2. 新規 spawn したプロセスを誤 kill しない順序を、Rust 側の service として保証する。
3. cleanup の実行状態・失敗・対象数を、ユーザーデータを含まない safe metadata として観測できるようにする。

## スコープ

- **cleanup を non-blocking 化する**: `setup` クロージャ内の `.join()` による同期待ちを廃し、cleanup が first window ready をブロックしないようにする。cleanup 自体は引き続き起動時に実行されるが、visible startup の critical path から外す。
- **cleanup と新規 spawn の ordering を Rust service として保証する**: 「cleanup 完了前に起動した新規プロセスを誤って kill しない」ことを、setup でのブロッキング join に依存せず、Rust 側の明示的な順序保証（cleanup 状態を参照する gate / ordering 機構）で担保する。具体的な機構は design で確定する。
- **safe metadata の観測点を追加する**: cleanup の実行状態（未実行 / 実行中 / 完了 / 失敗）、失敗の有無、対象数（走査 PID 数 / 孤児として処理した数など）を、ユーザーデータを含まない形で観測できるようにする。観測の公開先（構造化ログ / telemetry / 内部 read API）は本書の Open Questions / design で確定する（仮定は後述）。
- **race のテスト**: cleanup と新規 process spawn の race（cleanup が新規 spawn を誤 kill しない順序保証）を検証するテストを追加する。
- **ロジックは Rust 側に置く**（`.claude/rules/rust-first-logic.md`）。non-blocking 化・ordering 保証・observation のロジックを frontend に持ち込まない。

## 非スコープ

- PTY / workflow / agent process の idle timeout / cap の enforce（#1215 / M3 項目 3 が担当）。本 Issue は startup orphan cleanup の non-blocking 化と ordering 保証に限定する。
- terminal inactive tab の unmount、PTY lifecycle 分離（#1213 / M3 項目 2 が担当）。
- `bridge_common.rs` の module 分割（M4 項目 2 / #1217 が担当）。本 Issue では cleanup / startup ロジックの module 分割そのものは目的としない。
- cleanup の判定アルゴリズム（PID 再利用検出 / 所有者同一性判定 / SIGTERM→SIGKILL の昇格手順）の変更。既存の保守的 skip 方針（issue #1024）は維持する。
- Windows など非 unix プラットフォームへの cleanup 対応拡張（現状 `#[cfg(unix)]` 限定の範囲を変えない）。
- remote subscriber 向け buffer / broadcast の最小化（M3 項目 5 / 別 Issue）。
- frontend UI への cleanup 状態の表示機能の追加（観測は safe metadata の提供までとし、UI 表示は本 Issue に含めない）。

## 要求事項

1. **non-blocking**: アプリ起動時に orphan cleanup が実行されても、first window ready（visible startup）が cleanup の所要時間（PID 走査・SIGTERM 後の sleep・SIGKILL を含む）によってブロックされないこと。
2. **誤 kill 防止の順序保証**: cleanup 完了前に新規に spawn された agent / bridge プロセスを、cleanup が孤児として誤って kill しないこと。この順序保証が、setup での同期 join ではなく Rust 側の明示的な機構として成立すること。
3. **観測可能性**: cleanup の実行状態（少なくとも 完了 / 失敗 が区別できること）、失敗の有無、対象数（走査・処理した PID 数）を、ユーザーデータを含まない safe metadata として観測できること。
4. **ユーザーデータ非混入**: cleanup に関するログ・metadata に、command body・worktree path・session 本文などのユーザーデータを含めないこと。
5. **race のテスト**: cleanup と新規 process spawn の race（順序保証が破れて新規プロセスを誤 kill しないこと）を検証する自動テストが存在すること。
6. **既存挙動の保全**: 既存の孤児判定（所有者同一性 / PID 再利用検出 / 保守的 skip、issue #1024）と、孤児プロセスの確実な回収（最終的には起動ごとに孤児が掃除される）を壊さないこと。non-blocking 化により「孤児が回収されないまま放置される」状態を新たに作らない。
7. **Rust-first**: 上記ロジックを Rust（Tauri バックエンド）側に実装し、frontend はインターフェースに徹すること。

## 受け入れ基準の概要

- **first window ready が block されない**: 多数の `.pid` ファイルや終了待ちの孤児プロセスが存在する状態でも、first window ready が orphan cleanup によって遅延しない（visible startup の critical path に cleanup が乗らない）ことを確認できる。
- **cleanup と新規 spawn の race がテストされている**: cleanup 実行中／前後に新規プロセスを spawn する状況で、新規プロセスが誤って kill されないことを検証する自動テストが存在し、green であること。
- **safe metadata の観測**: cleanup の実行状態・失敗・対象数を観測でき、その metadata・ログにユーザーデータ（command body / worktree path / session 本文）が含まれないことを確認できる。
- **孤児回収の維持**: 既存の孤児プロセス回収（停止した旧インスタンスのプロセス群が起動時に掃除される）が、non-blocking 化後も成立することを確認できる。

## 仮定

本書では判断できる範囲について以下の仮定を置く。design で再検討・確定する。

- **(仮定) cleanup の起動タイミング**: cleanup は従来どおりアプリ起動時に 1 回起動するが、`setup` の同期経路から外し、バックグラウンドタスク（専用スレッド / async task）として走らせる。`setup` は cleanup の完了を待たずに先へ進む。
- **(仮定) ordering 機構**: 「新規 spawn を誤 kill しない」順序保証は、cleanup の完了状態を表す共有フラグ（例: `AtomicBool` / `OnceCell` / 完了通知）を Rust 側に持ち、cleanup が完了する前に spawn されたプロセスの PID は、その時点の cleanup 対象集合に含めない（または cleanup 側が「自インスタンス起動以降に作成された PID ファイルは対象外」と判定する）ことで担保する。具体方式は design で確定する。
- **observation の公開先（合意済み）**: cleanup の実行状態・失敗・対象数は、構造化ログと既存の telemetry 経路（`other::telemetry`）で観測できるようにする。frontend / 運用向けの専用 read コマンド（Tauri command 等）は本 Issue では追加しない。
- **(仮定) プラットフォーム範囲**: 対象は現状どおり unix（`#[cfg(unix)]`）に限定し、非 unix では従来どおり cleanup を行わない。
- **(仮定) 対象数の定義**: 「対象数」は最低限、走査した `.pid` ファイル数と、孤児として SIGTERM/SIGKILL を送った（または PID ファイルを除去した）数を指す。粒度は design で確定する。

## Open Questions

なし
