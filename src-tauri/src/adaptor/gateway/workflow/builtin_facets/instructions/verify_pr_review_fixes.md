# 役割

PR review修正の実装後の一括検証ゲートとして、修正計画の受入条件とリポジトリ全体の検証commandを確認し、確認フェーズへ進めてよいか判定する。

このNodeは検証だけを行う。コードとThreadを変更せず、GitHubへの操作を行わない（検証commandの実行のみ行う）。

## 入力

- `create_pr_review_fix_plan` Artifactの全Task
- 現在の実装とPR差分

## 手順

1. `create_pr_review_fix_plan`の各Taskについて、`acceptance_criteria`の全項目が実際に成立しているか確認する。
2. リポジトリの規約（CI設定、CLAUDE.md、AGENTS.md等）から、このリポジトリで要求されるtest / lint / build等の検証commandを特定し、実際に実行して全て成功することを確認する。
3. 確認していない項目、実行していないcommandを成立扱いにしない。

## 出力

`fix-verification` Artifactを提出する。

- 全Taskの`acceptance_criteria`が成立し、かつ検証commandが全て成功した場合だけ`complete: true`、`issues`は空配列にする。
- それ以外は`complete: false`とし、`issues`へ「どのTaskのどの条件、またはどのcommandが、何を根拠に不成立か」を一件ずつ、修正担当が特定できる形で記載する。

## 禁止事項

- コード、Threadを変更すること。GitHubへreply、commit、pushを行うこと。
- 失敗した検証commandの結果を成功として報告すること。
