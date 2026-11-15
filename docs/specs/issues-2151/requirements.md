# Context

- 入力文書は [#2151「event store の読み取りが期限を過ぎると接続にトランザクションが残り、以後の読み取りが再起動まで失敗し続ける」](https://github.com/siro33950/releash/issues/2151) である。
- 対象は、Releash のサーバ（releashd）が行う event store の読み取りである。
- 画面、CLI、hook、workflow の実行は、event store の読み取りで状態を得る。
- event store の読み取りは 1 件ごとに 2 秒の期限を持つ。サーバは、期限を過ぎた読み取りを打ち切り、失敗として返す。

# Outcome

対象者は、Releash で workflow と Session を動かす利用者である。

現在は、event store の読み取りが 1 件期限を過ぎると、その後の読み取りが、サーバを再起動するまで失敗し続ける。
実行中の workflow は先へ進めなくなり、利用者はサーバを強制停止して再起動するしかない。

変更後は、期限を過ぎた読み取りがあっても、後続の読み取りはサーバを再起動せずに成功し、event store に書き込まれた記録を返す。

# Current Behavior

2026-11-16 に、利用者の Mac の `/Applications/Releash.app`（版 0.5.3）のログと、SQLite 単体の再現で確認した。

## 起きた障害

`~/Library/Application Support/releash/logs/releash.log` に、次の記録がある。

| 時刻 | 記録 |
|---|---|
| 01:44:08 | workflow（development-v4）の qa が 24 個の Session を同時に起動した（#2151 の記載による） |
| 01:45:17.054 | `local event store read failure [<id>]: interrupted` が 1 回出た |
| 01:45:17.059〜01:46:20.675 | `local event store read failure [<id>]: cannot start a transaction within a transaction` が 1033 回出た |
| 01:46:31 | 利用者がサーバを強制停止して再起動した（#2151 の記載による） |

- `interrupted` は、このログの中でこの 1 回だけである。
- `cannot start a transaction within a transaction` は、`interrupted` の 5 ミリ秒後に始まり、サーバの再起動まで続いた。
- 同じ時間帯に、`failed to stop AgentSession … StorageFailure` と `failed to observe AgentSession terminal exit … StorageFailure` が出ている。Session の停止と終了の記録が、読み取りの失敗で進まなかった。
- #2151 によると、再起動の前に、Session の 1 つが「Thread の詳細の再取得が内部エラーになった」と報告している。
- 01:46:31 の再起動のあと、このログに `cannot start a transaction within a transaction` は出ていない。

## SQLite 単体での再現

Python の sqlite3（SQLite 3.50.4）で、次の手順を実行した。

1. WAL のデータベースを作り、1 行を書き込む。
2. 読み取り専用の接続で `BEGIN` と `SELECT` を実行する。
3. progress handler が打ち切りを返す状態にして、`ROLLBACK` を実行する。
4. progress handler を外し、同じ接続で `BEGIN` を実行する。
5. 別の接続で 1 行を書き込み、読み取り専用の接続で行数を読む。

実際の出力は次のとおりである。

```
ROLLBACK -> interrupted
in_transaction after ROLLBACK: True
next BEGIN -> cannot start a transaction within a transaction
writer sees: (2,) stuck reader sees: (1,)
```

- 打ち切られた `ROLLBACK` は、接続にトランザクションを残す。次の `BEGIN` は、サーバのログと同じ文言で失敗する。
- トランザクションが残った接続は、そのあとに書き込まれた行を読めない。
- `interrupt()` を別のスレッドから呼び続けながら `ROLLBACK` を実行した場合も、3000 回中 202 回でトランザクションが残った。

## 確認していないこと

- サーバそのものでの再現。再現は SQLite 単体で行った。
- 01:45:17 に期限を過ぎた読み取りが、どの読み取りだったか。期限を過ぎた理由が、処理の順番を待った時間なのか、読み取りの実行時間なのかも分かっていない。
- 01:46 にログへ出た `local event store corrupt read: stream=workspace-state:… head=125 missing event` の原因。

# Scope / Non-goals

## Scope

- 期限を過ぎて失敗した読み取りのあとの、event store の読み取りの回復。

## Non-goals

- 読み取りが期限を過ぎること自体を減らす変更。Node の同時実行数に上限を置く仕組みと、読み取りの速度改善を含む。

# Requirements

- R-001: event store の読み取りが期限を過ぎて失敗したあと、後続の event store の読み取りは、サーバを再起動せずに成功する。
- R-002: event store の読み取りが期限を過ぎて失敗したあと、後続の event store の読み取りは、その失敗のあとに event store へ書き込まれた記録を返す。

# Assumptions

なし

# Invariants

なし

# Risks

- サーバそのものでは障害を再現していない。原因は、#2151 が挙げるコードの読解と SQLite 単体の再現から導いた。サーバで読み取りが失敗し続けた原因がこれと異なる場合、R-001 と R-002 を満たせない。
