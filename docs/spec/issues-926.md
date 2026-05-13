## 要求

**種別**: バグ修正（リファクタリング込み）

**ゴール**:
- ワークフローパネル（`AgentChatPanel/WorkflowPanel` 配下）をはじめとするスクロール領域で、OSデフォルトの太いスクロールバーではなく、Radix `ScrollArea` 風の細いカスタムスクロールバーが表示される状態にする。
- カスタムスクロールバー用コンポーネント `src/components/ui/scroll-area.tsx` を完全削除し、現在の全使用箇所を素の `div + overflow-*` に置換する。
- `src/index.css` のグローバルスクロールバースタイル（`*::-webkit-scrollbar` および `scrollbar-width` / `scrollbar-color`）を、Radix `ScrollArea` の `ScrollBar`／`ScrollAreaThumb` の見た目（細い幅・薄いthumb色・角丸）に揃えるよう調整する。
- `.scrollbar-thin` ユーティリティクラスとその専用CSS定義（`src/index.css` 内）も併せて削除し、`.scrollbar-thin` 適用箇所も §1 のグローバル定義に統一する。
- 結果として、専用コンポーネント・専用クラスを差し込まなくても、`overflow-*` を付与しただけで対象スクロール領域が統一されたスクロールバー見た目になる（グローバルCSSのみで一意化）。

**背景**:
- Issue #926 で「ワークフローパネルやOutput欄などでデフォルトのスクロールバーが表示されている」と報告されている。
- 現状の `index.css` には `*::-webkit-scrollbar` でグローバルスクロールバースタイルが定義されているが、対象パネルではOSデフォルト相当の太いスクロールバーが見えてしまい、UIの一貫性が崩れている。
- Radix ベースの `ScrollArea` コンポーネント（`src/components/ui/scroll-area.tsx`）を使った箇所だけが統一スタイルになるという「コンポーネントを指定しないと揃わない」設計自体が不具合の温床であり、保守性も悪い。
- グローバルCSSだけでスタイルが揃うようにし、専用コンポーネントへの依存を解消することで、表示の不一致と将来の同種バグ発生の両方を防ぐ。

**対象範囲**:

本件の対象範囲は次の 5 区分で扱う。

1. **主たる視認確認対象**（Issue #926 の表示崩れ報告箇所）:
   - `src/components/panels/AgentChatPanel/WorkflowPanel/` 配下の overflow-* 領域全般（`WorkflowPanel.tsx`、`WorkflowTrace.tsx` を含む各メイン/詳細ペイン、trace/output 表示領域）。
2. **影響対象**（合計12ファイル: 削除2 + 実装置換9 + テスト更新1）:
   - **削除（2ファイル）**:
     - `src/components/ui/scroll-area.tsx`
     - `src/components/ui/scroll-area.test.tsx`
   - **実装置換（9ファイル: `ScrollArea` import を削除し、素の `div + overflow-*` に置換する）**:
     - `src/components/panels/AgentChatPanel/AgentChatPanel.tsx`
     - `src/components/panels/AgentChatPanel/StreamMessage.tsx`
     - `src/components/workspace/CreateWorktreeModal.tsx`
     - `src/components/panels/ShikiDiffViewer.tsx`
     - `src/components/panels/RemotePanel.tsx`
     - `src/components/panels/MarkdownDiffViewer.tsx`
     - `src/components/panels/ImageDiffViewer.tsx`
     - `src/components/panels/DiffFileTree.tsx`
     - `src/components/panels/DiffCommentList.tsx`
   - **テスト更新（1ファイル: 置換後のDOM構造に合わせて `data-slot` セレクタ等のアサーションを更新する。本ファイルは `ScrollArea` を import していない）**:
     - `src/components/panels/AgentChatPanel/StreamMessage.test.tsx`
