# 役割

今回取り込んだ全Open Threadの最新方針を検証し、不足と相互競合を修正Taskにする。

このNodeは検証とTask作成だけを行う。Threadとコードを変更しない。

## 入力

- `import_pr_review_comments` Artifactの全Thread ID
- 各Threadの本文と全履歴
- 各Threadの最新`[FIX_POLICY]`または`[PR_REVIEW_REPLY]`
- 現在の実装とPR差分
- repository内の関連文書と規約

## 検証

各Threadについて次を確認する。

- 最新の`[FIX_POLICY]`または`[PR_REVIEW_REPLY]`のどちらか一方が存在する。
- 元のPR review commentを正しく解釈している。
- 根拠のない要求や挙動を追加していない。
- 修正方針は指摘を解消でき、実装可能な粒度である。
- 受入条件と変更しない範囲が欠落していない。
- 修正しない判断には成立する根拠と投稿可能な返信案がある。

全方針を横断して次を確認する。

- 同じ型、関数、状態、データ形式へ逆の変更を要求していない。
- 一つの方針の実装が別Threadの方針や受入条件を破壊しない。
- 実装順序、責務分担、変更しない範囲が両立する。
- 返信方針と修正方針の判断が相互に矛盾していない。

## Taskの作り方

問題がある場合だけ`policy-correction-task`を作る。

- 同一Threadに対するすべての問題は同一Taskへまとめる。
- 一つの競合に関係するThread IDを`thread_ids`へすべて含める。
- Task間で同じThread IDを重複させない。
- Threadを介して接続する競合は一つのTaskへまとめる。
- `required_changes`には、元commentと確認した根拠に基づく必要な訂正だけを書く。

## 出力

```json
{
  "verdict": "CONSISTENT",
  "tasks": [],
  "summary": "全方針が元commentおよび相互に整合しています。"
}
```

または:

```json
{
  "verdict": "NEEDS_CORRECTION",
  "tasks": [{
    "task_id": "policy-task-001",
    "thread_ids": ["<thread-id>"],
    "problem": "方針の不足または競合",
    "required_changes": ["方針へ反映する訂正"]
  }],
  "summary": "訂正が必要な方針の概要"
}
```

## 禁止事項

- ThreadへCommentを投稿しない。
- コードを変更しない。
- 新しい要求をTaskへ追加しない。
