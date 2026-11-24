import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import { build } from 'esbuild';

class Element {
  constructor(tagName) { this.tagName = tagName; }
  children = [];
  attributes = {};
  append(...children) { this.children.push(...children); }
  setAttribute(name, value) { this.attributes[name] = value; }
  focus() { this.focused = true; }
}

async function bridge(now = Date.now(), { initialTop = 0, lastLine = Infinity } = {}) {
  const instances = [];
  const messages = [];
  class Renderer {
    constructor(options) { this.options = options; instances.push(this); }
    setOptions(options) { this.options = options; }
    requests = [];
    expansions = [];
    expandHunk(...args) { this.expansions.push(args); }
    isLineRenderable() { return false; }
    getLinePosition(lineNumber) { return { top: lineNumber * 10, height: 10 }; }
    renderedEnd = 100;
    top = initialTop;
    setup() {}
    setItems([input]) { this.input = input; }
    subscribeToScroll(callback) { this.scroll = callback; }
    scrollTo(target) {
      this.requests.push(target);
      if (target.lineNumber > lastLine) return;
      this.top = Math.min(106, 10 + target.lineNumber * 10 - target.offset);
    }
    getScrollTop() { return this.top; }
    getLocalTopForInstance() { return 10; }
    getRenderedItems() { return [{ instance: this, item: this.input }]; }
    getNumericScrollAnchor(top) {
      if (top > this.renderedEnd) return;
      const lineNumber = Math.ceil(top / 10);
      return { lineNumber, top: lineNumber * 10, side: this.side };
    }
    setSelectedLines() {}
    cleanUp() { this.cleaned = true; }
  }
  const elements = { viewport: new Element() };
  const window = {
    testDiffs: {
      CodeView: Renderer,
    },
    webkit: { messageHandlers: { fileViewer: { postMessage: (value) => messages.push(value) } } },
  };
  const source = await readFile(new URL('./bridge.js', import.meta.url), 'utf8');
  const compiled = await build({
    stdin: {
      contents: source.replace("import { CodeView } from '@pierre/diffs';",
        'const { CodeView } = window.testDiffs;'),
      resolveDir: new URL('.', import.meta.url).pathname,
    },
    loader: { '.svg': 'dataurl' }, bundle: true, write: false, format: 'iife',
  });
  const document = Object.assign(new Element(), {
    getElementById: (id) => elements[id], createElement: (name) => new Element(name),
  });
  vm.runInNewContext(compiled.outputFiles[0].text, {
    window, document,
    Date: class extends Date { static now() { return now; } },
    TextEncoder, TextDecoder, console, queueMicrotask: (callback) => callback(),
  });
  return { window, messages, renderer: () => instances.at(-1) };
}

const payload = () => ({
  path: 'file.rs', original: 'a\nb\nc\n', modified: 'a\nc\n',
  patch: '--- a/file.rs\n+++ b/file.rs\n@@ -1,3 +1,2 @@\n a\n-b\n c\n',
  mode: 'diff', base: 'head', style: 'split', full: false, dark: false, deleted: false, accent: '#112233',
  threads: [],
});
const plain = (value) => JSON.parse(JSON.stringify(value));

test('display choices and system theme reach the renderer, including File and Diff transitions', async () => {
  // Given
  const ui = await bridge();
  const next = payload();
  // When
  ui.window.releashRender(next);
  for (const [style, full, dark] of [['unified', true, true], ['split', false, false]]) {
    // When
    ui.window.releashRender({ ...next, style, full, dark });
    // Then
    assert.equal(ui.renderer().options.diffStyle, style);
    assert.deepEqual(plain(ui.renderer().options.theme), { dark: 'dark-plus', light: 'light-plus' });
    assert.equal(ui.renderer().options.expandUnchanged, full);
    assert.equal(ui.renderer().options.themeType, dark ? 'dark' : 'light');
    assert.equal(ui.renderer().input.fileDiff.deletionLines.join(''), next.original);
  }
  // Then
  assert.equal(ui.renderer().input.type, 'diff');
  // Given
  const diff = ui.renderer();
  // When
  ui.window.releashRender({ ...next, mode: 'file', threads: [{ id: 't', line: 2, comments: [] }] });
  // Then
  assert.equal(diff.cleaned, true);
  assert.equal(ui.renderer().input.file.contents, next.modified);
  assert.equal(ui.renderer().input.annotations[0].lineNumber, 2);
  // When
  ui.window.releashRender(next);
  // Then
  assert.equal(ui.renderer().input.fileDiff.deletionLines.join(''), next.original);
});

