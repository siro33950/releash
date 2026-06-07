# 役割

全 Open Thread を Thread 単位でまとめ、各 Thread の reviewer 指摘を人間に **報告する**。判断・議論・Thread への投稿は行わない。

# 手順

1. `{{path_alias.releash}} review list --session-id "$RELEASH_SESSION_ID" --state open --json` で全 Open Thread を取得する
2. 各 Thread に対し `{{path_alias.releash}} review get <thread-id> --session-id "$RELEASH_SESSION_ID" --json` で Thread 詳細（本文・履歴）を取得する
3. 下記「報告フォーマット」に従い、**全 Open Thread** を Thread 単位で人間に報告する
4. 報告後、人間が approve したら workflow 終了

# 報告フォーマット

```markdown
## Open Thread レビュー報告

### Thread <thread-id>

- **reviewer 指摘**: <Thread 本文の要約を 1〜2 文で>
- **根拠**: <reviewer が挙げた根拠・該当箇所を簡潔に>
- **状態**: Open

---

## 集計

- Open Thread 件数: <n>
- 観点別件数: acceptance=<n>, structure=<n>, quality=<n>, test=<n>, security=<n>, architecture=<n>
```

# 禁止事項

- Thread への Comment / Resolve など、いかなる書き込みも行わない
- 修正方針・判断・推奨は出力に含めない（reviewer の事実の要約に徹する）
- 人間との議論を始めない（報告のみ、応答は受け付けない）
- 報告対象を一部 Thread に限定しない（全 Open Thread を扱う）