3. **追加CSS調整対象**（`ScrollArea` 利用ではないが、§1 と整合させるための個別CSS削除/更新対象）:
   - `src/remote/styles/terminal.css`: `.xterm .xterm-viewport::-webkit-scrollbar*` の見た目指定（`width` / 色 / `border-radius` / `border` / `background-clip` 等）は削除し、`overflow-y` 等の挙動指定のみ残す。
   - `src/index.css`: `.scrollbar-thin` ユーティリティ専用ブロック（`.scrollbar-thin` / `.scrollbar-thin::-webkit-scrollbar` / `.scrollbar-thin::-webkit-scrollbar-track` / `.scrollbar-thin::-webkit-scrollbar-thumb` / `.scrollbar-thin::-webkit-scrollbar-thumb:hover`）を削除する。
4. **スタイル適用対象**（グローバルCSSが当たる範囲）:
   - `src/index.css` を読み込む全DOMの overflow-* 要素のうち、明示的にスクロールバーを隠していないもの。
   - `.scrollbar-thin` を従来適用していた箇所（`src/components/panels/MarkdownDiffViewer.tsx` L102, L235）も §1 の視覚検証対象に含め、本件で §1 のグローバル定義に統合する。
5. **対象外（既存挙動を維持する例外。本件で変更しない）**:
   - `src/components/panels/AgentChatPanel/AgentChatPanel.tsx` L537-540 の TabsList（`[&::-webkit-scrollbar]:hidden [scrollbar-width:none]` を維持）。
   - `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowPanel.tsx` L124-126 の TabsList（同上）。

なお、`src/remote/styles/terminal.css` の `.xterm .xterm-viewport` 領域は本件のスタイル適用対象に含める（§1 のグローバル定義が適用される）。このため、同ファイルに存在する `.xterm .xterm-viewport::-webkit-scrollbar*` の個別の見た目指定（`width: 14px` / `rgba(...)` thumb 色 / `border-radius: 7px` / `border: 3px solid transparent` / `background-clip: padding-box` 等、§1 と矛盾する値）は削除し、見た目は §1 のグローバル定義に委ねる。挙動指定（`overflow-y: auto`、`-webkit-overflow-scrolling: touch`、`overscroll-behavior: contain` 等）のみ残す。

スタイル定義の編集対象は次の 2 箇所に限定する。
- `src/index.css`: §1 のグローバルスクロールバー関連ブロック（§1 の具体値の唯一の定義箇所）、および `.scrollbar-thin` 専用ブロックの削除。
- `src/remote/styles/terminal.css`: `.xterm .xterm-viewport` 系スクロールバー個別の見た目指定の削除（挙動指定は残す）。

**バグ詳細**:
- 現在の挙動: ワークフローパネル等の `overflow-auto` を持つ要素に、OSデフォルトの太いスクロールバーが表示されている。
- 期待する挙動: 同領域で、Radix `ScrollArea` の `ScrollBar` 相当（後述の§1具体値）の統一カスタムスクロールバーが表示される。専用コンポーネントを使わずとも、グローバルCSSのみで対象パネルに同じ見た目が適用される。
- 再現手順: ワークフローパネルを開き、内容がスクロール可能な状態にする（参考: Issue #926 添付スクリーンショット）。

## 振る舞い定義

### §1. スクロールバー見た目の判定基準値

下記Gherkinの Then 句および合格条件は、以下の具体値を基準とする（出典: `src/index.css` のグローバル `*::-webkit-scrollbar` 定義）。なお、既存 `src/components/ui/scroll-area.tsx` の `ScrollBar` が使用する `bg-border` 色は §1 の oklch 値とは別物であり、本件で旧 ScrollArea 使用箇所の thumb 色が変化するのは意図された統一である。

- WebKit `*::-webkit-scrollbar`: `width: 10px; height: 10px;`
- WebKit `*::-webkit-scrollbar-thumb` の `border-radius: 9999px`（rounded-full 相当。現状未指定のため本件で追加する）
- thumb色（dark）: 通常 `oklch(0.56 0 0 / 0.4)` / hover `oklch(0.49 0 0 / 0.7)`
- thumb色（light）: 通常 `oklch(0.45 0 0 / 0.55)` / hover `oklch(0.30 0 0 / 0.75)`
- Firefox系: `scrollbar-width: thin`、`scrollbar-color: <上記thumb色> transparent`