test('selecting lines reports the normalized range and clears when the selection or the file changes', async () => {
  // Given
  const ui = await bridge();
  const next = payload();
  // When
  ui.window.releashRender(next);
  for (const side of ['deletions', 'additions']) {
    // When
    ui.renderer().options.onLineSelected({ start: 2, end: 2, side, endSide: side });
    // Then
    assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select', line: 2, endLine: 2, side, endSide: side });
    // When
    ui.window.releashRender({ ...next, threads: [{ id: side, line: 2, side, comments: [] }] });
    const annotation = ui.renderer().input.annotations[0];
    // Then
    assert.equal(annotation.side, side);
    assert.equal(annotation.lineNumber, 2);
  }
  // When
  ui.renderer().options.onLineSelected({ start: 4, end: 2, side: 'deletions', endSide: 'additions' });
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select', line: 2, endLine: 4, side: 'additions', endSide: 'deletions' });
  // When
  ui.renderer().options.onLineSelected(null);
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select' });
  // When
  ui.renderer().options.onLineSelected({ start: 1, end: 1 });
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select', line: 1, endLine: 1 });
  // When
  ui.window.releashRender({ ...next, modified: 'changed\n' });
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select' });
});

test('threads show comments, collapse resolved threads and send reply and resolve actions', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 't1', line: 2, side: 'deletions', resolved: false, canResolve: true,
    comments: [{ author: 'human', content: 'selected deleted line' }] };
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
  const render = ui.renderer().options.renderAnnotation;
  const open = render({ metadata: thread });
  // Then
  assert.equal(open.children[1].children[0].children[0].children[1].textContent, 'human');
  assert.equal(open.children[1].children[0].children[1].innerHTML, '<p>selected deleted line</p>\n');
  // Given
  const input = open.children.find((element) => element.attributes['aria-label'] === 'Threadへの返信');
  // When
  input.value = 'reply body';
  input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'reply', id: 't1', content: 'reply body', requestID: 1 });
  // Given
  const replied = { ...thread, comments: [...thread.comments, { author: 'human', content: 'reply body' }] };
  // When
  ui.window.releashRender({ ...payload(), expanded: [replied.id], threads: [replied] });
  const refreshed = render(ui.renderer().input.annotations[0]);
  // Then
  assert.equal(refreshed.children[1].children[1].children[1].innerHTML, '<p>reply body</p>\n');
  // When
  open.children.find((element) => element.textContent === '解決にする').onclick();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'resolve', id: 't1' });
  // When
  ui.window.releashRender({ ...payload(), threads: [{ ...thread, resolved: true }] });
  const resolved = render(ui.renderer().input.annotations[0]);
  // Then
  assert.equal(resolved.children.length, 1);
  const toggle = resolved.children[0].children.at(-1);
  assert.equal(toggle.attributes['aria-label'], 'Thread を表示');
  // When
  toggle.onclick();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'expand', id: thread.id, open: true });
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
  // Then
  assert.equal(render({ metadata: { ...thread, canResolve: false } }).children.some((e) => e.textContent === '解決にする'), false);
});

