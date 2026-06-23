# Behavior

Terminal / PTY ライフサイクルに cap・eviction・idle timeout を導入し、メモリ常駐量を境界づける振る舞いを定義する。

実装詳細（具体的なデータ構造・モジュール配置・経路）は含めず、外部から観測可能なビジネスルールとして記述する。cap 値・idle 時間などの具体値は requirements.md の仮定 A2/A6/A7 に従う暫定値であり、design.md でチューニング可能なパラメータとして扱う。

## Feature: Terminal / PTY ライフサイクル境界づけ

inactive な terminal を軽量状態へ退避し、cap・idle timeout を超えたリソースを Rust 側のポリシーで解放することで、メモリ常駐量とピークを境界づける。同時に active terminal の UX は維持する。

Background:
```gherkin
Given Releash のワークスペースが開かれている
And terminal のライフサイクルポリシー（cap・idle timeout・LRU 単位）は Rust 側で enforce される
And inactive terminal の復元元は Rust が保持する lightweight state である
```

---

## Rule R1: inactive terminal の xterm は unmount され、mounted 数が hidden 数に比例しない

```gherkin
Scenario: active な terminal だけが mount される
  Given 1 つの tab に 4 つの terminal pane があり、1 つだけが active である
  When ユーザーがその tab を表示している
  Then active な pane の xterm のみが DOM 上に mount されている
  And inactive な 3 つの pane の xterm は DOM から unmount されている

Scenario: hidden terminal を増やしても mounted 数が比例して増えない
  Given 複数の tab と pane に terminal が存在する
  When hidden（inactive）な terminal の数を増やす
  Then mounted xterm 数は hidden terminal 数に比例して増え続けない
```

---

## Rule R2: 再アクティブ化した terminal は直近スクロールバック・サイズ・カーソル状態を含めて復元される

```gherkin
Scenario: tab を切り替えて戻ると表示内容が復元される
  Given active な terminal にスクロールバックが蓄積されている
  When 別の tab に切り替え、その後元の tab に戻る
  Then その terminal の xterm が再 mount される
  And 直近スクロールバック・サイズ・カーソル状態が Rust の lightweight state から復元される
  And 復元結果は切り替え前と同等の UX で表示される

Scenario: 復元範囲は output buffer cap 相当の直近スクロールバックまで
  Given terminal の出力が output buffer cap を超えて蓄積されている
  When その terminal を unmount し再 mount する
  Then 復元されるスクロールバックは output buffer cap 相当の直近分までである
  And cap を超えた古い出力は復元対象に含まれない
```

仮定: フルスクロールバックの保持・復元は要求しない（A7）。

---

## Rule R3: terminal pane state は LRU / per-worktree cap / idle timeout で eviction される

```gherkin
Scenario: cap 到達時に最も古い idle terminal を LRU で eviction して新規を許可する
  Given ある worktree の terminal pane state が per-worktree cap に達している
  And cap 内に idle な terminal が少なくとも 1 つ存在する
  When 新しい terminal pane state を追加しようとする
  Then 最も古い idle terminal が LRU で自動 eviction される
  And 新しい terminal pane state の追加が許可される

Scenario: active terminal は eviction 対象から除外される
  Given per-worktree cap に達しており、active terminal が含まれている
  When cap 超過により eviction が発生する
  Then active terminal は eviction されない
  And eviction 対象は idle terminal の中から選ばれる

Scenario: idle timeout を超えた pane state が eviction される
  Given ある terminal pane state が idle timeout を超えて操作されていない
  When idle 判定が行われる
  Then その pane state は eviction 対象になり解放される
```

仮定: per-worktree cap 初期値は現行 MAX_PANES_PER_TAB×MAX_TABS 相当（A2/A5）。ポリシー値は design.md で定める。

---

## Rule R4: PTY は max panes（全体）・per-worktree cap・output buffer cap を Rust 側で enforce する

