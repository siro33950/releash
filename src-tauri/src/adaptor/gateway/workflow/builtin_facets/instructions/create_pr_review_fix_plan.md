# 役割

今回取り込んだThreadの最新`[FIX_POLICY]`を読み、実装可能な一つの修正計画を作成する。

このNodeは計画だけを行う。コードとThreadを変更しない。

## 入力

- `import_pr_review_comments` Artifactの全Thread ID
- 各Threadの本文と全履歴
- 各Threadの最新`[FIX_POLICY]`または`[PR_REVIEW_REPLY]`
- `check_pr_review_fix_policy_consistency` Artifact
- 現在の実装とPR差分

## 手順

1. 各Threadの最新方針を確定する。
2. `[FIX_POLICY]`があるThreadだけを実装Taskにする。
3. 現在の実装で変更が必要なファイルと責務を特定する。
4. 一つのThreadに対する変更を一つのTaskとして記載する。
5. Thread間の依存関係を考慮し、Task配列を実装順に並べる。
6. 実装手順、受入条件、変更しない範囲、根拠となる方針を欠落なく入れる。

`[PR_REVIEW_REPLY]`だけがあるThreadはTaskに含めない。

方針整合性の上限到達によって問題が残っている場合は、隠さず`summary`へ記載する。新しい要求を作って解消したことにしてはならない。

## 出力

```json
{
  "tasks": [{
    "task_id": "fix-task-001",
    "thread_id": "<thread-id>",
    "target_files": ["src/example.rs"],
    "implementation_steps": ["実装手順"],
    "acceptance_criteria": ["最新FIX_POLICYの受入条件"],
    "non_goals": ["変更しない範囲"],
    "source_policy": "最新[FIX_POLICY]の全文"
  }],
  "summary": "実装順序、依存関係、残っている方針問題の概要"
}
```

修正Taskがない場合は`tasks: []`とする。

## 禁止事項

- `[PR_REVIEW_REPLY]`のThreadを実装Taskにしない。
- 方針にない修正を追加しない。
- コード、Thread、GitHubを変更しない。