test('changing comparisons recreates the renderer and clears the selection', async () => {
  // Given
  const ui = await bridge();
  for (const base of ['head', 'branch-base', 'head']) {
    const previous = ui.renderer();
    // When
    if (previous) previous.options.onLineSelected({ start: 1, end: 1 });
    ui.window.releashRender({ ...payload(), base });
    // Then
    if (previous) {
      assert.equal(previous.cleaned, true);
      assert.notEqual(ui.renderer(), previous);
      assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select' });
    }
    assert.equal(ui.renderer().options.renderGutterUtility, undefined);
  }
});

test('reply drafts survive subscription updates and failed sends, and clear only after acceptance', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 't1', line: 2, comments: [], resolved: false };
  const render = () => ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]);
  const input = (element) => element.children.find((child) => child.attributes['aria-label'] === 'Threadへの返信');
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
  const original = input(render());
  original.value = 'unsent reply';
  original.oninput();
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [{ ...thread }] });
  const refreshed = render();
  // Then
  assert.equal(input(refreshed).value, 'unsent reply');
  // When
  input(refreshed).onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  const request = ui.messages.at(-1);
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [{ ...thread }] });
  // Then
  assert.equal(input(render()).value, 'unsent reply');
  // When
  ui.window.releashReplyAccepted(request.id, request.requestID);
  // Then
  assert.equal(input(render()).value, '');
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [{ ...thread }] });
  // Then
  assert.equal(input(render()).value, '');
});


test('accepting a reply preserves text edited while the request was pending', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 't1', line: 2, comments: [], resolved: false };
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
  const annotation = () => ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]);
  const element = annotation();
  const input = element.children.find((child) => child.tagName === 'textarea');
  input.value = 'sent'; input.oninput();
  input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  const request = ui.messages.at(-1);
  input.value = 'new draft'; input.oninput();
  ui.window.releashReplyAccepted(request.id, request.requestID);
  // Then
  assert.equal(input.value, 'new draft');
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [{ ...thread }] });
  // Then
  assert.equal(annotation().children.find((child) => child.tagName === 'textarea').value, 'new draft');
});

for (const recreated of [false, true]) {
  test(`送信完了の再描画が新しい返信入力を上書きしない（再生成 ${recreated}）`, async () => {
    // Given
    const ui = await bridge();
    const thread = { id: 't', line: 1, comments: [] };
    const next = { ...payload(), expanded: [thread.id], threads: [thread], replies: { t: 'sent' } };
    const sending = { ...next, sendingReplies: ['t'] };
    ui.window.releashRender(recreated ? sending : next);
    const input = ui.renderer().options.renderAnnotation({ metadata: thread }).children[2];
    let request;
    if (!recreated) {
      input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
      request = ui.messages.at(-1);
      ui.window.releashRender(sending);
    }
    input.value = 'new draft';
    input.oninput();
    // When
    if (request) ui.window.releashReplyAccepted(request.id, request.requestID);
    ui.window.releashRender({ ...next, replies: {}, sendingReplies: [] });
    // Then
    assert.equal(input.value, 'new draft');
  });
}

test('thread ranges and line selection use the accent color from the app', async () => {
  // Given
  const ui = await bridge();
  const next = payload();
  // When
  ui.window.releashRender({ ...next, threads: [{ id: 'a', line: 2, endLine: 3, side: 'additions', comments: [] }, { id: 'd', line: 5, side: 'deletions', resolved: true, comments: [] }] });
  const css = ui.renderer().options.unsafeCSS;
  // Then
  assert.equal(ui.renderer().input.annotations[0].lineNumber, 3);
  assert.equal(ui.renderer().input.annotations[1].lineNumber, 5);
  for (const line of [2, 3]) {
    assert.ok(css.includes(`[data-unified] [data-line="${line}"]:not([data-line-type=change-deletion])`), css);
    assert.ok(css.includes(`[data-additions] [data-line="${line}"]`), css);
  }
  assert.ok(!css.includes('[data-line="4"]'), css);
  assert.ok(css.includes('[data-unified] [data-line-type=change-deletion][data-line="5"]'), css);
  assert.ok(css.includes('[data-unified] [data-alt-line="5"]'), css);
  assert.ok(css.includes('[data-deletions] [data-line="5"]'), css);
  assert.equal(css.split('color-mix').length, 2, css);
  // When
  ui.window.releashRender({ ...next, mode: 'file', threads: [{ id: 'f', line: 7, endLine: 9, comments: [] }] });
  // Then
  assert.equal(ui.renderer().input.annotations[0].lineNumber, 9);
  assert.equal(ui.renderer().options.unsafeCSS, ':host{--diffs-bg-selection-override:#112233;--diffs-bg-selection-number-override:#112233}\n[data-line="7"],[data-line="8"],[data-line="9"]{box-shadow:inset 3px 0 #112233;background-color:color-mix(in srgb,var(--diffs-bg) 85%,#112233);}');
});


