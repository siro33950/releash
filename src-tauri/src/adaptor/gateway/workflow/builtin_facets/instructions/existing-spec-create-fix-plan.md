# 役割

Spec、Open Threadの指摘、最新`[FIX_POLICY]`を読み、実装可能な一つの修正計画を作成する。

このNodeは計画だけを行う。コード、Spec文書、Threadを変更しない。

## 入力

- `{{ main.spec_dir }}/requirements.md`
- `{{ main.spec_dir }}/behavior.md`
- `{{ main.spec_dir }}/design.md`
- 全Open Threadの本文と全履歴
- 各Threadの最新`[FIX_POLICY]`
- 現在の実装と差分

## 手順

1. Open Threadごとに元指摘と最新`[FIX_POLICY]`を確定する。
2. 現在の実装で変更が必要なファイルと責務を特定する。
3. Thread間の依存関係を考慮して実装順序を決める。
4. 一つのThreadに対する変更を一つのTaskとして記載する。
5. 各Taskへ、実装手順、受入条件、変更しない範囲、根拠となる方針を欠落なく入れる。

方針整合性の上限到達によって未解消の問題が残っている場合、その問題を隠さず`summary`へ記載する。新しい要求を作って競合を解消したことにしてはならない。

## 出力

`fix-plan` Artifactを提出する。

```json
{
  "tasks": [
    {
      "task_id": "fix-task-001",
      "thread_id": "<thread-id>",
      "target_files": ["src/example.rs"],
      "implementation_steps": ["実装手順"],
      "acceptance_criteria": ["最新FIX_POLICYの受入条件"],
      "non_goals": ["変更しない範囲"],
      "source_policy": "最新[FIX_POLICY]の全文"
    }
  ],
  "summary": "実装順序、依存関係、残っている方針競合の概要"
}
```

Task配列の順序を実装順序とする。
