# 役割

{{ request }} のフルレビュー後修正 Task を実装する。

この Step は、直前 Step から渡された Task だけを実装対象にする。Open Thread や `[FIX_POLICY_APPROVED]` Comment を直接読んではならない。

# 入力

直前の `check_and_make_tasks` Step が提出した `review-fix-tasks` Artifact に含まれる Task 一覧。

# Task の扱い

実装対象は、渡された `tasks` 配列に含まれる Task のみ。

各 Task について次を確認する。

- `task_id`
- `thread_id`
- `file`
- `objective`
- `acceptance_criteria`
- `non_goals`
- `source_policy`

`acceptance_criteria` は受入条件であり、実装完了判定に使う。省略、読み替え、独自解釈での置き換えは禁止する。

# プロセス

## 1. Task の逐条確認

実装前に、渡された全 Task をそのまま一覧化して確認する。

```markdown
## 実装対象 Task

| task_id | thread_id | file | objective |
| --- | --- | --- | --- |
| `<task_id>` | `<thread_id>` | `<file>` | <objective> |

## 受入条件

### `<task_id>`
- <acceptance_criteria>
```

Task が存在しない場合は、コード変更せず終了する。

## 2. 実装

Task の `objective` と `acceptance_criteria` を満たす最小範囲で実装する。

守ること:

- `non_goals` に含まれる内容は実装しない。
- Task に含まれない Thread や指摘は扱わない。
- 方針が矛盾している、または実装できない場合はコード変更を止め、理由を出力する。
- 既存の設計、命名、テスト配置に合わせる。

## 3. 検証

変更内容に応じて最小限の検証を行う。

- Rust 変更: 関連する `cargo test` / `cargo fmt --check`
- フロントエンド変更: 関連する `pnpm test` / `pnpm lint`
- workflow 定義変更: 関連する builtin workflow / contract の Rust test

検証できない場合は、その理由を明記する。

# 終了時の出力

```markdown
## 実装結果

### 実装内容
- `<変更したファイル>`: <変更内容>

### 対応 Task
| task_id | thread_id | file | 実装対応 | 受入条件の確認 |
| --- | --- | --- | --- | --- |
| `<task_id>` | `<thread_id>` | `<file>` | <実装で何をしたか> | <満たした受入条件> |

### 検証
- `<コマンド>`: <結果>
```

# 禁止事項

- Open Thread、`review get`、`review history` を確認しない。
- `[FIX_POLICY_APPROVED]` を直接読みに行かない。
- Task にない修正を行わない。
- Thread の resolve / comment / 状態変更を行わない。
- Task の方針や受入条件を勝手に変更しない。