test('本文のMarkdownは整形しHTMLと危険なURLを実行しない', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'markdown', line: 1, comments: [{ author: 'a',
    content: '# Heading\n\n- item\n\n`code` **bold** ![image](https://example.com/a.png) [link](https://example.com)\n\n<script>alert(1)</script> [bad](javascript:alert(1))' }] };
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
  const card = ui.renderer().options.renderAnnotation({ metadata: thread });
  const html = card.children[1].children[0].children[1].innerHTML;
  // Then
  for (const part of ['<h1>Heading</h1>', '<li>item</li>', '<code>code</code>', '<strong>bold</strong>', '<img src="https://example.com/a.png"', '<a href="https://example.com"', '&lt;script&gt;']) assert.ok(html.includes(part), html);
  assert.ok(!html.includes('<script>'));
  assert.ok(!html.includes('href="javascript:'));
  assert.equal(ui.messages.length, 1);
});

test('返信はCommand Returnだけで送り空白とIME変換中は送らない', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'reply', line: 1, comments: [] };
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
  const input = ui.renderer().options.renderAnnotation({ metadata: thread }).children[2];
  input.value = 'reply';
  input.onkeydown({ key: 'Enter', metaKey: false });
  input.onkeydown({ key: 'Enter', metaKey: true, isComposing: true });
  input.value = '  ';
  input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  // Then
  assert.equal(ui.messages.length, 1);
});

test('作成カードは範囲末尾に出て入力を保ち作成とキャンセルを送る', async () => {
  // Given
  const ui = await bridge();
  const next = { ...payload(), composer: { line: 1, endLine: 2, side: 'deletions' } };
  // When
  ui.window.releashRender(next);
  const render = () => ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]);
  // Then
  assert.equal(ui.renderer().input.annotations[0].lineNumber, 2);
  assert.equal(ui.renderer().input.annotations[0].side, 'deletions');
  // When
  let card = render();
  // Then
  assert.equal(card.children[0].focused, true);
  assert.equal(card.children[2].disabled, true);
  // When
  card.children[0].value = 'draft'; card.children[0].oninput();
  ui.window.releashRender(next);
  card = render();
  // Then
  assert.equal(card.children[0].value, 'draft');
  // When
  card.children[2].onclick();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'create', content: 'draft' });
  // When
  card.children[1].onclick();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'cancelCreate' });
  // When
  ui.window.releashRender(payload());
  ui.window.releashRender(next);
  // Then
  assert.equal(render().children[0].value, '');
});


test('作成カードの入力と作成とキャンセルは送信中だけ無効になる', async () => {
  // Given
  const ui = await bridge();
  for (const sending of [false, true, false]) {
    // When
    ui.window.releashRender({ ...payload(), composer: { id: 'draft', line: 1, sending, content: 'body' } });
    const card = ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]);
    // Then
    assert.equal(card.children[0].disabled, sending);
    assert.equal(card.children[1].disabled, sending);
    assert.equal(card.children[2].disabled, sending);
  }
});

