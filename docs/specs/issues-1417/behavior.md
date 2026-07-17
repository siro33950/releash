# Behavior

関連: #1417 ([Agentチャット安定化] X1: 画像のみ送信時の空 text block 修正)

対象: Claude backend の turn 入力（user message wire）組立て。Codex backend の
`codex_user_input` と論理的に対称な条件で text block を生成する。

## Feature: 画像のみ送信時の空 text block を送らない

  ユーザーが本文なしで画像だけを送信したとき、Claude backend が生成する user
  message content に空の text block を含めない。これにより Codex backend と
  「画像のみ送信」の成否が一致する。

  Background:
    Given ユーザーが Claude backend の agent session でメッセージを送信する
    And 送信内容は prompt（本文文字列）と 0 個以上の画像から成る

  Rule: prompt が非空、または画像がないときだけ text block を含める

    Scenario: 空 prompt ＋ 画像あり（画像のみ送信）
      Given prompt が空文字列である
      And 1 個以上の画像が添付されている
      When user message content を組み立てる
      Then content に text block を含めない
      And content は添付画像に対応する image block のみで構成される

    Scenario: 非空 prompt ＋ 画像あり
      Given prompt が非空文字列である
      And 1 個以上の画像が添付されている
      When user message content を組み立てる
      Then content 先頭に prompt を内容とする text block を 1 つ含める
      And text block の後ろに添付画像に対応する image block を並べる

    Scenario: 非空 prompt ＋ 画像なし
      Given prompt が非空文字列である
      And 画像が添付されていない
      When user message content を組み立てる
      Then content は prompt を内容とする text block 1 つのみで構成される

    Scenario: 空 prompt ＋ 画像なし（境界・現状維持）
      Given prompt が空文字列である
      And 画像が添付されていない
      When user message content を組み立てる
      Then content は空文字列を内容とする text block 1 つのみで構成される
      # Codex `codex_user_input` と対称の挙動。送信可否の判断は frontend の既存
      # 挙動に委ね、backend では送信拒否しない。

  Rule: text block の生成条件は Codex backend と論理的に対称である

    Scenario Outline: Claude と Codex で text block の有無が一致する
      Given prompt が <prompt> である
      And 画像が <images> である
      When 両 backend が user message content を組み立てる
      Then どちらも text block を <text_block> する

      Examples:
        | prompt | images | text_block |
        | 空     | あり   | 含めない   |
        | 非空   | あり   | 含める     |
        | 非空   | なし   | 含める     |
        | 空     | なし   | 含める     |

## 仮定

- 空 text block の生成箇所は `claude/wire.rs` の `user_message` のみであり、
  `claude/session.rs` はこれを無加工で呼ぶだけである（監査 OB-7 の記述と一致）。
- 空 prompt ＋ 画像なしのケースで空 text block を残す挙動は許容される。Codex も
  同条件で空 text を送る対称挙動であり、frontend が本文・画像とも空の送信を通常
  許可しないため実運用上到達しにくい。対称性の維持を優先し、backend 側での送信
  拒否は本 Issue に含めない。
- 「Anthropic API が空 text block を拒否し得る」挙動は非対称解消の根拠として扱い、
  CLI のサニタイズ有無の外部検証は本変更の完了条件に含めない。
- image block の並び順は「text の後に image」を従来どおり維持する。

## Open Questions

なし