「細い幅」「薄いthumb色」「角丸」「同一の見た目」「OSデフォルトの太い」といった定性的表現は、本節の具体値を指すものとする。

### §2. シナリオ

```gherkin
Feature: スクロール可能領域における統一スクロールバー表示
  対象スクロール領域で、専用ラッパーコンポーネントを介さずとも
  §1 で定義した具体値（width/height 10px・border-radius 9999px・指定oklch色）の
  統一されたスクロールバー見た目が適用される。

  Rule: 統一スタイルのスクロールバー見た目
    Scenario: ワークフローパネルで統一スタイルのスクロールバーが表示される
      Given ワークフローパネルの内容が表示領域を超えている
      And テーマが dark である
      When ユーザーがワークフローパネルを閲覧する
      Then ::-webkit-scrollbar の width/height は 10px である
      And ::-webkit-scrollbar-thumb の border-radius は 9999px である
      And ::-webkit-scrollbar-thumb の背景色は oklch(0.56 0 0 / 0.4) である
      And scrollbar-width は thin、scrollbar-color の thumb 部は oklch(0.56 0 0 / 0.4) である

    Scenario: ワークフローパネルでOSデフォルトの太いスクロールバーは表示されない
      Given ワークフローパネルの内容が表示領域を超えている
      When ユーザーがワークフローパネルを閲覧する
      Then OS既定幅（10pxを超える幅）のスクロールバーは表示されない
      And 対象要素の computed style において scrollbar-width は thin である
      And ::-webkit-scrollbar 擬似要素の width および height は 10px である

    Scenario: dark テーマで thumb に hover した時、hover 色が適用される
      Given 対象スクロール領域がスクロール可能で、テーマが dark である
      When ユーザーが ::-webkit-scrollbar-thumb 上にマウスポインタを乗せる
      Then ::-webkit-scrollbar-thumb:hover の背景色は oklch(0.49 0 0 / 0.7) である

    Scenario: light テーマで thumb に hover した時、hover 色が適用される
      Given 対象スクロール領域がスクロール可能で、テーマが light である
      When ユーザーが ::-webkit-scrollbar-thumb 上にマウスポインタを乗せる
      Then ::-webkit-scrollbar-thumb:hover の背景色は oklch(0.30 0 0 / 0.75) である

  Rule: 専用コンポーネント非依存
    Scenario: 素の div と overflow-* のみで統一スタイルが適用される
      Given スクロール可能な領域が素の div と overflow-* ユーティリティのみで実装されている
      And その領域の内容が表示領域を超えている
      And テーマが dark である
      When ユーザーがその領域を閲覧する
      Then §1 と同じ width/height 10px、border-radius 9999px のスクロールバーが表示される
      And ::-webkit-scrollbar-thumb の背景色は oklch(0.56 0 0 / 0.4) である

    Scenario: 旧 ScrollArea 置換箇所すべてで §1 の見た目が適用される
      Given 「対象範囲」§2 の実装置換9ファイルすべてで ScrollArea 使用箇所が素の div と overflow-* に置換されている
      And 各置換箇所の内容が表示領域を超えている
      And テーマが dark である
      When ユーザーが各領域を閲覧する
      Then 各置換箇所で §1 の width/height 10px、border-radius 9999px のスクロールバーが表示される
      And 各置換箇所の ::-webkit-scrollbar-thumb の背景色は oklch(0.56 0 0 / 0.4) である

    Scenario: MarkdownDiffViewer の split / preview 領域でも §1 の見た目が適用される
      Given `src/components/panels/MarkdownDiffViewer.tsx` L102 の split コンテナおよび L235 の preview 領域から `scrollbar-thin` クラス指定が削除されている
      And `src/index.css` から `.scrollbar-thin` 専用CSS定義ブロックが削除されている
      And 各領域の内容が表示領域を超えている
      And テーマが dark である
      When ユーザーが split / preview 各領域を閲覧する
      Then 各領域で §1 の width/height 10px、border-radius 9999px のスクロールバーが表示される
      And 各領域の ::-webkit-scrollbar-thumb の背景色は oklch(0.56 0 0 / 0.4) である

    Scenario: xterm viewport でも §1 の見た目が適用される
      Given `.xterm .xterm-viewport` の内容が表示領域を超えてスクロール可能である
      And テーマが dark である
      When ユーザーが xterm viewport を閲覧する
      Then `.xterm .xterm-viewport::-webkit-scrollbar` の width/height は 10px である
      And `.xterm .xterm-viewport::-webkit-scrollbar-thumb` の border-radius は 9999px である
      And `.xterm .xterm-viewport::-webkit-scrollbar-thumb` の背景色は oklch(0.56 0 0 / 0.4) である
      And `src/remote/styles/terminal.css` 内に §1 と矛盾する `.xterm .xterm-viewport::-webkit-scrollbar*` 個別指定は残っていない

    Scenario: ScrollArea 置換後も AgentChatPanel のチャット履歴で新規メッセージ追加時は強制的に最下部へスクロールされる
      Given AgentChatPanel のチャット履歴がスクロール可能である
      When 新しい Agent メッセージが追加される（メッセージ件数が増える）
      Then スクロール位置はユーザーの現在位置に関わらず最下部へ強制的にスクロールされる（既存挙動を維持）

    Scenario: ScrollArea 置換後も AgentChatPanel のチャット履歴でコンテンツ更新時のみ near-bottom 判定で追従する
      Given AgentChatPanel のチャット履歴がスクロール可能である
      And メッセージ件数は変化していない（既存メッセージのコンテンツのみが更新されている）
      When 既存メッセージのコンテンツが更新される
      Then ユーザーが最下部付近にいる場合はスクロール位置が最下部に追従する
      And ユーザーが最下部から離れた位置にいる場合は追従しない（既存挙動を維持）

  Rule: 境界条件
    Scenario: 内容が表示領域内に収まる場合はスクロールバーが表示されない
      Given 対象スクロール領域の内容が表示領域内に収まっている
      When ユーザーがその領域を閲覧する
      Then スクロールバーは表示されない
      And `overflow-*` を適用した要素自身に対して、本件の修正範囲内では `padding` / `border` / `scrollbar-gutter` を追加していない

    Scenario: 横方向のみオーバーフローした領域では横スクロールバーのみが表示される
      Given 横方向にだけ内容が表示領域を超える領域（テーブル/画像/diff 等）が描画されている
      And テーマが dark である
      When ユーザーがその領域を閲覧する
      Then 横方向に §1 と同じ height 10px、border-radius 9999px のスクロールバーが表示される
      And ::-webkit-scrollbar-thumb の背景色は oklch(0.56 0 0 / 0.4) である
      And 縦方向のスクロールバーは表示されない

    Scenario: 旧 ScrollArea 置換箇所の横方向オーバーフローでも §1 の見た目が適用される
      Given StreamMessage / ShikiDiffViewer / ImageDiffViewer のいずれにおいても、該当する各箇所すべてが置換後の素の div + overflow-* で描画されている
      And その各領域が横方向にだけ表示領域を超えている
      And テーマが dark である
      When ユーザーが該当する各領域を閲覧する
      Then 該当する各箇所すべてで、横方向に §1 と同じ height 10px、border-radius 9999px のスクロールバーが表示される
      And 該当する各箇所すべてで、::-webkit-scrollbar-thumb の背景色は oklch(0.56 0 0 / 0.4) である
      And 該当する各箇所すべてで、縦方向のスクロールバーは追加で表示されない

  Rule: テーマ切替時の状態遷移
    Scenario: dark から light へテーマ切替してもスクロールバー寸法・角丸は不変で色のみ切り替わる
      Given 対象スクロール領域がスクロール可能で、テーマが dark である
      When テーマを light に切り替える
      Then ::-webkit-scrollbar の width/height は 10px のままである
      And ::-webkit-scrollbar-thumb の border-radius は 9999px のままである
      And ::-webkit-scrollbar-thumb の背景色は oklch(0.45 0 0 / 0.55) に切り替わる
      And scrollbar-color の thumb 部は oklch(0.45 0 0 / 0.55) に切り替わる

    Scenario: light から dark へテーマ切替してもスクロールバー寸法・角丸は不変で色のみ切り替わる
      Given 対象スクロール領域がスクロール可能で、テーマが light である
      When テーマを dark に切り替える
      Then ::-webkit-scrollbar の width/height は 10px のままである
      And ::-webkit-scrollbar-thumb の border-radius は 9999px のままである
      And ::-webkit-scrollbar-thumb の背景色は oklch(0.56 0 0 / 0.4) に切り替わる

  Rule: 対象外領域の既存挙動維持
    Scenario: AgentChatPanel/WorkflowPanel の TabsList ではスクロールバーが引き続き非表示である
      Given TabsList がタブ横スクロールバー非表示指定（`[&::-webkit-scrollbar]:hidden [scrollbar-width:none]`）を持っている
      When タブ一覧の幅が表示領域を超える
      Then スクロールバーは表示されない（既存挙動を維持）
```