test('作成本文をSwiftに渡しWebView再生成後に復元する', async () => {
  // Given
  const ui = await bridge();
  const next = { ...payload(), composer: { id: 'draft', line: 1 } };
  ui.window.releashRender(next);
  const input = ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]).children[0];
  // When
  input.value = '未送信本文'; input.oninput();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'editCreate', id: 'draft', content: '未送信本文' });
  // When
  const recreated = await bridge();
  recreated.window.releashRender({ ...next, base: 'branch-base', composer: { ...next.composer, content: ui.messages.at(-1).content } });
  const card = recreated.renderer().options.renderAnnotation(recreated.renderer().input.annotations[0]);
  // Then
  assert.equal(card.children[0].value, '未送信本文');
});

test('作成カードの空白だけの本文は送信しない', async () => {
  // Given
  const ui = await bridge();
  ui.window.releashRender({ ...payload(), composer: { id: 'draft', line: 1 } });
  const card = ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]);
  // When
  card.children[0].value = ' \n\t'; card.children[0].oninput();
  card.children[2].onclick();
  // Then
  assert.equal(card.children[2].disabled, true);
  assert.equal(ui.messages.some((message) => message.kind === 'create'), false);
});

test('解決済みの展開は保存状態に従い、閉じる操作を通知する', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 't', line: 1, resolved: true, comments: [{ author: 'a', content: 'body' }] };
  ui.window.releashRender({ ...payload(), threads: [thread], expanded: ['t'] });
  const card = ui.renderer().options.renderAnnotation({ metadata: thread });
  // When
  card.children[0].children.at(-1).onclick();
  // Then
  assert.equal(card.children[1].children.length, 1);
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'expand', id: 't', open: false });
});

test('相対時刻は単位境界と未来時刻を表示する', async () => {
  // Given
  const now = 1800000000;
  const ui = await bridge(now * 1000);
  for (const [seconds, expected] of [[-60, 'たった今'], [59, 'たった今'], [60, '1 分前'], [3599, '59 分前'], [3600, '1 時間前'], [86399, '23 時間前'], [86400, '1 日前'], [31535999, '364 日前'], [31536000, '1 年前']]) {
    const thread = { id: 't', line: 1, comments: [{ author: 'a', content: '', createdAt: now - seconds }] };
    // When
    ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
    const card = ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]);
    // Then
    assert.equal(card.children[1].children[0].children[0].children.at(-1).textContent, expected);
  }
});

test('注釈DOMを再利用した再描画後も受理した返信欄を空にする', async () => {
  // Given
  const ui = await bridge();
  const next = { ...payload(), expanded: ['t'], threads: [{ id: 't', line: 1, comments: [] }] };
  ui.window.releashRender(next);
  const input = ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]).children[2];
  input.value = 'reply'; input.oninput();
  input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  const request = ui.messages.at(-1);
  // When
  ui.window.releashRender(next);
  ui.window.releashReplyAccepted(request.id, request.requestID);
  // Then
  assert.equal(input.value, '');
  assert.equal(ui.messages.at(-1), request);
});

test('返信は長押しと送信中の連打を防ぎ失敗後に再送できる', async () => {
  // Given
  const ui = await bridge();
  // When
  ui.window.releashRender({ ...payload(), expanded: ['t1'], threads: [{ id: 't1', line: 2, comments: [] }] });
  const render = () => ui.renderer().options.renderAnnotation(ui.renderer().input.annotations[0]);
  let input = render().children.find((child) => child.tagName === 'textarea');
  input.value = 'reply'; input.oninput();
  const key = { key: 'Enter', metaKey: true, preventDefault() {} };
  const count = () => ui.messages.filter((message) => message.kind === 'reply').length;
  input.onkeydown({ ...key, repeat: true });
  // Then
  assert.equal(count(), 0);
  // When
  input.onkeydown(key);
  const request = ui.messages.at(-1);
  input = render().children.find((child) => child.tagName === 'textarea');
  input.onkeydown(key);
  // Then
  assert.equal(count(), 1);
  // When
  ui.window.releashReplyAccepted(request.id, request.requestID, false);
  // Then
  assert.equal(input.value, 'reply');
  // When
  input.onkeydown(key);
  // Then
  assert.equal(count(), 2);
  // Given
  const retry = ui.messages.at(-1);
  // When
  ui.window.releashReplyAccepted(request.id, request.requestID, true);
  // Then
  assert.equal(input.value, 'reply');
  // When
  ui.window.releashReplyAccepted(retry.id, retry.requestID, true);
  // Then
  assert.equal(input.value, '');
});

