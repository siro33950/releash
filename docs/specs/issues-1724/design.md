# Design

## The actual design

### Architecture

#### terminal link の責務owner

新しい責務は frontend の terminal 表示面に閉じる。Rust 側に owner を新設しない。

- 本件の振る舞いは「terminal buffer 上の文字列をリンクとして描画する」「遷移先を hover で提示する」表示制御と、「URL を既定ブラウザへ渡す」Tauri plugin command 呼び出しだけで構成される。`AGENTS.md`「Rust がロジックを所有する」が frontend へ許す範囲（表示とレイアウト制御、ユーザー入力の受付、Tauri command の呼び出し）に収まる。
- 既存の外部遷移経路 `src/components/workspace/WorkspaceList.tsx:1304` が `@tauri-apps/plugin-opener` の `openUrl()` を直接呼んでいる。本件も同じ経路へ接続し、判断・分類・検証を伴う domain 規則は増やさない。

#### 主要な変更対象

- `src/hooks/useTerminal.ts` — `Terminal` 生成（`:166`）へ `linkHandler` を与え、`FitAddon`（`:174-175`）と同じ同期タイミングで `WebLinksAddon` を `loadAddon` する。Releash 内で `Terminal` を生成する唯一の箇所であり、workspace terminal と AgentSession terminal の双方がここを通る。
- `src/lib/terminalLinkActivation.ts`（新設） — 両経路が共有する activation 実装。
- `src/lib/terminalLinkTooltip.ts`（新設） — 両経路が共有する遷移先提示（hover tooltip）の実装。activation とは扱う対象（Tauri plugin 呼び出し / DOM 表示）が異なるため別 module に置く。いずれも terminal 系の frontend helper が `src/lib/terminal*.ts` に隣接テスト付きで置かれている既存構成に合わせる。
- `package.json` / `pnpm-lock.yaml` — `@xterm/addon-web-links` を `^0.12.0` で追加する。`@xterm/xterm@6.0.0` に対応する最新の stable（`dist-tags.latest` = `0.12.0`）であり、既存の `@xterm/addon-fit@^0.11.0` / `@xterm/addon-webgl@^0.19.0` と同じ系列に揃う。
- `src/test/setup.ts` — `@xterm/addon-web-links` の共有 mock を追加する（`Cross-cutting concerns` の検証を参照）。

#### 二経路を一つの実装に集約する

plain text URL と OSC 8 リンクは xterm 内で別機構であり、片方の設定だけでは R-001 と R-002 を同時に満たせない。

- plain text: リンク要素そのものが存在しない。`WebLinksAddon` が `registerLinkProvider` する provider が生成する。
- OSC 8: リンクは既に存在するが、`Terminal.options.linkHandler` が未設定だと core の `OscLinkProvider` 既定経路（`confirm` → `window.open()`）へ落ちる。WKWebView では `window.open()` が `null` を返し遷移しない（`@xterm/xterm@6.0.0` の実装で確認）。

したがって `linkHandler` と `WebLinksAddon` の両方を設定する。R-001〜R-006 は「いずれの経路でも同じ結果になること」を要求するため、activation と hover の実装をそれぞれ 1 つに集約し、両登録点から同じ関数を渡す。経路ごとに実装を分けると、片方だけが要求を満たす状態を構造的に排除できない。

`WebLinksAddon` は handler 引数を省略すると既定 handler（`window.open()` を使う）を採用する（`@xterm/addon-web-links@0.12.0` `src/WebLinksAddon.ts` で確認）。handler を必ず渡す。

#### hover の位置決めを MouseEvent 基準にする

両登録点が hover へ渡す位置引数は、型と実際の値が揃っていない。

- `ILinkHandler.hover(event, text, range: IBufferRange)` — buffer 座標。
- `ILinkProviderOptions.hover(event, text, location: IViewportRange)` — 型は viewport 座標だが、`WebLinkProvider._addCallbacks` は `link.range`（`LinkComputer._mapStrIdx` が返す buffer 座標）をそのまま渡す。

```ts
link.hover = (event: MouseEvent, uri: string): void => {
  if (this._options.hover) {
    const { range } = link;
    this._options.hover(event, uri, range);
  }
};
```

