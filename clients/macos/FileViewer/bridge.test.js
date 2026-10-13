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
}

async function bridge() {
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
    TextEncoder, TextDecoder, console,
  });
  return { window, messages, instances, elements, renderer: () => instances.at(-1) };
}

const payload = () => ({
  path: 'file.rs', version: '1', original: 'a\nb\nc\n', modified: 'a\nc\n',
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
  assert.equal(open.open, true);
  assert.equal(open.children[0].textContent, 'human');
  assert.equal(open.children[1].textContent, 'selected deleted line');
  const input = open.children.find((element) => element.attributes['aria-label'] === 'Threadへの返信');
  input.value = 'reply body';
  open.children.find((element) => element.textContent === '返信').onclick();
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'reply', id: 't1', content: 'reply body', requestID: 1 });
  const replied = { ...thread, comments: [...thread.comments, { author: 'human', content: 'reply body' }] };
  ui.window.releashRender({ ...payload(), threads: [replied] });
  const refreshed = render(ui.renderer().input.lineAnnotations[0]);
  assert.equal(refreshed.children[2].textContent, 'reply body');
  open.children.find((element) => element.textContent === '解決').onclick();
  assert.deepEqual(plain(ui.messages.at(-1)), { kind: 'resolve', id: 't1' });
  ui.window.releashRender({ ...payload(), threads: [{ ...thread, resolved: true }] });
  const resolved = render(ui.renderer().input.lineAnnotations[0]);
  assert.equal(resolved.open, false);
  assert.equal(resolved.children[0].textContent, 'human（解決済み）');
  assert.equal(resolved.children.length, 2);
  assert.equal(render({ metadata: { ...thread, canResolve: false } }).children.some((e) => e.textContent === '解決'), false);
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
  refreshed.children.find((child) => child.textContent === '返信').onclick();
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
  element.children.find((child) => child.textContent === '返信').onclick();
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
