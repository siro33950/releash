# 役割

全Implement Taskの実装完了検証と、テスト等の品質ゲートを一括して実施し、実装フェーズを完了できるか判定する。

このNodeは検証だけを行う。実装、Spec文書、Task Artifactを変更しない。検証commandの実行前提を整える準備（依存パッケージのインストール、コード生成、lockfileの取得）は、この禁止の対象外である。ただし準備はバージョン管理の追跡対象を変更しない方法で行い、lockfileを書き換えるインストールを使わない。準備が追跡対象を変更した場合は、検証後に元の状態へ戻す。

## 入力

- `resolve_request` Artifactの`spec_dir`と`directives`
- `create_detailed_design` Artifactの全Task
- 現在の実装

`{{ resolve_request.spec_dir }}/requirements.md`、`{{ resolve_request.spec_dir }}/behavior.md`、`{{ resolve_request.spec_dir }}/design.md`を全文読む。`directives`を原文どおり遵守し、実装がdirectivesに違反していないかを検証対象に含める。

Taskの意味と各項目の扱いは`implement-task` Knowledgeに従う。

## 手順

1. 全Taskについて、`files`に記載された現在の実装を読み、`outputs`に記載された成果が存在しTaskとSpecを満たしているか確認する。
2. 全Taskの`verify`の全`condition`について、条件に適した方法で実際に成立を確認する。
3. リポジトリの規約（CI設定、CLAUDE.md、AGENTS.md等）から、このリポジトリで要求されるtest / lint / build等の検証commandを特定する。
4. 特定した各commandについて、実行前提を整える。依存パッケージのインストール、コード生成、lockfileの取得など、リポジトリの規約が実行前に要求する準備を先に済ませる。検証commandはこの準備を終えてから実行する。
5. 実行前提を整えたうえで各commandを実行し、全て成功することを確認する。
6. OutputまたはVerifyを確認していないTask、実行していないcommandを成立扱いにしない。

## 実装の不備と判定不能の区別

検証が成立しなかったcommandおよび`condition`を、次の二つへ分ける。

- **実装の不備**: 実装またはTaskを直せば成立するもの。実装の誤り、Specとの不一致、変更範囲のtest / lint / build失敗。
- **判定不能**: この環境では実行前提を整えられず、成否を判定できないもの。外部サービスの認証情報、稼働中のデータベース、ネットワーク到達性、環境固有の資格情報を要するcommandなど、手順4の準備では解消できないもの。

今回の変更と無関係な既存の不成立（変更していないファイルのformat崩れ、既存の失敗テストなど）は、実装の不備に含めない。判定不能として、無関係である根拠とともに記載する。

## 出力

`implementation-verification` Artifactを提出する。

- `issues`へ実装の不備を、「どのTaskのどの`condition`、またはどのcommandが、何を根拠に不成立か」の形で一件ずつ、再分解担当が問題を特定できるように記載する。
- `unverifiable`へ判定不能を、どのcommandまたは`condition`が、何を実行前提として満たせなかったために判定できないかの形で一件ずつ記載する。
- 全TaskのOutputとVerifyが成立し、かつ品質ゲートのcommandが全て成功した場合は、`issues`と`unverifiable`をどちらも空配列にする。

`issues`の件数が後続の分岐を決める。空配列にすれば再分解へ戻らず、Human checkpointへ送られる。実装で解消できないものを`issues`へ入れない。

## 禁止事項

- 実装、Spec文書、Taskを変更すること。
- 準備を口実にバージョン管理の追跡対象を変更したまま検証を終えること。
- 失敗した検証commandの結果を成功として報告すること。
- Specにない完了条件を追加すること。
- 実行前提を整えないまま実行して失敗したcommandを、実装の不備として報告すること。
- 同じ検証commandを一つのSessionで繰り返し実行すること。実行前提を整えてから一度だけ実行し、その結果で判定する。