位置引数を tooltip の配置に使うと、この不一致がどちらかの経路で表示位置のずれになる。両経路が同じ意味で渡すのは `MouseEvent` だけなので、tooltip の位置は `MouseEvent` から決め、位置引数は使わない。

#### tooltip を xterm の hover 契約に載せる

tooltip 要素は `Terminal.element` の子として生成し、`xterm-hover` class を付ける。`Terminal.element` は xterm が `open()` で生成する `.xterm` 要素そのものである。

xterm の `_handleMouseMove` は composed path を外側へ辿り、`.xterm` に到達する前に `xterm-hover` を持つ要素があれば hover 更新を打ち切る。

```js
const i = e.composedPath();
for (let e = 0; e < i.length; e++) {
  const t = i[e];
  if (t.classList.contains("xterm")) break;
  if (t.classList.contains("xterm-hover")) return;
}
```

生成先と class のどちらを外しても、tooltip 上へポインタが乗った時点でリンク領域から外れたと判定され、tooltip の明滅とクリックの取りこぼしが起きて B-004 と B-002 / B-003 を満たせない。

`Terminal.element` は `terminal.open(container)` が生成する React ツリー外の要素であり、`useTerminal` は component tree を持たない hook である。したがって tooltip は React component ではなく DOM 操作で構築する。

#### addon と tooltip の lifecycle

`WebLinksAddon` は `WebglAddon`（`:395-400`、性能スイッチで gate される動的 import）ではなく `FitAddon` と同じく `Terminal` 生成直後に同期 load する。最初の出力が届く前から link provider が有効である必要があり、性能スイッチによる無効化の対象でもない。

tooltip 要素は `terminal.dispose()`（`:781`）で `Terminal.element` ごと除去されるが、hover 状態と要素参照の解放は unmount 時に明示的に行い、`leave` から通る解放と同じ経路を使う。

#### activation の失敗の扱い

`openUrl()` の rejection は握り潰さず `console.error` で記録する。`useTerminal.ts` の他の Tauri command 失敗経路（detach / kill）と同じ扱いにする。UI への表面化は Requirements にないため追加しない。

### Interface

外部から観測できる契約（Tauri command、local API、workflow 定義、CLI）は変更しない。新設・変更する Rust 側の公開契約はない。

内部境界として追加する契約は 2 つである。

- activation: `activateTerminalLink(uri: string): void` 相当の単一関数。受け取った URL を `openUrl()` へ渡す。
- 遷移先提示: hover で URL と `MouseEvent` を受け取り tooltip を表示し、leave と解放で隠す一組の関数。tooltip の生成先となる `Terminal.element` を保持する。

両登録点の handler 型は次のとおりで、activation / hover 実装は URL と `MouseEvent` 以外の引数を使わない。

- `WebLinksAddon` の handler: `(event: MouseEvent, uri: string) => void`
- `ILinkProviderOptions`: `hover?(event, text, location)` / `leave?(event, text)`
- `ILinkHandler`: `activate(event, text, range)` / `hover?(event, text, range)` / `leave?(event, text, range)`

`ILinkHandler.allowNonHttpProtocols` は設定せず、既定の falsy を保つ（R-004）。

### Data Model

該当なし。

### Database

該当なし。

### UI/UX

- リンクであることの提示（B-001）は xterm 内蔵の link 装飾（pointer cursor と下線）に依り、独自の装飾を追加しない。
- 遷移先の提示（B-004）は hover tooltip で行う。表示内容は遷移先 URL 全体とし、R-007 の連結範囲内で折り返された URL も連結後の全体を出す。プレーンテキスト URL でも省略しない。
- tooltip は `xterm.css` が helper 類へ割り当てる z-index 帯（`5` / `6` / `10`）より上に置く。下に置くと WebGL canvas や helper に隠れて R-005 を満たせない。
- クリック時に確認の応答を求めない（R-006、B-005）。OSC 8 既定実装が持つ `confirm` は `linkHandler` を設定することで置き換わり、plain text 側も handler を渡すため確認手順は入らない。

### Algorithm

plain text URL の検出は `WebLinksAddon` 既定の provider と既定 `urlRegex` を使い、`urlRegex` を上書きしない。

