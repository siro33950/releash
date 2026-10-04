## B-001: サーバの受け入れテストが Tauri のアプリなしで動く

GIVEN サーバの受け入れテスト（workflow control plane、agent session TUI、client API、provider lifecycle、workflow diagnostics、workflow delegate、state subscription）とサーバ側の lib テストがある
WHEN 開発者または CI がそれらを実行する
THEN Tauri のアプリを作らずに、サーバの入口（usecase、Connect の client API）を通して実行される

## B-002: desktop 無効の構成でサーバのテストが動く

GIVEN `desktop` feature を無効にした構成である
WHEN 開発者または CI がサーバの受け入れテストとサーバ側の lib テストをビルド・実行する
THEN ビルドが成功し、テストが実行される

## B-003: Tauri を参照するファイルが限られる

GIVEN この変更を取り込んだソースツリーである
WHEN Tauri クレートを参照するファイルを列挙する
THEN 列挙されるのは [03]（#1853）で Tauri シェルのクレートへ移すものだけである

## B-004: 既存の断定が保たれる

GIVEN 既存の受け入れテストと lib テストが断定している振る舞いがある
WHEN 変更後にそれらのテストを実行する
THEN 同じ振る舞いを同じ断定のまま検証し、成功する

## B-005: Tauri の invoke の拒否がシェルのテストで断定される

GIVEN client のコマンド（`current-branch`）と削除済みのコマンド（`get_terminal_stream_endpoint`）がある
WHEN シェルのテストがそれらを Tauri の invoke で呼ぶ
THEN 今と同じ断定のまま拒否が確認される
AND サーバのテストは Tauri の invoke を断定しない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
