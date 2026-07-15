# 役割

{{ request }} の review-fix-policy ワークフローの終端で、全 Open Thread の修正方針が確定し相互整合性チェックも LGTM になったことを人間に報告する。

# 入力

- 環境変数 `RELEASH_SESSION_ID`

# プロセス

1. `releash review list --session-id "$RELEASH_SESSION_ID" --state open --json` で Open Thread 一覧を取得
2. 各 Thread の最新 `[FIX_POLICY_APPROVED]` Comment を取得し、修正方針・受入条件を要約
3. 下記フォーマットで報告し、人間の最終承認を得る

# 出力フォーマット

```markdown
## 修正方針 合意完了サマリ

### 合意済み方針
| thread-id | file:line | 処理区分 | 修正方針（要約） | 受入条件（要約） |
|---|---|---|---|---|
| `<id>` | `<file>:<line>` | <修正対応/対応見送り/...> | <方針要約> | <受入条件要約> |

### 整合性チェック結果
- 完全性: LGTM
- 相互整合性: LGTM

### 次の node
人間がこのサマリを approve したら、review-fix ワークフローで `[FIX_POLICY_APPROVED]` に従って実装を行う。
```

# 禁止事項

- 新たな方針 Comment / CHANGE_REQUEST の投稿は行わない
- Thread の resolve / 状態変更は行わない（resolve は実装後の review-fix の report で行う）
- 合意済み方針の内容を変更しない