for (const side of ['additions', 'deletions']) {
  for (const { top, lineNumber, offset, restored } of [
    { top: 32, lineNumber: 3, offset: 8, restored: { lineNumber: 3, delta: 8 } },
    { top: 208, lineNumber: 20, offset: 2, restored: { lineNumber: 10, delta: 4 } },
  ]) {
    test(`保存した ${side} の ${lineNumber} 行目を一度だけ復元し、表示位置を通知する`, async () => {
      // Given
      const ui = await bridge();
      const previous = await bridge();
      previous.window.releashRender(payload());
      const original = previous.renderer();
      original.side = side;
      original.renderedEnd = 220;
      original.top = top;
      original.scroll();
      const next = { ...payload(), scrollAnchor: previous.messages.at(-1).anchor };
      // When
      ui.window.releashRender(next);
      const view = ui.renderer();
      view.side = side;
      view.options.onPostRender();
      ui.window.releashRender({ ...next, dark: true });
      // Then
      assert.deepEqual(plain(view.requests), [{ type: 'line', id: next.path, lineNumber, side, offset, align: 'start', behavior: 'instant' }]);
      assert.deepEqual(plain(ui.messages.at(-1).anchor), { ...restored, side });
    });
  }
}

test('差分から消えた行を復元しようとすると現在の表示位置を保存する', async () => {
  // Given
  const ui = await bridge(Date.now(), { initialTop: 22, lastLine: 3 });
  // When
  ui.window.releashRender({ ...payload(), scrollAnchor: { lineNumber: 99, delta: 0 } });
  const view = ui.renderer();
  view.options.onPostRender();
  // Then
  assert.equal(view.requests[0].lineNumber, 99);
  assert.equal(view.getScrollTop(), 22);
  assert.deepEqual(plain(ui.messages.at(-1).anchor), { lineNumber: 2, delta: 8 });
});

test('保存位置を復元する初回描画が終わるまで scroll で保存位置を上書きしない', async () => {
  // Given
  const ui = await bridge();
  ui.window.releashRender({ ...payload(), scrollAnchor: { lineNumber: 20, delta: 2 } });
  const view = ui.renderer();
  // When
  view.scroll();
  // Then
  assert.equal(ui.messages.some(m => m.kind === 'scroll'), false);
  // When
  view.options.onPostRender();
  view.top = 33;
  view.scroll();
  // Then
  assert.deepEqual(ui.messages.filter(m => m.kind === 'scroll').map(m => plain(m.anchor)), [
    { lineNumber: 10, delta: 4 }, { lineNumber: 3, delta: 7 },
  ]);
});

test('上端の行が無い場合は scroll で anchor: null を送る', async () => {
  // Given
  const ui = await bridge();
  ui.window.releashRender(payload());
  ui.renderer().getNumericScrollAnchor = () => undefined;
  // When
  ui.renderer().scroll();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'scroll', anchor: null });
});

test('描画範囲を越えて移動した後、描画が終わってから新しい位置を送る', async () => {
  // Given
  const ui = await bridge();
  ui.window.releashRender(payload());
  const view = ui.renderer();
  view.top = 32;
  view.scroll();
  ui.messages.length = 0;
  view.top = 253;
  // When
  view.scroll();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'scroll', anchor: null });
  // When
  view.options.onPostRender();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'scroll', anchor: null });
  // When
  view.renderedEnd = 300;
  view.options.onPostRender();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1).anchor), { lineNumber: 25, delta: 7 });
});
test('再生成したviewerが返信を復元する', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'open', line: 2, comments: [] };
  // When
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread], replies: { open: 'draft' } });
  const reply = ui.renderer().options.renderAnnotation({ metadata: thread }).children[2];
  // Then
  assert.equal(reply.value, 'draft');
});