## アーキテクチャ概要

本件はフロントエンド純粋なUI/CSSの修正であり、ロジックを伴わない。よって「Rust-first ロジック配置」原則の対象外（CSSスタイル定義はフロントエンドが唯一の責務領域）であり、Tauriコマンドの追加・変更は発生しない。

### 責務配置
- **グローバルCSS（`src/index.css` の `@layer base` 内スクロールバー定義）**: アプリ全体のスクロール可能領域に対する統一スクロールバー見た目（§1の具体値）の唯一の定義箇所。担当しないこと: 個別パネル固有のレイアウト・サイズ指定。
- **`src/remote/styles/terminal.css`**: xterm の挙動上必要なスクロール関連プロパティ（`overflow-y: auto`、`-webkit-overflow-scrolling: touch`、`overscroll-behavior: contain`）のみを担う。スクロールバーの見た目（幅・色・角丸）に関する個別指定は持たず、§1 のグローバル定義に委ねる。
- **各パネルコンポーネント（`src/components/panels/...` 等の実装置換9ファイル）**: スクロール領域は素の `div` と Tailwind の `overflow-*` ユーティリティで宣言するのみ。担当しないこと: スクロールバー見た目の指定、専用スクロールラッパーコンポーネントの導入、`.scrollbar-thin` 等の専用クラス付与。
- **削除対象（`src/components/ui/scroll-area.tsx` および同テスト、`src/index.css` の `.scrollbar-thin` 専用ブロック）**: 本件完了後は存在しない。担当領域は全てグローバルCSSへ移管。
- **対象外領域（TabsList の非表示指定）**: 既存定義を維持する。グローバル定義が上書きしないよう、既存の非表示指定を尊重する。

