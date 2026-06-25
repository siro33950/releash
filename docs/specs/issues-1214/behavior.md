# Behavior

要求 (`requirements.md`) を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。

本 Issue は性能・メモリ効率改善であり、ストリーミング表示や履歴復元の **見た目・UI 仕様の変更は含まない**。
振る舞い定義の中心は、Agent ストリーミング配信を「累積スナップショット」から「`seq` 付き delta（通常配信）＋
resync 時 snapshot」へ移行した結果として観測できる以下の特性である:

- 通常配信が `seq` 付き delta で行われ、1 配信単位の payload が応答全体長に比例して増えない（R1）。
- 受信側が `seq` で順序づけ・欠落検知・重複排除でき、同一 `seq` の重複適用が冪等である（R2）。
- reconnect / resync 時に限り snapshot を送り、`since_seq` から欠落分を一意に復元できる（R3）。
- frontend 配信と WS 配信の双方が delta + resync を扱え、欠落検知時は resync で最終状態へ収束する（R4）。
- delta 化後も #970 の表示即時性が維持される（R5）。
- `parts_to_legacy` が compatibility 出力に限定され、配信・保存の正典に使われない（R6）。
- delta protocol が `streaming_parts` の累積常駐を再導入せず、#1194 の解放と矛盾しない（R7）。
- delta を順に適用した結果・resync 後の状態が、変更前のスナップショット方式と外部観測範囲で一致する（R8）。
- delta 生成・順序づけ・適用・重複排除・resync 復元のロジックが Rust 側に置かれる（R9）。
- 既存テスト・lint が green で、新規ロジックに正常系・エッジ系テストが追加される（R10）。

## 仮定

- A1: 「外部から観測可能な振る舞い」とは、受信側 message store に最終的に適用されるメッセージ内容（parts）、
  表示の即時性、resync 後の収束状態を指す。内部の payload 形状・送信回数・clone 回数・メモリ常駐量は
  これに含めない（requirements A5 に対応）。
- A2: 通常配信の Tauri event 名・WS メッセージ型の新設／改廃・互換維持方針、`seq` の最終粒度・採番起点、
  resync snapshot の復元源（runtime bounded buffer か永続化済み message / paging 経由か）、delta の具体表現は
  すべて `design.md` で確定する。本 behavior は「通常 = delta、resync = snapshot」という配信の意味論と、
  外部観測できる収束・即時性・冪等性のみを定義する（requirements A2/A3/A6/A7 に対応）。
- A3: `seq` は `(session_id, message_id)` 単位で単調増加し、delta はその message のストリーミング進行に沿って
  順序づけられるものと仮定する。
- A4: 本 Issue は #1194（turn 完了時の `streaming_parts` 解放）の成果を前提とし、delta protocol がその解放と
  矛盾しないことの保証に範囲を限定する。解放実装そのものは #1194 が owner である。
- A5: WS 経路はモバイル向けフロント remote クライアントが削除済みのため、protocol / サーバー側が delta + resync を
  運べる状態にすることまでを範囲とする。受信クライアントを介した E2E 検証は行わず、WS 側の検証は Rust 側
  protocol / 配信・適用ロジックの単体・結合テストで担保する。
- A6: #970 表示即時性の基準点は「配信受信直後に UI へ反映される（33ms coalescing 基準）」であり、行途中での
  停止やストリーミング完了時の一括表示が再発しないことを指す。