```gherkin
Scenario: per-worktree cap に達した状態で新規 PTY を要求する
  Given ある worktree の alive PTY 数が per-worktree cap に達している
  When その worktree で新しい terminal を開こうとする
  Then Rust 側のポリシーに従って eviction または上限制御が行われる
  And alive PTY 数が cap を超えて無制限に増えない

Scenario: 全体 max panes に達した状態で新規 PTY を要求する
  Given 全 worktree 合計の alive PTY 数が max panes に達している
  When 新しい terminal を開こうとする
  Then Rust 側のポリシーに従って上限制御が行われる
  And alive PTY 数が max panes を超えない

Scenario: output buffer は cap を超えて蓄積しない
  Given ある PTY が継続的に出力している
  When 出力量が output buffer cap を超える
  Then 古い出力が破棄され、buffer は cap を超えて増えない
```

仮定: output buffer cap 初期値は 64KB（A2）。

---

## Rule R5: alive PTY に idle timeout を導入し、idle 超過 PTY を解放できる

```gherkin
Scenario: idle timeout を超えた alive PTY が解放される
  Given alive な PTY が idle timeout を超えて入出力されていない
  When idle 判定が行われる
  Then その PTY は解放対象になる
  And 関連する terminal リソースが解放される

Scenario: idle timeout 内の PTY は解放されない
  Given alive な PTY が idle timeout 内に入出力されている
  When idle 判定が行われる
  Then その PTY は解放されない
```

仮定: alive PTY idle timeout 初期値は 5 分（A2）。

---

## Rule R6: PTY output buffer の正本 owner は一意であり、二重保持が解消されている

```gherkin
Scenario: output buffer の owner は Rust runtime 側に一本化される
  Given ある PTY が出力を生成している
  When その出力のスクロールバックを参照する
  Then output buffer の正本は Rust runtime 側に 1 つだけ存在する
  And WS bridge は独立した output buffer を保持しない

Scenario: スクロールバック取得は正本 owner から行われる
  Given inactive terminal の復元または remote subscriber への供給が必要である
  When スクロールバックを取得する
  Then 正本 owner（Rust runtime buffer）から取得される
```

仮定: 正本 owner は Rust runtime 側に一本化（A3）。

---

## Rule R7: remote subscriber 不在時は remote 用 buffer 蓄積・broadcast を最小化する

```gherkin
Scenario: subscriber 不在時は remote 用 buffer 蓄積と broadcast を最小化する
  Given WS remote subscriber が 1 つも接続していない
  When PTY が出力を生成する
  Then remote 用の buffer 蓄積と broadcast は最小化される

Scenario: subscriber 接続時に必要なスクロールバックが供給される
  Given subscriber が不在の状態で PTY が出力していた
  When remote subscriber が新たに接続する
  Then 接続時に必要なスクロールバックが正本 owner から供給される
```

---

## Rule R8: cap / eviction / idle timeout 導入によって active terminal の UX が劣化しない

```gherkin
Scenario: cap・eviction・idle timeout 導入後も active terminal の入出力が劣化しない
  Given cap・eviction・idle timeout が有効である
  And ユーザーが active terminal を操作している
  When inactive terminal の eviction や idle 解放がバックグラウンドで発生する
  Then active terminal の入出力・履歴・サイズ追随が劣化しない
  And active terminal は eviction や idle 解放の影響を受けない

Scenario: active terminal はサイズ変更に追随する
  Given active terminal が表示されている
  When terminal を表示する領域のサイズが変わる
  Then active terminal は新しいサイズに追随する
```

---

## 仮定（requirements.md より引き継ぎ）

- A2: cap / idle timeout の具体値は design.md でチューニング可能なパラメータとする。初期値は per-worktree cap = 現行 MAX_PANES_PER_TAB×MAX_TABS 相当、alive PTY idle timeout = 5 分、output buffer cap = 64KB。
- A3: PTY output buffer の正本 owner は Rust runtime 側に一本化し、WS bridge は独立バッファを廃止する。
- A4: lightweight state は Rust が保持する PTY output buffer ＋ pane のサイズ/メタ情報を基本とする。
- A5: 既存上限定数（MAX_TABS=8 / MAX_PANES_PER_TAB=4）は維持しつつ、cap enforcement の正本を Rust 側へ移す。
- A6: cap 到達時は LRU で最も古い idle terminal を自動 eviction し、active terminal は対象外とする。
- A7: 再 mount 時の復元範囲は output buffer cap 相当の直近スクロールバックまで。フルスクロールバックの保持・復元は要求しない。

## Open Questions

なし（requirements.md ですべて解消済み。本振る舞い定義でも追加の未確定事項なし）。
