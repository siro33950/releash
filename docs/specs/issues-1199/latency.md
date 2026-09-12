# ローカル WebSocket 往復レイテンシ実測

[Requirements](requirements.md) R-012、[Behavior](behavior.md) B-016、[Design 01](design-01.md) の実測値記録に対応する。

## 結果

2026-09-12（JST）に既存の計測テストを実行した。100 回のウォームアップを除いた 1,000 回の `get_current_branch` の往復結果は次のとおり。全 1,100 回で応答の `result` が `ws-branch` であることを断定し、テストは成功した（1 passed、0 failed）。

| 指標 | 実測値（ms） |
| --- | ---: |
| 最小 | 0.133958 |
| 中央値（昇順 500 番目） | 0.163875 |
| p95（昇順 950 番目） | 0.200292 |
| p99（昇順 990 番目） | 0.234084 |
| 最大 | 0.387083 |
| 平均 | 0.167963 |

計測テストの標準出力をそのまま記録する。

```text
client-ws get_current_branch n=1000 warmup=100 min_ms=0.133958 median_ms=0.163875 p95_ms=0.200292 p99_ms=0.234084 max_ms=0.387083 mean_ms=0.167963
```

## 環境と再実行

- macOS 26.6.2（25G83）、arm64、Apple M4 Pro、物理／論理 14 コア、メモリ 64 GiB。
- rustc 1.96.0（ac68faa20 2026-05-25）、cargo 1.96.0（30a34c682 2026-05-25）。
- `test` profile（unoptimized + debuginfo）、既定 feature、`Cargo.lock` を固定。
- `feat/issues/1199`、HEAD `8f6a107a230af558bc4da79a81da0c6f01b5a311` に A1 実装の未コミット差分を適用した状態。
- loopback への bind が許可された環境で、次のコマンドを `src-tauri/` から実行する。

```sh
cargo test --locked --test client_api test_クライアントws_往復レイテンシ実測 -- --ignored --nocapture --test-threads=1
```

## 計測範囲と判断上の制限

[既存の計測テスト](../../../src-tauri/tests/client_api/mod.rs) は、同一プロセスの Tokio current-thread runtime 上で実際の local API と WebSocket クライアントを動かす。非 master token による認証後の `127.0.0.1` 上の接続 1 本で、要求を逐次送信する。対象はテストが作成した空の tree と 1 commit を持つ一時 Git リポジトリで、共有 dispatch、Rust usecase、git2 による現在ブランチ取得を通る。Tauri は MockRuntime を使用する。

`Instant` による計測区間は、要求 JSON の文字列化の直前から応答 JSON の解析完了までで、WebSocket の送受信と backend の処理を含む。要求オブジェクトの生成、リポジトリ作成、アプリ初期化、接続・認証 handshake、応答値の断定は区間外である。

これは単一実行・単一クライアントの debug 計測であり、desktop renderer の JavaScript と画面描画、release build、別プロセス、多数接続、push 併走時のレイテンシは未計測。後続の A-flip 予算判断では、この条件付きの実測値として参照する。Requirements の Q-004 に従い、許容閾値の追加や GO/NO-GO 判定は行わない。