Feature: Agent ストリーミングの delta + resync 配信への移行

  Background:
    Given Agent セッションでストリーミング応答を配信する経路（frontend 向け配信および WS 配信）が利用可能である
    And アプリは本 Issue（#1214）の delta + resync 配信を適用済みのビルドで動作している
    And 受信側には delta を適用する message store が存在する

  Rule: ストリーミング中の通常配信は累積スナップショットではなく seq 付き delta で行われる

    Scenario: 応答が長くなっても 1 配信単位の payload が応答全体長に比例しない
      Given あるターンのストリーミング応答が進行し、parts が累積している
      When 通常配信で新たな増分が送られる
      Then その配信単位の payload はそのフレームで新たに生じた増分に概ね比例する
      And それまでに蓄積した応答全体長に比例して payload や処理量が増加しない

    Scenario: 通常配信中は snapshot を送らない
      Given ストリーミングが通常進行中である（reconnect / resync が発生していない）
      When 通常配信が行われる
      Then 送られるのは seq 付き delta であり、累積 parts スナップショットは送られない

  Rule: 受信側は seq により順序づけ・欠落検知・重複排除でき、適用が冪等である

    Scenario: 単調増加する seq により delta が一意に順序づけられる
      Given 同一 (session_id, message_id) に対する複数の delta が配信される
      When 受信側が delta を受け取る
      Then 各 delta は単調増加する seq を持ち、受信側はその seq で適用順序を決定できる

    Scenario: 同一 seq の delta を重複受信しても適用結果が冪等である
      Given ある seq の delta を既に message store へ適用済みである
      When 同一 seq の delta を再度受信する
      Then 重複適用しても message store の状態は変化せず、最終メッセージ内容は一意である

    Scenario: 順序が入れ替わって到着した delta でも最終状態が一意に収束する
      Given 複数の delta が本来の seq 順とは異なる順序で到着する
      When 受信側が seq に従って delta を適用する
      Then 適用後の最終メッセージ内容は、正しい seq 順で適用した場合と一致する

    Scenario: seq の欠落を検知できる
      Given 受信済み delta の seq に連続しない欠落が生じている
      When 受信側が次の delta を受け取る
      Then 受信側は seq の不連続から欠落の発生を検知できる

  Rule: reconnect / resync 時に限り snapshot が送られ、since_seq から欠落分を復元できる

    Scenario: reconnect 後に since_seq を起点として欠落 delta 範囲を復元する
      Given 受信側が seq=N まで適用済みの状態で配信が切断される
      And 切断中に seq=N+1 以降の delta が生成されている
      When 受信側が再接続し、since_seq=N を起点に resync を要求する
      Then resync snapshot により欠落していた delta 範囲が一意に復元される
      And 復元後の最終メッセージ内容はスナップショット方式での最終状態と一致する

    Scenario: resync 完了後は通常配信（delta）へ戻る
      Given reconnect により resync snapshot を適用し最新状態へ復元している
      When ストリーミングが継続して新たな増分を生じる
      Then 以降の配信は通常配信（seq 付き delta）で行われ、snapshot は送られない

  Rule: frontend 配信と WS 配信の双方が delta + resync を扱い、最終状態へ収束する

    Scenario: frontend 配信で欠落が生じても resync 後に正規の最終状態へ収束する
      Given frontend 向け配信で delta の欠落・重複・再接続が発生する
      When 受信側が delta 適用と resync 復元を行う
      Then 表示内容は破綻せず、最終的に正規の最終メッセージ内容へ収束する

    Scenario: WS protocol / サーバー側が delta + resync を運べる
      Given WS 配信経路が利用可能である（受信クライアントは不在）
      When ストリーミング配信と resync 要求を WS protocol で表現する
      Then protocol / サーバー側は通常配信を delta、resync を snapshot として運べる
      And この経路は Rust 側 protocol / 配信・適用ロジックの単体・結合テストで検証される

  Rule: delta 化後も #970 の表示即時性が維持される

    Scenario: 配信受信直後に UI へ反映される
      Given delta + resync 配信を適用済みのビルドで動作している
      When ストリーミングの delta が受信される
      Then 受信直後に当該増分が UI へ反映される
      And 行途中での停止やストリーミング完了時の一括表示は再発しない

  Rule: parts_to_legacy は compatibility 出力に限定され、配信・保存の正典に使われない

    Scenario: 配信・保存の正典が legacy 表現に依存しない
      Given delta / snapshot による配信・保存が行われている
      When 配信される delta と保存される正典メッセージを観測する
      Then いずれも `parts_to_legacy` による legacy 表現に依存しない
      And `parts_to_legacy` は互換目的の legacy 表現生成（compatibility 出力）にのみ用いられる

  Rule: delta protocol は streaming_parts の累積常駐を再導入せず #1194 の解放と矛盾しない

    Scenario: resync 用 snapshot の生成が完了ターン分の常駐を要求しない
      Given #1194 によりターン完了後にアイドル session の完了ターン分 parts が解放される
      When delta protocol が resync 用 snapshot を生成する
      Then その生成経路は完了ターン分の累積 parts を session に常駐させ続けることを要求しない
      And delta 化による累積常駐の再導入は発生しない

  Rule: delta を順に適用した結果が変更前のスナップショット方式と外部観測範囲で一致する

    Scenario: delta を順次適用した最終メッセージが変更前と一致する
      Given 変更前ビルド（スナップショット方式）での最終メッセージ内容が観測されている
      When 同一のターンを変更後ビルドで実行し、delta を seq 順に適用する
      Then 適用後の最終メッセージ内容は変更前と一致する

    Scenario: resync snapshot 適用後の状態が変更前と一致する
      Given 変更前ビルドでの最終メッセージ内容が観測されている
      When 切断・再接続を含むシナリオを変更後ビルドで実行し、resync snapshot を適用する
      Then resync 適用後の最終メッセージ内容は変更前と一致する

  Rule: delta 関連ロジックは Rust 側に置かれる

    Scenario: delta 生成・順序づけ・適用・重複排除・resync 復元が Rust 側に存在する
      Given 本 Issue で delta + resync の配信・適用ロジックを実装している
      When delta の生成・順序づけ・適用・重複排除・resync 復元のロジックの所在を確認する
      Then これらのロジックは Rust（read model / shared）側に置かれている
      And frontend には表示用フォーマットのみが残り、上記ロジックは持ち込まれない

  Rule: 既存テスト・lint が green であり、新規ロジックにテストが追加される

    Scenario: 既存テスト・lint が green である
      Given 本 Issue の変更を適用済みである
      When 既存のテスト（cargo test / pnpm test）と lint（cargo clippy -D warnings / pnpm lint）を実行する
      Then すべて成功する

    Scenario: 新規ロジックに delta 適用・欠落／重複・reconnect のテストが追加されている
      Given 本 Issue で delta 生成・適用・resync 復元のロジックを追加・変更している
      When 追加されたテストを実行する
      Then delta 適用の正常系、seq の欠落・重複・順序入れ替えのエッジ系、reconnect / resync 復元が検証される

## Open Questions

なし。