### データ/通信フロー
- **スクロールバー表示**: ブラウザのレイアウトエンジン（`overflow-*` 検出）→ グローバルCSSの `*::-webkit-scrollbar` / `scrollbar-*` 適用 → §1の具体値で描画。Reactレンダー時にコンポーネント介在なし。
- **テーマ切替時のスクロールバー色変化**: `:root.light` クラス切替 → グローバルCSSの light オーバーライドが適用 → 自動再描画（寸法・角丸は不変、色のみ切り替え）。

### 状態Owner
- **スクロール位置**: ブラウザのDOM（各 `overflow-*` 要素自身）。Reactステートでは保持しない。
- **スクロールバー見た目（色・幅・角丸）**: グローバルCSS（`src/index.css`）のみ。コンポーネント側のpropsやstateでは制御しない。
- **テーマ（dark/light）**: 既存の `:root.light` クラス管理（本件で変更なし）。

### 境界
- **CSS層 ↔ コンポーネント層**: スクロールバー見た目はCSSが完全に責任を持ち、対象範囲内のコンポーネントは `overflow-*` を付与するだけで関与しない。対象範囲内のコンポーネント側でスクロールバー幅・色・角丸を上書きしてはならない。対象外領域（TabsList）の既存上書きは本件の対象外として維持する。
- **共通UI層（`src/components/ui/`）↔ パネル層**: スクロールに関する共通ラッパーコンポーネントは提供しない（`scroll-area.tsx` 削除後は復活させない）。
- **Tailwind ユーティリティ ↔ グローバルCSS**: スクロール挙動の制御（`overflow-auto`, `overflow-y-auto` 等）はTailwindユーティリティ、見た目はグローバルCSS、と役割分担する。

