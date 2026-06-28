# Behavior

このリファクタリング ISSUE は、WebSocket server 本体・broadcast bridge・status event 変換を
clean architecture の層へ移設するものであり、対外契約と transport 振る舞いを完全に維持する。
したがって振る舞い定義は「移設後も観測可能な振る舞いが変わらないこと」と
「構造上の受け入れ条件が満たされること」を中心に記述する。

Feature: transport 境界コードの層配置と振る舞い維持

  WebSocket server / bridge / status event 変換を ISSUE が示す層へ移設しても、
  WebSocket の対外契約（message 名・payload JSON shape）と既存の transport 振る舞い
  （auth / reconnect / resync / PTY replay / stream buffering / push notification）は変わらない。

  Background:
    Given protocol 型・helper は adaptor/protocol/ 配下に集約済みである（#1130 完了）
    And WebSocket server / broadcaster / status event 変換が移設対象である

  Rule: WebSocket の対外契約は変更されない

    Scenario: message 名が維持される
      Given クライアントが移設前と同じ WebSocket リクエストを送る
      When backend が応答する
      Then 応答に含まれる WebSocket message 名は移設前と同一である

    Scenario: payload の JSON shape が維持される
      Given backend が push / 応答 payload を serialize する
      When その serialize 結果をクライアントが受け取る
      Then JSON shape（フィールド構成・型）は移設前と互換である

    Scenario: frontend は変更を必要としない
      Given wire contract が不変である
      When 移設が完了する
      Then frontend（TypeScript）側の型定義・通信コードは変更されない

  Rule: 既存の transport 振る舞いが維持される

    Scenario: HMAC auth の成功と失敗
      Given クライアントが WebSocket 接続を開始する
      When HMAC 認証情報が妥当である
      Then 接続が確立される

    Scenario Outline: 異常な認証・入力は移設前と同じく拒否される
      Given クライアントが WebSocket へ接続を試みる
      When <condition>
      Then backend は移設前と同じく接続/メッセージを拒否する

      Examples:
        | condition                   |
        | HMAC 認証情報が不正である     |
        | 不正な形式の message を送る   |
        | rate limit を超過する         |

    Scenario: reconnect と resync
      Given クライアントが切断後に再接続する
      When resync を要求する
      Then 移設前と同じ resync stream が返る

    Scenario: PTY replay
      Given クライアントが再接続後に PTY 出力の replay を受け取る
      When backend が buffered 出力を送出する
      Then 移設前と同じ replay 内容が再生される

    Scenario: agent stream の buffering と push notification
      Given agent stream の delta/snapshot が backend に蓄積される
      When backend がクライアントへ push する
      Then queue 上限・byte 上限・slow consumer 向け snapshot 折りたたみの振る舞いが移設前と同一である
      And push notification が移設前と同じく送出される

    Scenario: broadcaster の drop と buffer limit
      Given slow consumer により buffer が上限へ達する
      When backend が delta を送出し続ける
      Then 移設前と同じ drop / snapshot 折りたたみ挙動になる

  Rule: ルート直下の transport module が層へ解消される

    Scenario: ルート直下 module の削除
      When 移設が完了する
      Then src-tauri/src/ws_server/ ディレクトリが存在しない
      And src-tauri/src/ws_bridge.rs が存在しない
      And src-tauri/src/agent_status_events.rs が削除されるか adaptor/presenter/gateway 配下へ移動している
      And lib.rs に mod ws_server / mod ws_bridge / mod agent_status_events 宣言が残っていない

    Scenario Outline: 各責務が対応する層へ配置される
      Given <code> が移設対象である
      When 移設が完了する
      Then それは <layer> 配下に存在する

      Examples:
        | code                                       | layer                          |
        | WebSocket routing / request handler entry  | adaptor/controller/handler/    |
        | HMAC auth / rate limit / HTTP upgrade / TLS・server plumbing の純粋 transport concern | infrastructure/middleware/ |
        | outbound broadcaster / push・sync notifier  | adaptor/gateway/               |
        | usecase/domain state → push payload 変換    | adaptor/presenter/             |

  Rule: 依存方向が維持される

    Scenario: 層の依存方向を逆転させない
      Given 移設後の module 構成である
      When 依存関係を検査する
      Then domain / usecase module は adaptor / infrastructure を import していない
      And 依存方向は adaptor / infrastructure → usecase → domain を保つ

  Rule: 既存 test が移設先で維持される

    Scenario: WebSocket 関連 test が移設先で通過する
      Given 移設前に auth success/failure・invalid message・resync stream・buffered replay・broadcaster drop/buffer limit の test が存在した
      When test を移設先 module 側で実行する
      Then それらの test が存在し、通過する

    Scenario: 既存 test が失われない
      Given 移設前に rate limit / HTTP upgrade / routing 等の test が存在した
      When 移設が完了する
      Then それらの test は失われていない
      And test の期待値・アサーション内容は変更されていない

  Rule: 品質ゲートを満たす

    Scenario Outline: 品質チェックが通る
      When <command> を実行する
      Then 成功する

      Examples:
        | command                       |
        | cargo fmt --check             |
        | cargo clippy -- -D warnings   |
        | cargo test                    |

## 仮定

- 本 ISSUE は「コードの物理的な移動と import 経路の更新」を主とし、wire 上の
  シリアライズ結果（message 名・payload JSON shape）と既存の transport 振る舞い
  （auth / reconnect / resync / PTY replay / stream buffering / push notification）を完全に維持する。
- broadcaster の queue 上限（`STREAM_DELTA_QUEUE_LIMIT` = 1024）・byte 上限
  （`STREAM_DELTA_QUEUE_BYTE_LIMIT` = 512KiB）・snapshot 折りたたみアルゴリズムは
  振る舞い維持対象であり、定数・ロジックを変更しない。
- `ws_server/` 配下各ファイルの分割粒度、`WsServerState` / `WsServerHandle` の配置、
  起動・bind・shutdown plumbing の所属、`agent_status_events.rs` の最終配置
  （presenter への純変換切り出し + wiring 残置 等）は本振る舞い定義では確定させず、design.md で決定する。
  本振る舞い定義は「対応する層への配置」と「振る舞い維持」のみを要件とする。
- `infrastructure/middleware/` ディレクトリは現状存在しないため本 ISSUE で新規作成する。

## Open Questions

なし。
