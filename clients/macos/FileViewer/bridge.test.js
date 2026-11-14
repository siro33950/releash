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
  replaceChildren(...children) { this.children = children; }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener() {}
  focus() { this.focused = true; }
}

async function bridge(now = Date.now()) {
  const instances = [];
  const messages = [];
  class Renderer {
    constructor(options) { this.options = options; instances.push(this); }
    setOptions(options) { this.options = options; }
    render(input) { this.input = input; }
    setSelectedLines() {}
    cleanUp() { this.cleaned = true; }
  }
  const elements = Object.fromEntries(['viewport', 'content', 'file'].map((id) => [id, new Element()]));
  const window = {
    testDiffs: {
      VirtualizedFile: class extends Renderer {},
      VirtualizedFileDiff: class extends Renderer {},
      Virtualizer: class { setup() {} },
    },
    webkit: { messageHandlers: { fileViewer: { postMessage: (value) => messages.push(value) } } },
  };
  const source = await readFile(new URL('./bridge.js', import.meta.url), 'utf8');
  const compiled = await build({
    stdin: {
      contents: source.replace("import { VirtualizedFile, VirtualizedFileDiff, Virtualizer } from '@pierre/diffs';",
        'const { VirtualizedFile, VirtualizedFileDiff, Virtualizer } = window.testDiffs;'),
      resolveDir: new URL('.', import.meta.url).pathname,
    },
    bundle: true, write: false, format: 'iife',
  });
  vm.runInNewContext(compiled.outputFiles[0].text, {
    window, document: { getElementById: (id) => elements[id], createElement: (name) => new Element(name) },
    Date: class extends Date { static now() { return now; } },
    TextEncoder, TextDecoder, console, queueMicrotask: (callback) => callback(),
  });
  return { window, messages, instances, elements, renderer: () => instances.at(-1) };
}

const payload = () => ({
  path: 'file.rs', original: 'a\nb\nc\n', modified: 'a\nc\n',
  patch: '--- a/file.rs\n+++ b/file.rs\n@@ -1,3 +1,2 @@\n a\n-b\n c\n',
  mode: 'diff', base: 'head', style: 'split', full: false, dark: false, deleted: false, accent: '#112233',
  threads: [],
});
const plain = (value) => JSON.parse(JSON.stringify(value));

test('display choices and system theme reach the renderer, including File and Diff transitions', async () => {
  const ui = await bridge();
  const next = payload();
  ui.window.releashRender(next);
  for (const [style, full, dark] of [['unified', true, true], ['split', false, false]]) {
    ui.window.releashRender({ ...next, style, full, dark });
    assert.equal(ui.renderer().options.diffStyle, style);
    assert.deepEqual(plain(ui.renderer().options.theme), { dark: 'dark-plus', light: 'light-plus' });
    assert.equal(ui.renderer().options.expandUnchanged, full);
    assert.equal(ui.renderer().options.themeType, dark ? 'dark' : 'light');
    assert.equal(ui.renderer().input.fileDiff.deletionLines.join(''), next.original);
  }
  assert.equal(ui.renderer().input.fileContainer.tagName, 'diffs-container');
  const diff = ui.renderer();
  ui.window.releashRender({ ...next, mode: 'file', threads: [{ id: 't', line: 2, comments: [] }] });
  assert.equal(diff.cleaned, true);
  assert.equal(ui.renderer().input.file.contents, next.modified);
  assert.equal(ui.renderer().input.lineAnnotations[0].lineNumber, 2);
  ui.window.releashRender(next);
  assert.equal(ui.renderer().input.fileDiff.deletionLines.join(''), next.original);
});

test('selecting lines reports the normalized range and clears when the selection or the file changes', async () => {
  const ui = await bridge();
  const next = payload();
  ui.window.releashRender(next);
  for (const side of ['deletions', 'additions']) {
    ui.renderer().options.onLineSelected({ start: 2, end: 2, side, endSide: side });
    assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select', line: 2, endLine: 2, side, endSide: side });
    ui.window.releashRender({ ...next, threads: [{ id: side, line: 2, side, comments: [] }] });
    const annotation = ui.renderer().input.lineAnnotations[0];
    assert.equal(annotation.side, side);
    assert.equal(annotation.lineNumber, 2);
  }
  ui.renderer().options.onLineSelected({ start: 4, end: 2, side: 'deletions', endSide: 'additions' });
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select', line: 2, endLine: 4, side: 'additions', endSide: 'deletions' });
  ui.renderer().options.onLineSelected(null);
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select' });
  ui.renderer().options.onLineSelected({ start: 1, end: 1 });
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select', line: 1, endLine: 1 });
  ui.window.releashRender({ ...next, modified: 'changed\n' });
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select' });
});

