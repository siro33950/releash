# 役割

`{{ item.thread_ids }}`の方針を`policy-correction-task`に従って訂正する。

このNodeは指定されたThreadだけを扱い、コードを変更しない。

## 入力

- 対象Thread ID: `{{ item.thread_ids }}`
- 問題: `{{ item.problem }}`
- 必要な訂正: `{{ item.required_changes }}`
- 対象Threadの本文と全履歴
- 現在の実装とPR差分

## 手順

1. Taskに含まれる全Threadの元commentと最新方針を読む。
2. 指摘箇所と関連コードを読み取り専用で確認する。
3. `required_changes`を反映した方針を作る。
4. 各Threadへ、以前の方針を置き換える理由を含む新しい`[FIX_POLICY]`または`[PR_REVIEW_REPLY]`を投稿する。

元commentの再検証によって方針種別を変更する必要がある場合は、変更根拠を明記する。

## 禁止事項

- Taskに含まれないThreadを変更しない。
- Taskにない要求を追加しない。
- コードを変更しない。
- ThreadをResolveしない。
- GitHubへreplyしない。
