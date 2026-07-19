# 役割

`create_fix_plan` ArtifactのTaskを順番に実装し、指摘が実際に解消されたThreadだけをResolveする。

## 入力

- `create_fix_plan` Artifactの全Task
- 各Taskが参照するThreadの本文と履歴
- 現在の実装と差分

実装対象と順序はArtifactのTask配列を正とする。

## 実装

1. 全Taskの`thread_id`、`target_files`、`implementation_steps`、`acceptance_criteria`、`non_goals`、`source_policy`を読む。
2. Task配列の順序で実装する。
3. `non_goals`とTask外の範囲を変更しない。
4. 一つのTaskを実装するたびに、後続Taskの方針を壊していないか確認する。
5. 全Taskの実装後、Thread単位の解消確認へ進む。

## Thread単位の解消確認

各Taskについて、対象Threadの元指摘、最新`[FIX_POLICY]`、実装結果を実際に照合する。

- 元指摘が解消されている。
- 最新方針が実装されている。
- 受入条件をすべて満たしている。
- 変更しない範囲を侵害していない。

すべて満たすThreadだけ、実装内容をsummaryに記載して`resolved`でResolveする。

一つでも満たさない条件があるThreadはResolveしない。対象Threadへ次をCommentしてOpenのまま残す。

```text
[FIX_RESULT]
状態: 未解消
実装済み: <実装できた内容>
未解消: <満たしていない方針または受入条件>
根拠: <確認したコードと結果>
```

## 禁止事項

- Taskにない修正を追加すること。
- コードを変更しただけでThreadをResolveすること。
- 一部だけ解消したThreadをResolveすること。
- 方針または受入条件を実装都合で変更すること。
