# Requirements

## Type

リファクタリング

## Goal

`code` ドメイン（ファイル内容〔at_ref／at_branch_base／staged〕、diff、diff_tree、branch_diff、hunk、patch、staging〔差分 Approve〕、language、file_mention、visible/hidden ranges の各責務）を、`docs/architecture/` のクリーンアーキテクチャ規約に従ったレイヤー構成へ移行する。

完了時には、これらの責務がドメイン層・ユースケース層・アダプタ層・インフラ層の各規約に沿って配置され、git2 やファイル I/O といった外部リソースへの依存がインフラ／ゲートウェイ層に閉じ込められた状態になる。あわせて旧構成のモジュールが除去され、これらの責務がレイヤー規約に違反せず単一クレート内で再利用可能になっていることを成功とする。本移行を通じて、フロントエンド・リモートクライアントから見える機能の振る舞いは一切変わらない。

## Background

現状、`code` ドメインに属するロジックは `src-tauri/src/git/`（diff、diff_tree、branch_diff、hunk、lang、stage、共有 util／型）と `src-tauri/src/file_mention.rs` に、レイヤー分離のない構成で実装されている。ビジネスロジックと git2／ファイル I/O が同一モジュール内に混在しており、Tauri コマンド（`git/commands.rs`）からの呼び出しと密結合している。

この状態には以下の課題がある。

- ドメインロジックが git2・ファイル I/O・Tauri に直接依存しており、CLI 等の別エントリポイントから再利用できない。
- ビジネスルールとインフラ詳細が同一モジュールにあるため、ドメイン単位のテストが書きづらい。
- 単一責務に基づくファイル分割が徹底されておらず、責務の所在が追いにくい。

リポジトリ全体で進行中のクリーンアーキテクチャ移行（ミルストーン「[12] クリーンアーキテクチャ移行」）の一環として、`code` ドメインを規約準拠の構成へ移行し、上記課題を解消する。本ドメインは先行する `repository` ドメイン（issue 973、移行済み）に依存し、`repository` 移行時に整理された共有ユーティリティ・共有型を前提とする。

## Users / Actors

- Releash デスクトップアプリを利用するエンドユーザー（diff 閲覧・差分 Approve・ファイル内容表示・メンション補完等の利用者）
- リモートクライアント（モバイル／タブレット）から WebSocket 経由で diff 閲覧・ソース管理を行う利用者
- `code` ドメインの機能を将来再利用する別エントリポイント（CLI 等）
- 本コードベースを保守する開発者

## Scope

- issue 974 が定める責務（ファイル内容〔at_ref／at_branch_base／staged〕、diff、diff_tree、branch_diff、hunk、patch、staging〔差分 Approve〕、language、file_mention、visible/hidden ranges）のロジックを、クリーンアーキテクチャ規約に従ったレイヤー構成へ移行する。
- これらの責務に対応するドメイン層（エンティティ・値オブジェクト・永続化／外部リソースの抽象・ドメインサービス）を新構成で実装する。
- これらの責務の Command 側・Query 側ユースケースを新構成で実装する。
- これらの責務に対する外部リソース（git2、ファイルシステム）アクセスの具体実装を、ドメインが定義する抽象の実装としてゲートウェイ／インフラ層に配置する。
- これらの責務に対応する Tauri コマンドを、ユースケースを呼び出す薄い入口としてアダプタ層へ再配置する。
- これらの責務に対応する WebSocket 入口が存在する場合、薄いハンドラとしてアダプタ層へ再配置する。
- ドメイン層・ユースケース層・ゲートウェイ層に対するテストを追加する。
- 移行完了後、旧構成の対応モジュールを除去する。
- 本ドメインに属する Tauri コマンドの登録方式を、アプリ起動時の DI 配線（`register` 関数経由）に整合させる。
- `code` 移行に必要な範囲で、`git/` 配下に残存する共有ユーティリティ・共有型のうち本ドメインに属するものを整理する。

## Non-goals