test('threads show comments, collapse resolved threads and send reply and resolve actions', async () => {
  const ui = await bridge();
  const thread = { id: 't1', line: 2, side: 'deletions', resolved: false, canResolve: true,
    comments: [{ author: 'human', content: 'selected deleted line' }] };
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const render = ui.renderer().options.renderAnnotation;
  const open = render({ metadata: thread });
  assert.equal(open.children[0].children[0].children[0].children[1].textContent, 'human');
  assert.equal(open.children[0].children[0].children[1].innerHTML, '<p>selected deleted line</p>\n');
  const input = open.children.find((element) => element.attributes['aria-label'] === 'Threadへの返信');
  input.value = 'reply body';
  input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'reply', id: 't1', content: 'reply body', requestID: 1 });
  const replied = { ...thread, comments: [...thread.comments, { author: 'human', content: 'reply body' }] };
  ui.window.releashRender({ ...payload(), threads: [replied] });
  const refreshed = render(ui.renderer().input.lineAnnotations[0]);
  assert.equal(refreshed.children[0].children[1].children[1].innerHTML, '<p>reply body</p>\n');
  open.children.find((element) => element.textContent === '解決にする').onclick();
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'resolve', id: 't1' });
  ui.window.releashRender({ ...payload(), threads: [{ ...thread, resolved: true }] });
  const resolved = render(ui.renderer().input.lineAnnotations[0]);
  assert.equal(resolved.children[1].children.length, 0);
  const toggle = resolved.children[0].children[1];
  assert.equal(toggle.textContent, '表示');
  toggle.onclick();
  assert.equal(toggle.textContent, '隠す');
  assert.equal(resolved.children[1].children.length, 1);
  toggle.onclick();
  assert.equal(resolved.children[1].children.length, 0);
  assert.equal(resolved.children.length, 2);
  assert.equal(render({ metadata: { ...thread, canResolve: false } }).children.some((e) => e.textContent === '解決にする'), false);
});

test('changing comparisons recreates the renderer and clears the selection', async () => {
  const ui = await bridge();
  for (const base of ['head', 'branch-base', 'head']) {
    const previous = ui.renderer();
    if (previous) previous.options.onLineSelected({ start: 1, end: 1 });
    ui.window.releashRender({ ...payload(), base });
    if (previous) {
      assert.equal(previous.cleaned, true);
      assert.notEqual(ui.renderer(), previous);
      assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'select' });
    }
    assert.equal(ui.renderer().options.renderGutterUtility, undefined);
  }
});

test('reply drafts survive subscription updates and failed sends, and clear only after acceptance', async () => {
  const ui = await bridge();
  const thread = { id: 't1', line: 2, comments: [], resolved: false };
  const render = () => ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
  const input = (element) => element.children.find((child) => child.attributes['aria-label'] === 'Threadへの返信');
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const original = input(render());
  original.value = 'unsent reply';
  original.oninput();
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  const refreshed = render();
  assert.equal(input(refreshed).value, 'unsent reply');
  input(refreshed).onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  const request = ui.messages.at(-1);
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  assert.equal(input(render()).value, 'unsent reply');
  ui.window.releashReplyAccepted(request.id, request.requestID);
  assert.equal(input(render()).value, '');
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  assert.equal(input(render()).value, '');
});


test('accepting a reply preserves text edited while the request was pending', async () => {
  const ui = await bridge();
  const thread = { id: 't1', line: 2, comments: [], resolved: false };
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const annotation = () => ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
  const element = annotation();
  const input = element.children.find((child) => child.tagName === 'textarea');
  input.value = 'sent'; input.oninput();
  input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  const request = ui.messages.at(-1);
  input.value = 'new draft'; input.oninput();
  ui.window.releashReplyAccepted(request.id, request.requestID);
  assert.equal(input.value, 'new draft');
  ui.window.releashRender({ ...payload(), threads: [{ ...thread }] });
  assert.equal(annotation().children.find((child) => child.tagName === 'textarea').value, 'new draft');
});

test('thread ranges and line selection use the accent color from the app', async () => {
  const ui = await bridge();
  const next = payload();
  ui.window.releashRender({ ...next, threads: [{ id: 'a', line: 2, endLine: 3, side: 'additions', comments: [] }, { id: 'd', line: 5, side: 'deletions', resolved: true, comments: [] }] });
  const css = ui.renderer().options.unsafeCSS;
  assert.equal(ui.renderer().input.lineAnnotations[0].lineNumber, 3);
  assert.equal(ui.renderer().input.lineAnnotations[1].lineNumber, 5);
  for (const line of [2, 3]) {
    assert.ok(css.includes(`[data-unified] [data-line="${line}"]:not([data-line-type=change-deletion])`), css);
    assert.ok(css.includes(`[data-additions] [data-line="${line}"]`), css);
  }
  assert.ok(!css.includes('[data-line="4"]'), css);
  assert.ok(css.includes('[data-unified] [data-line-type=change-deletion][data-line="5"]'), css);
  assert.ok(css.includes('[data-unified] [data-alt-line="5"]'), css);
  assert.ok(css.includes('[data-deletions] [data-line="5"]'), css);
  assert.equal(css.split('color-mix').length, 2, css);
  ui.window.releashRender({ ...next, mode: 'file', threads: [{ id: 'f', line: 7, endLine: 9, comments: [] }] });
  assert.equal(ui.renderer().input.lineAnnotations[0].lineNumber, 9);
  assert.equal(ui.renderer().options.unsafeCSS, ':host{--diffs-bg-selection-override:#112233;--diffs-bg-selection-number-override:#112233}\n[data-line="7"],[data-line="8"],[data-line="9"]{box-shadow:inset 3px 0 #112233;background-color:color-mix(in srgb,var(--diffs-bg) 85%,#112233);}');
});


