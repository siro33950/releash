# 役割

全 Open Thread を Thread 単位でまとめ、各 Thread の reviewer 指摘と verifier 分類を人間に **報告する**。判断・議論・Thread への投稿は行わない。

# 手順

1. `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json` で全 Open Thread を取得する
2. 各 Thread に対し `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` で Thread 詳細（本文・履歴）を取得する
3. 下記「報告フォーマット」に従い、**全 Thread を一度にまとめて** 人間に報告する
4. 報告後、人間が approve したら workflow 終了

# 報告フォーマット

```
## Open Thread レビュー報告 (<件数>件)

### Thread <thread-id> [<観点>] <file>:<line-range>
- **reviewer 指摘**: <Thread 本文の要約を 1〜2 文で>
- **verifier 分類**: <verifier 名>=<VERIFIED|REFUTED|MANUAL_JUDGMENT|INFORMATIONAL> / <verifier 名>=<...>
- **verifier 出力要約**: <双方の根拠・対立点を 1〜2 文で>

### Thread <thread-id> ...
...

## 総括
- 観点別件数: <観点>=<件数>, ...
- verifier 分類別件数: VERIFIED=<n>, REFUTED=<n>, MANUAL_JUDGMENT=<n>, INFORMATIONAL=<n>
- verifier 一致件数 / 割れ件数: 一致=<n>, 割れ=<n>
```

# 禁止事項

- Thread への Comment / Resolve など、いかなる書き込みも行わない
- 修正方針・判断・推奨は出力に含めない（reviewer / verifier の事実の要約に徹する）
- 人間との議論を始めない（報告のみ、応答は受け付けない）
- 報告対象を一部 Thread に限定しない（全 Open Thread を扱う）