- 既定 regex は `(https?|HTTPS?)://…` の形で http / https だけを検出する（R-001、R-004）。
- 折り返し行の連結は `@xterm/addon-web-links@0.12.0` の `LinkComputer._getWindowedLineStrings` が `isWrapped` を辿って行う（R-007）。同メソッドはクリックした行から上方向・下方向へ別々に展開し、どちらのループも連結済み文字列の `length < 2048` を条件に打ち切るため、R-007 の連結範囲は各方向 2,048 文字までになる。
- `LinkComputer` は同 package の `lib` entry point からも `typings/addon-web-links.d.ts` からも export されず、この上限を差し替える手段はない。自前 regex と `registerLinkProvider` で provider 全体を置き換えると、この連結と、文字列 index から buffer range への写像（`_mapStrIdx`、全角幅の補正を含む）を再実装することになるため、採用する provider の実装値を R-007 の境界とする。

OSC 8 側では検出を行わない。core の `OscLinkProvider` が `linkHandler` の有無だけで activation 先を切り替え、`allowNonHttpProtocols` が falsy のとき `new URL(uri).protocol` が http / https でないものを link 化しない（R-004）ため、追加の provider を登録しない。

### Infra

該当なし。

## Alternatives Considered

- **Rust に「URL を既定ブラウザで開く」usecase / command を新設し、frontend は `invoke` する案。** 既存の `src-tauri/src/adaptor/gateway/external_editor/launcher_impl.rs` は `tauri_plugin_opener::OpenerExt` を Rust から使うが、あれは「どの editor で開くか」という設定由来の規則を伴うため Rust が所有している。本件は URL をそのまま既定ブラウザへ渡すだけで規則を持たず、frontend の既存 `openUrl()` 経路と同一の操作になる。新設すると同じ操作の実装が 2 つになる。
- **`@xterm/addon-web-links` を追加せず、`registerLinkProvider` で自前 provider を実装する案。** 依存は増えないが、`Algorithm` に記した折り返し行の連結と buffer range 写像を自前で保守することになり、R-007 の維持コストを本件が負う。

## Cross-cutting concerns

- **セキュリティ**: terminal 出力は agent 由来で、Releash が内容を制御できない。R-004 は 3 つの独立した制限で成立する。plain text 側は既定 `urlRegex` が http / https に限る。OSC 8 側は `allowNonHttpProtocols` を有効にしないため、core が `javascript:` 等を link 化しない。さらに `opener:default`（`src-tauri/capabilities/normal-workbench.json:8`）が含む `allow-default-urls` の scope は `http://` / `https://` / `mailto:` / `tel:` であり、範囲外の URL は Rust 側で拒否される。この三重の制限は現状 capability の変更を伴わない。
- **検証**: B-002 / B-003 / B-005 / B-006 の「OS の既定ブラウザが開く」は jsdom で観測できない。単体検証の境界は「activation で `openUrl()` が当該 URL を引数に呼ばれること」と「`window.open()` が呼ばれないこと」に置く。B-007 は検出が第三者ライブラリの既定 regex と core の protocol 判定に委ねられるため、検証境界を「`urlRegex` と `allowNonHttpProtocols` を上書きしていないこと」に置く。`Terminal` の constructor options（`linkHandler`）と `loadAddon` 呼び出しは `src/hooks/useTerminal.test.ts` の局所 `@xterm/xterm` mock で観測する。`src/test/setup.ts` の共有 fixture へ追加するのは、handler と options を保持する `@xterm/addon-web-links` mock のみとする。B-001 / B-004 の tooltip は、provider 登録を経由せず hover / leave 実装を直接呼ぶ形で DOM を観測する。`@tauri-apps/plugin-opener` は setup.ts に mock がなく、既存の `WorkspaceList.test.tsx` 等と同じくテストファイル側で mock する。

## Risks

- R-007 の境界は、採用する provider の実装値で決まる。`@xterm/addon-web-links@0.12.0` の `LinkComputer._getWindowedLineStrings` は空白を含まない wrapped line をクリックした行から上方向・下方向へ辿り、どちらの展開ループも連結済み文字列の `length < 2048` を条件に打ち切る。`LinkComputer` は package の `lib` entry point と typings のいずれからも export されず差し替え手段がないため、各方向 2,048 文字を超える URL は R-007 の適用範囲外とする。