test('本文のMarkdownは整形しHTMLと危険なURLを実行しない', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'markdown', line: 1, comments: [{ author: 'a',
    content: '# Heading\n\n- item\n\n`code` **bold** ![image](https://example.com/a.png) [link](https://example.com)\n\n<script>alert(1)</script> [bad](javascript:alert(1))' }] };
  // When
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const card = ui.renderer().options.renderAnnotation({ metadata: thread });
  const html = card.children[0].children[0].children[1].innerHTML;
  // Then
  for (const part of ['<h1>Heading</h1>', '<li>item</li>', '<code>code</code>', '<strong>bold</strong>', '<img src="releash-comment-image://image?url=https%3A%2F%2Fexample.com%2Fa.png"', '<a href="https://example.com"', '&lt;script&gt;']) assert.ok(html.includes(part), html);
  assert.ok(!html.includes('<script>'));
  assert.ok(!html.includes('href="javascript:'));
  assert.equal(ui.messages.length, 1);
});

test('返信はCommand Returnだけで送り空白とIME変換中は送らない', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 'reply', line: 1, comments: [] };
  // When
  ui.window.releashRender({ ...payload(), threads: [thread] });
  const input = ui.renderer().options.renderAnnotation({ metadata: thread }).children[1];
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
  const render = () => ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
  // Then
  assert.equal(ui.renderer().input.lineAnnotations[0].lineNumber, 2);
  assert.equal(ui.renderer().input.lineAnnotations[0].side, 'deletions');
  let card = render();
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
    const card = ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
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
  const input = ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]).children[0];
  // When
  input.value = '未送信本文'; input.oninput();
  // Then
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'editCreate', id: 'draft', content: '未送信本文' });
  // When
  const recreated = await bridge();
  recreated.window.releashRender({ ...next, base: 'branch-base', composer: { ...next.composer, content: ui.messages.at(-1).content } });
  const card = recreated.renderer().options.renderAnnotation(recreated.renderer().input.lineAnnotations[0]);
  // Then
  assert.equal(card.children[0].value, '未送信本文');
});

test('作成カードの空白だけの本文は送信しない', async () => {
  // Given
  const ui = await bridge();
  ui.window.releashRender({ ...payload(), composer: { id: 'draft', line: 1 } });
  const card = ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
  // When
  card.children[0].value = ' \n\t'; card.children[0].oninput();
  card.children[2].onclick();
  // Then
  assert.equal(card.children[2].disabled, true);
  assert.equal(ui.messages.some((message) => message.kind === 'create'), false);
});

test('解決済みの展開は再描画で保持し未解決に戻ると解除する', async () => {
  // Given
  const ui = await bridge();
  const thread = { id: 't', line: 1, resolved: true, comments: [{ author: 'a', content: 'body' }] };
  const render = (resolved) => {
    ui.window.releashRender({ ...payload(), threads: [{ ...thread, resolved }] });
    return ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
  };
  render(true).children[0].children[1].onclick();
  // When
  const expanded = render(true);
  // Then
  assert.equal(expanded.children[0].children[1].textContent, '隠す');
  assert.equal(expanded.children[1].children.length, 1);
  // When
  render(false);
  const collapsed = render(true);
  // Then
  assert.equal(collapsed.children[0].children[1].textContent, '表示');
  assert.equal(collapsed.children[1].children.length, 0);
});

test('相対時刻は単位境界と未来時刻を表示する', async () => {
  // Given
  const now = 1800000000;
  const ui = await bridge(now * 1000);
  for (const [seconds, expected] of [[-60, 'たった今'], [59, 'たった今'], [60, '1 分前'], [3599, '59 分前'], [3600, '1 時間前'], [86399, '23 時間前'], [86400, '1 日前'], [31535999, '364 日前'], [31536000, '1 年前']]) {
    const thread = { id: 't', line: 1, comments: [{ author: 'a', content: '', createdAt: now - seconds }] };
    // When
    ui.window.releashRender({ ...payload(), threads: [thread] });
    const card = ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]);
    // Then
    assert.equal(card.children[0].children[0].children[0].children.at(-1).textContent, expected);
  }
});

test('注釈DOMを再利用した再描画後も受理した返信欄を空にする', async () => {
  // Given
  const ui = await bridge();
  const next = { ...payload(), threads: [{ id: 't', line: 1, comments: [] }] };
  ui.window.releashRender(next);
  const input = ui.renderer().options.renderAnnotation(ui.renderer().input.lineAnnotations[0]).children[1];
  input.value = 'reply'; input.oninput();
  input.onkeydown({ key: 'Enter', metaKey: true, preventDefault() {} });
  const request = ui.messages.at(-1);
  // When
  ui.window.releashRender(next);
  ui.window.releashReplyAccepted(request.id, request.requestID);
  // Then
  assert.equal(input.value, '');
});
