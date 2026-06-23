# Releash New Relic Terraform

このディレクトリは Releash の New Relic 側の観測リソースを管理する。

- `newrelic_one_dashboard`: #1209 メトリクスと M0 性能予算のダッシュボード
- `newrelic_alert_policy` / `newrelic_nrql_alert_condition`: 性能予算、クラッシュ、ストリーム品質、ペイロードのアラート
- `newrelic_metric_pruning_rule`: メトリクス取り込み量 / カーディナリティ制御
- HCP Terraform: リモート state と手動 CLI 実行

アプリ側の計装はここでは変更しない。

## 入力

New Relic 認証情報と HCP Terraform トークンは 1Password CLI (`op`) から読み込む。1Password CLI のアカウントは `my.1password.com` を明示する。

```bash
export TF_TOKEN_app_terraform_io="$(op read --account my.1password.com op://releash/hcp-terraform/token)"
export TF_VAR_newrelic_account_id="$(op read --account my.1password.com op://releash/new-relic/account-id)"
export TF_VAR_newrelic_api_key="$(op read --account my.1password.com op://releash/new-relic/user-key)"
```

New Relic リージョンが `US` 以外の場合だけ指定する。

```bash
export TF_VAR_newrelic_region="EU"
```

実値を `*.tfvars`、HCP Terraform トークン、API キーとしてリポジトリへコミットしない。

## バックエンド

リモート state と手動 CLI 実行は HCP Terraform を使う。

- 組織: `releash`
- ワークスペース: `newrelic`

`TF_TOKEN_app_terraform_io` を `op` から読み込んだうえで初期化する。

```bash
export TF_TOKEN_app_terraform_io="$(op read --account my.1password.com op://releash/hcp-terraform/token)"
terraform init
```

## 手動検証

バックエンド認証なしで構文・整形・テストだけ確認する場合:

```bash
terraform init -backend=false
terraform fmt -check
terraform validate
terraform test
```

`terraform validate` / `terraform test` はプロバイダープラグインを起動するため、ローカル環境によっては Unix ソケット作成が許可される環境で実行する必要がある。

## 手動 plan / apply

`op` から必要な値を読み込み、HCP Terraform バックエンドを初期化してから plan / apply する。

```bash
export TF_TOKEN_app_terraform_io="$(op read --account my.1password.com op://releash/hcp-terraform/token)"
export TF_VAR_newrelic_account_id="$(op read --account my.1password.com op://releash/new-relic/account-id)"
export TF_VAR_newrelic_api_key="$(op read --account my.1password.com op://releash/new-relic/user-key)"

terraform init
terraform plan
terraform apply
```

初回 plan は全リソースの新規作成になる想定。New Relic アカウントに手動構築済みリソースは存在しない前提なので、`terraform import` はこの展開では不要。

Terraform は CI から実行しない。検証、plan、apply はこのディレクトリで作業者が手動実行する。

## データ管理

New Relic Pipeline Control Cloud Rules は Advanced Compute の有料アドオンなので、この Terraform では使わない。`newrelic_pipeline_cloud_rule` は定義しない。

無料枠の保護は、次元付きメトリクス集計の高カーディナリティ属性を `newrelic_metric_pruning_rule` で削除することで行う。Log / Span の保存前 drop はこの構成では行わないため、PII / 絶対パスを送らない責務は #1209 のアプリ側 allowlist と path scrub が持つ。

## 無料枠の取り込み制御

#1209 は高頻度テレメトリを次元付き `Metric` としてエクスポートする。クラッシュ報告は `releash.error` ロガー経由で OTLP ログプロバイダーに接続されるが、通常の `log::debug!` 出力は New Relic ログエクスポーターに接続されていない。

新しい PII / パス属性キーをメトリクスへ追加する場合は、同じ変更で `local.pii_attribute_keys` も更新する。
