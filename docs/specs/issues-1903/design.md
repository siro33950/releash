# Design

パスは、`.github/`・`scripts/`・`docs/`・`AGENTS.md` で始まるもの以外は `src-tauri/` 起点。

## 変える部分

- crate・lib・bin の改名: `Cargo.toml` の package 名（`:9`）と bin 名（`:25`）を `releashd` にし、`[lib] name = "releash_lib"`（`:22`）の指定を外して lib 名を既定（`releashd`）にする。自分への dev-dependency（`:89`）、`releash-desktop/Cargo.toml:12,39,40,47` の依存と feature、`Cargo.lock` を追従させ、`releash_lib::` の参照（Tauri シェル、テスト）を `releashd::` にする。根拠: R-001「サーバの crate と実行ファイルの名前は `releashd` である」、B-001。ルート: root package の置き場所（`src-tauri/` 直下）は変えない。`_lib` を付けていた理由は Windows での名前の衝突（`Cargo.toml:19-21`）で、対応プラットフォームは macOS だけのため外す。
- 引数なしの起動: `releashd` を引数なしで起動したとき foreground のサーバとして動かす（今は usage を出して exit 2。`src/bin/backend.rs:9-12`）。`--data-dir <DIR>` も受ける。根拠: R-002、R-003、B-002、B-003。ルート: `--internal-daemon [DATA_DIR]` と `--internal-background-worker` は画面と background worker のために残し、ヘルプに出さないのは今のまま。`--internal-daemon` の DATA_DIR は、次の data dir の規則の「明示指定」として渡す（規則を 2 つにしない）。
- data dir の規則の共有: `--data-dir` ＞ `RELEASH_DATA_DIR` ＞ 既定 の規則を `releash-sdk/src/data_dir.rs` に置き、CLI と `releashd` が同じ関数を使う。根拠: R-003「CLI（`releash`）と同じ規則である」、B-003。ルート: CLI の `fn data_dir`（`releash/src/lib.rs:54-71`）は消す。サーバの既定だけの解決（`src/lib.rs:37-40` → `src/infrastructure/platform/app_data_dir.rs:5-7`）はこの関数に置き換える。background worker もこの関数で data dir を決める。daemon は子プロセスへ渡す `RELEASH_DATA_DIR` を解決した自分の data dir に揃え、別の Releash 由来の env を判別する規則（`src/infrastructure/platform/path_aliases.rs` の `known_alias_data_dirs`・`resolve_session_data_dir_env`・`ensure_release_data_dir_env_for_resolved_path`）はこれで置き換えて消す（子プロセスは自分を起動した daemon に届く必要があるため）。
- background worker の起動先: `src/infrastructure/process/background_worker.rs:36,38` の `releash-backend` を `releashd` にする。根拠: R-001、R-006。ルート: 名前で隣（cargo の `deps/` の中では 2 つ上）を探す今の形を変えない。受け入れテストはテストバイナリの中で同じ処理を動かすため、`current_exe()` は使えない。
- 画面の起動先: `releash-desktop/src/desktop.rs:69` の `releash-backend` を `releashd` にする。根拠: R-001、R-006、B-006。ルート: 画面が子プロセスとして起動する経路（`releash-desktop/src/adaptor/gateway/daemon_supervision.rs:163-173` の引数・env・pipe）は変えない。
- 同梱: `scripts/build-desktop-backend.mjs:7,15,19` と `releash-desktop/tauri.conf.bundle.json:8` の `releash-backend` を `releashd` にする。根拠: R-001、B-001。ルート: 委任。
- テスト: `releash-backend` を書いているテスト（`tests/daemon_smoke.rs`、`tests/workflow_control_plane_acceptance_test.rs:40`、`tests/provider_lifecycle_acceptance_test.rs:93`、`tests/agent_session_tui_acceptance.rs:384`、`releash-desktop/tests/desktop_daemon.rs:285,314,319`、`releash-desktop/tests/support/mod.rs:11,14`、`releash/tests/support/diagnostics.rs:12,15`）を `releashd` に追従させる。根拠: R-001。ルート: 委任。
- 単独起動の確認: `releash/tests/support/diagnostics.rs:40-54` のサーバの起動を、引数なし・env `RELEASH_DATA_DIR` で data dir を渡す形に変え、それを使う CLI の統合テスト（`releash/tests/workflow_diagnostics_cli_test.rs` ほか）を単独起動と CLI の接続の確認にする。根拠: R-002、R-005、B-002、B-005。ルート: `tests/daemon_smoke.rs:319-335`（`--internal-daemon` での多重起動の拒否）は画面の経路の確認として今のまま残し、単独起動での多重起動のテストは足さない（排他は store の writer lock で、起動の形に依らない）。
- CI: `.github/workflows/ci.yml:190,192,194,219,221,261,421,522` の `releash-backend` を `releashd` にする。根拠: R-001。ルート: job 名（`server-*`／`server`）と rust-cache の `shared-key` は役割の名前なので変えない。
- 文書: `AGENTS.md`（「構成で押さえる点」の root package と実行ファイルの記述 `:71,76`、「ビルド・テスト・Lint」のコマンド `:105-122`、Tauri 依存の確認のコマンド `:167`）と `docs/architecture/TEST.md:28,33,41` の `releash-backend` を `releashd` にする。根拠: R-001。ルート: 委任。

## 固定するルート

- lib 名は `[lib] name` の指定を外した既定（`releashd`）にする。
- root package の置き場所は `src-tauri/` 直下のまま（#1854 まで変えない）。
- data dir の規則は `releash-sdk/src/data_dir.rs` の 1 つの関数にし、CLI の `fn data_dir` を消す。`--internal-daemon` の DATA_DIR はその関数の明示指定として流す。
- background worker は名前で隣を探す今の形のまま名前だけ変える。
- 単独起動の確認は CLI の統合テストの起動の形（`releash/tests/support/diagnostics.rs:40-54`）で行い、`tests/daemon_smoke.rs:319-335` は変えない。
- CI の job 名と rust-cache の `shared-key` は変えない。`.github/scripts/test-placement.mjs` は変えない（`releash-backend` を含まず、ディレクトリの path だけを見ている。`:62,64,66,89`）。

## 変えないもの

- 画面が子プロセスとして起動する経路（`--internal-daemon`、stdin の EOF での終了、stdout の完了マーカー、`RELEASH_DAEMON_LAUNCH_ID`・`RELEASH_DAEMON_PARENT_PIPE`）。#1904 で消すため。
- 多重起動の排他の仕組み（store の writer lock）。本文の方針どおり。
- 画面（Tauri シェル）の data dir は、今と同じ既定だけで決め、`RELEASH_DATA_DIR` を見ない。R-006 のため。

## 未確定・リスク

- なし