- 外部から観測可能な機能の振る舞い（Tauri コマンドの引数・戻り値・エラー表現、WebSocket メッセージ形式、UI 上の操作結果）の変更。
- `repository` ドメインに属する責務（branch、commit、log、worktree、status、repo_paths、git_config）の移行。これは issue 973 で完了済みであり、本移行の対象外とする。
- `code` 以外のドメイン（`comment`、`app_config`、`agent_session` 等）の移行。
- Git 操作そのものの仕様変更・機能追加（新しい Git コマンドや操作の追加、diff／hunk／staging アルゴリズムの挙動変更）。
- フロントエンド／リモートクライアント側のコード変更（バックエンド I/F が不変であるため不要）。

## Requirements

- `code` ドメインの各責務（ファイル内容、diff、diff_tree、branch_diff、hunk、patch、staging、language、file_mention、visible/hidden ranges）が、`docs/architecture/` の各層規約に従って配置されること。
- ドメイン層が git2・ファイル I/O・Tauri などの外部リソースを直接参照しないこと。
- ユースケース層がドメイン層の抽象（trait）のみに依存し、具体的な外部リソース実装に依存しないこと。
- 外部リソースへのアクセスが、ドメインが定義する抽象の実装としてゲートウェイ／インフラ層に閉じ込められること。
- Tauri コマンド／WebSocket ハンドラが、ユースケースを呼び出す薄い入口に徹すること（ビジネスロジックを持たないこと）。
- 移行対象の責務について、ドメイン層・ユースケース層・ゲートウェイ層のテストが存在すること。
- 移行後、旧構成の対応モジュールが除去され、同一責務の重複実装が残らないこと。
- 本ドメインの Tauri コマンドが、起動時 DI 配線（`register` 関数経由）に整合して登録されること。
- 移行前後で、各 Tauri コマンドの引数・戻り値・エラー表現が等価であること。
- 移行前後で、各 WebSocket メッセージの形式・意味が等価であること。

## Constraints

- 外部から観測可能な振る舞い（Tauri コマンド I/F、WebSocket メッセージ形式、diff／hunk／staging／メンション補完の結果）を一切変えない純粋移行とすること。
- レイヤー間の依存方向が規約（`infrastructure → adaptor（controller / gateway / presenter）→ usecase → domain`、依存は内向きのみ）に従い、逆方向（内側の層が外側の層に依存する向き）の依存を生まないこと。
- `repository` ドメイン（issue 973 で移行済み）に属するファイル・責務へ影響を及ぼさないこと。`code` と `repository` が共有する型・ユーティリティに変更が必要な場合は、`repository` 側の振る舞いと配置の規約準拠を崩さない範囲に限定すること。
- 既存の CI 品質チェック（フロントエンド lint／test／build、Rust fmt／clippy／test）が移行後も通過すること。

## Success Criteria

- `code` ドメインの各責務が新アーキテクチャの各層に規約準拠で配置され、依存方向の規約に違反していない。
- ドメイン層が外部リソースを直接参照していない。
- 移行対象責務のドメイン層・ユースケース層・ゲートウェイ層にテストが存在し、通過する。
- 旧構成の対応モジュール（`git/diff.rs`、`git/diff_tree.rs`、`git/branch_diff.rs`、`git/hunk.rs`、`git/lang.rs`、`git/stage.rs`、`file_mention.rs`、および `git/commands.rs` の `code` 責務部分）が除去され、同一責務の重複実装が残っていない。
- 本ドメインの Tauri コマンドが起動時 DI 配線（`register` 関数経由）に整合して登録されている。
- 移行前後で、デスクトップ UI・リモートクライアントから見た振る舞い（コマンド I/F・WebSocket メッセージ形式・diff／hunk／staging／メンション補完の操作結果）が等価である。
- Rust の fmt／clippy／test、およびフロントエンドの lint／test／build が通過する。

## Open Questions

- なし（スコープ境界および振る舞い保持の方針は確認済み。`code` ドメインの責務範囲は `docs/architecture/` のドメイン一覧および issue 974 で確定済み、`repository` ドメインは対象外、外部 I/F は完全保持の純粋移行とする）。