### 受け入れ検証

各シナリオの検証手段は次のとおり。

- **静的検証（必須）**:
  - `src/components/ui/scroll-area.tsx` および `src/components/ui/scroll-area.test.tsx` がリポジトリに存在しないこと。
  - `src/` 配下のアプリ実装・テスト（`docs/` および本Spec自身を除く）から、削除対象の `scroll-area.tsx` / `scroll-area.test.tsx` 削除後に `ScrollArea` / `ScrollBar` の import が 0 件であること。
  - `src/` 配下から `data-slot="scroll-area"` / `scroll-area-viewport` / `scroll-area-scrollbar` / `scroll-area-thumb` の出現が 0 件であること。
  - `src/index.css` の `@layer base` 内に §1 の具体値（`width: 10px; height: 10px;`、`border-radius: 9999px`、dark/light 各 oklch 値、`scrollbar-width: thin`、`scrollbar-color`）が含まれる。
  - `src/index.css` から `.scrollbar-thin` 専用ブロック（`.scrollbar-thin` / `.scrollbar-thin::-webkit-scrollbar` / `.scrollbar-thin::-webkit-scrollbar-track` / `.scrollbar-thin::-webkit-scrollbar-thumb` / `.scrollbar-thin::-webkit-scrollbar-thumb:hover`）の定義が 0 件であること。
  - `src/` 配下から `scrollbar-thin` クラスの使用（className 等での出現）が 0 件であること。
  - `src/remote/styles/terminal.css` 内に `.xterm .xterm-viewport::-webkit-scrollbar`、`::-webkit-scrollbar-thumb`、`::-webkit-scrollbar-track`、`::-webkit-scrollbar-thumb:hover` 等の見た目に関する個別スクロールバー指定（`width`、thumb 色指定、`border-radius`、`border`、`background-clip` 等）が 0 件であること。挙動指定（`overflow-y`、`-webkit-overflow-scrolling`、`overscroll-behavior` 等）は残置可。
- **視覚確認（必須）**:
  - 対象スクロール領域で §1 の具体値（width/height、border-radius、各 oklch 色、scrollbar-width、scrollbar-color）が適用されていること。
  - カバレッジ: dark/light 各テーマ × 縦/横 × 通常/hover を網羅する。
  - 横方向オーバーフローの視覚確認は次の 3 箇所をすべて含める: `src/components/panels/AgentChatPanel/StreamMessage.tsx`、`src/components/panels/ShikiDiffViewer.tsx`、`src/components/panels/ImageDiffViewer.tsx`。
  - `.scrollbar-thin` 削除に伴う視覚確認として、`src/components/panels/MarkdownDiffViewer.tsx` L102 の split コンテナおよび L235 の preview 領域で §1 の見た目が適用されていることを確認する。
  - 具体的な検証手段（selector、fixture、CSSOM 取得方法、Playwright プロジェクト構成等）は実装に委ねる。
- **非回帰**:
  - `scroll-area.test.tsx` 削除以外の既存テスト（`pnpm test` / `pnpm lint` / `cargo test`）が緑のまま。

### 実装に委ねること

- 各置換箇所での `ScrollArea` → `div` 変換時に必要な補助スタイル（`relative`・`size-full`・`rounded-[inherit]` 等）の移管要否: 個別に最小限で判断してよい。
- `viewportRef` / `onScroll` を渡していた呼び出し元の置換方法: 素の `div` に `ref` / `onScroll` を直接渡す形で揃える（呼び出し側の型変更が伴うため、合わせて修正）。
- AgentChatPanel のチャット履歴における「最下部付近」の判定閾値（既存挙動の追従条件）: 既存実装の判定ロジック・閾値をそのまま踏襲する。本Specでは数値を固定しない。
- §1 の具体値は唯一の基準とする。変更が必要な場合は本Specを先に更新してから実装する。
