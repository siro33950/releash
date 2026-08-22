# インフラストラクチャ層 規約

## 原則

- **外部世界の都合を、その形のまま扱う。** SQLite、git、CLI プロセス、HTTP、OS が要求する形式・手順・型を、その言葉のまま受け取り、そのまま渡す
- **変換しない。** 外部世界の都合を内側の言語へ写すのは gateway の責務である（[GATEWAY.md](./GATEWAY.md)）。変換を始めたら、それはもう infrastructure ではない
- **内側の層を知らない。** domain / usecase / adaptor に依存しない。domain 型を import せず、domain の trait を実装しない
- **業務判断を持たない。** 「マージ済みか」「削除してよいか」のような判定規則は domain に属する（[DOMAIN.md](./DOMAIN.md)）

## gateway との境界

判定はひとつ。**変換しているか。**

| 対象 | infrastructure（そのまま扱う） | gateway（変換する） |
|---|---|---|
| SQLite | 接続、DDL、トランザクション機構、保存形式の表現 | SQL（DML）、保存表現 ↔ domain record の codec |
| git | `git2` の呼び出しと `git2` の型 | `Branch` / `Commit` の構築、`git2::Error` → ドメインエラー |
| Terminal / Provider CLI | PTY、process group、raw byte I/O、checkpoint journal | Terminal Surface ownership、AgentSession lifecycle、Provider lifecycleへの変換 |
| 外向き通知 | 送信機構そのもの | ドメインの通知 → 送信形式への組み立て |

DDL は「保存形式がどういう形をしているか」の宣言であって、何も変換しない。対して SQL（DML）は、ドメインの値を行へ、行をドメインの値へ写す行為そのものであり、変換である。

**外部ライブラリ（`git2`、`reqwest` 等）を呼ぶこと自体は、どちらの層かの基準にならない。** gateway も infrastructure も呼ぶ。基準は変換しているかどうかだけである。

## Provider CLI の扱い

Provider CLIのconversation本文と画面は解析しない。AgentSession TUIはProvider CLIをTerminal SurfaceのPTYへ接続し、infrastructureはそのbyte streamをopaqueに扱う。Provider session identityとStop通知、transcript候補の探索は外部I/Oとして扱い、Agent固有wireをdomain eventへ変換するruntimeは持たない。

## ディレクトリの切り方

ドメイン名でディレクトリを切ることはあるが、それは「その外部世界を誰が使うか」の目印であって、そのドメインの語彙を持ってよいという意味ではない。

## 依存の向き

infrastructure は内側のどの層にも依存しない。`use crate::domain` / `use crate::usecase` / `use crate::adaptor` は書かない。

逆に adaptor/gateway は、変換の材料を得るために infrastructure に依存する。これは gateway が外部世界と内側を橋渡しする層だからであり、内向き依存の原則の例外ではない（[README.md](./README.md) 依存方向）。

## 何を置かないか

- ドメインの語彙への変換（gateway）
- フロントの語彙（DTO）への変換（gateway / QueryService 実装）
- 業務判断・検証・分類（domain）
- 複数集約をまたぐ手順の調停（usecase）

外部世界に触る処理がドメインの語彙を必要とし始めたら、その部分は gateway へ切り出す。infrastructure に残すのは、ドメインを知らなくても書ける部分だけである。