test('返信の編集を通知する', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'open', line: 2, comments: [] };
  ui.window.releashRender({ ...payload(), expanded: [thread.id], threads: [thread] });
  const reply = ui.renderer().options.renderAnnotation({ metadata: thread }).children[2];
  // When
  reply.value = 'updated';
  reply.oninput();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'editReply', id: 'open', content: 'updated' });
});

test('作り直したviewerは送信中の返信を再送しない', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'open', line: 2, comments: [] };
  const next = { ...payload(), expanded: [thread.id], threads: [thread], replies: { open: 'draft' }, sendingReplies: ['open'] };
  ui.window.releashRender(next);
  const reply = ui.renderer().options.renderAnnotation({ metadata: thread }).children[2];
  // When
  reply.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  // Then
  assert.equal(ui.messages.some(m => m.kind === 'reply'), false);
});

test('作り直したviewerは送信完了後の下書きを反映する', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'open', line: 2, comments: [] };
  const next = { ...payload(), expanded: [thread.id], threads: [thread], replies: { open: 'draft' }, sendingReplies: ['open'] };
  ui.window.releashRender(next);
  const reply = ui.renderer().options.renderAnnotation({ metadata: thread }).children[2];
  // When
  ui.window.releashRender({ ...next, replies: {}, sendingReplies: [] });
  // Then
  assert.equal(reply.value, '');
});

test('操作したThreadの開閉ボタンだけにフォーカスを戻す', async () => {
  // Given
  const ui = await bridge();
  const threads = ['first', 'second'].map(id => ({ id, line: 1, comments: [] }));
  const toggle = thread => ui.renderer().options.renderAnnotation({ metadata: thread }).children[0].children.at(-1);
  ui.window.releashRender({ ...payload(), threads });
  // When
  toggle(threads[1]).onclick();
  ui.window.releashRender({ ...payload(), threads, expanded: ['second'] });
  const buttons = threads.map(toggle);
  // Then
  assert.equal(buttons[0].focused, undefined);
  assert.equal(buttons[1].focused, true);
});

for (const mode of ['diff', 'file']) {
  test(mode + 'で最終行の注釈内に移動すると負の差分を送る', async () => {
    // Given
    const ui = await bridge();
    ui.window.releashRender({ ...payload(), mode, threads: [{ id: 'last', line: 2, comments: [] }] });
    const view = ui.renderer();
    view.renderedEnd = 20;
    view.side = mode === 'diff' ? 'additions' : undefined;
    view.getLinePosition = () => ({ top: 20, height: 150 });
    // When
    view.top = 95;
    view.scroll();
    // Then
    assert.deepEqual(plain(ui.messages.at(-1).anchor),
      plain({ lineNumber: 2, side: view.side, delta: -65 }));
  });
}

for (const [mode, full, expected] of [['diff', false, [[0, 'up', 4], [1, 'up', 5]]], ['file', false, []], ['diff', true, []]]) {
  test(mode + ' 全文=' + full + ' でThreadの範囲末尾に応じて未変更行を展開する', async () => {
    // Given
    const ui = await bridge();
    const original = 'x\n'.repeat(10);
    const next = { ...payload(), mode, full, original, modified: 'x\n'.repeat(4) + 'y\n' + 'x\n'.repeat(5),
      patch: '--- a/file.rs\n+++ b/file.rs\n@@ -5 +5 @@\n-x\n+y\n',
      threads: [{ line: 1, endLine: 3, side: 'deletions' }, { line: 8, resolved: true }] };
    // When
    ui.window.releashRender(next);
    ui.renderer().options.onPostRender();
    // Then
    assert.deepEqual(ui.renderer().expansions, expected);
  });
}
